//! 门禁的 turn 测量：运行时刷新、发布快照/delta、按需重建场景。
//!
//! 测量边界与真实宿主一致：一次动作、一次 flush、向每个窗口发布最新根，
//! 只在 revision 变化时重建场景。计数是确定性的；时间包含这条完整路径。

use std::collections::BTreeMap;
use std::io;
use std::time::{Duration, Instant};

use zenui::prelude::*;
use zenui_render::{InteractionPaths, Scene, SceneCache, SceneEnv, Viewport};

use crate::alloc;

/// 通用错误容器；性能工具不区分 zenui 的 typed error。
pub type PerfError = Box<dyn std::error::Error>;
pub type PerfResult<T> = std::result::Result<T, PerfError>;

/// 把任意错误转成门禁错误，保留原始 Display 文本。
pub fn fail(message: impl Into<String>) -> PerfError {
    Box::new(io::Error::other(message.into()))
}

/// 一次测量里的计数器；键名与基线文件、文档一致。
pub type Counters = BTreeMap<String, u64>;

/// 把一个 counter 写入映射。
pub fn put(counters: &mut Counters, name: &str, value: u64) {
    counters.insert(name.to_owned(), value);
}

/// 合并一次样本：同名 counter 取最大值，保证硬上限按“最坏的一 turn”检查。
pub fn merge_max(into: &mut Counters, sample: &Counters) {
    for (name, value) in sample {
        let slot = into.entry(name.clone()).or_insert(0);
        *slot = (*slot).max(*value);
    }
}

/// 一次 turn 的时间与计数，以及窗口的发布状态。
pub struct TurnSample {
    pub elapsed: Duration,
    pub counters: Counters,
}

/// 固定算术负载的最快耗时（纳秒），用于归一化时间门禁。
///
/// CPU 频率、功耗状态与迁移会让同一份代码的绝对时间在两个进程间成倍
/// 变化；同一进程内测得的时钟比例能抵消这层漂移。它只校准时钟，不进入
/// 产品时间统计。
pub fn clock_probe() -> u64 {
    let mut best = u64::MAX;
    for _ in 0..7 {
        let started = Instant::now();
        let mut value = 0x9e37_79b9_7f4a_7c15_u64;
        for index in 0..2_000_000_u64 {
            value = value
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(index ^ 0x5a5a);
        }
        std::hint::black_box(value);
        best = best.min(duration_ns(started.elapsed()));
    }
    best
}

/// 把 `repeats` 个连续 turn 合成为一个样本：时间是总和，计数取每 turn 最大值。
///
/// 短操作（空 click、主题切换）的单次耗时接近调度噪声；合成长度固定的
/// 样本块后，最快样本的比例更稳定，真实回归仍会推高整块时间。
pub fn repeat<F>(repeats: usize, mut sample: F) -> PerfResult<TurnSample>
where
    F: FnMut() -> PerfResult<TurnSample>,
{
    let mut counters = Counters::new();
    let mut elapsed = Duration::ZERO;
    for _ in 0..repeats {
        let next = sample()?;
        merge_max(&mut counters, &next.counters);
        elapsed += next.elapsed;
    }
    Ok(TurnSample { elapsed, counters })
}

#[derive(Clone, Copy)]
struct Metrics {
    bindings: u64,
    memos: u64,
    builds: u64,
    live_nodes: u64,
}

impl Metrics {
    fn read(app: &Runtime) -> Self {
        Self {
            bindings: app.binding_evaluations(),
            memos: app.memo_recomputes(),
            builds: app.presentation_builds(),
            live_nodes: app.memory_report().live_nodes as u64,
        }
    }
}

fn delta_stats(counters: &mut Counters, update: &PresentationUpdate, previous_revision: u64) {
    let revision = update.snapshot().revision();
    let (changed, inserted, removed, layout_roots, full, appearance, row_bands) =
        match update.delta() {
            Some(delta) => (
                delta.changed().len() as u64,
                delta.inserted().len() as u64,
                delta.removed().len() as u64,
                delta.layout_roots().len() as u64,
                0,
                u64::from(delta.appearance_changed()),
                u64::from(delta.row_bands_changed()),
            ),
            None if revision == previous_revision => (0, 0, 0, 0, 0, 0, 0),
            // revision 前进却没有 delta：完整根，按全部节点计算重建量。
            None => (update.snapshot().node_count() as u64, 0, 0, 0, 1, 0, 0),
        };
    put(counters, "snapshot_nodes_changed", changed);
    put(counters, "snapshot_nodes_inserted", inserted);
    put(counters, "snapshot_nodes_removed", removed);
    put(
        counters,
        "snapshot_nodes_rebuilt",
        changed.saturating_add(inserted),
    );
    put(counters, "delta_full_roots", full);
    put(counters, "layout_roots", layout_roots);
    put(counters, "appearance_changed_turns", appearance);
    put(counters, "row_bands_changed_turns", row_bands);
    put(
        counters,
        "revision_advanced",
        u64::from(revision != previous_revision),
    );
    put(
        counters,
        "snapshot_total_nodes",
        update.snapshot().node_count() as u64,
    );
}

/// 一个已打开窗口的 headless 宿主；场景缓存跨样本复用。
pub struct WindowHarness {
    pub app: Runtime,
    pub window: WindowCtx,
    viewport: Viewport,
    env: SceneEnv,
    paths: InteractionPaths,
    cache: SceneCache,
    scene_enabled: bool,
    last_revision: u64,
    last_scene_revision: u64,
}

impl WindowHarness {
    /// 打开窗口并完成首次发布与（可选）首帧场景构建。
    pub fn new(mut app: Runtime, window: WindowCtx, scene_enabled: bool) -> PerfResult<Self> {
        let viewport = Viewport::new(900.0, 700.0, 1.0).map_err(|error| fail(error.to_string()))?;
        let _ = app.refresh().map_err(|error| fail(error.to_string()))?;
        let update = app
            .presentation_update_of(window)
            .map_err(|error| fail(error.to_string()))?;
        let last_revision = update.snapshot().revision();
        let mut harness = Self {
            app,
            window,
            viewport,
            env: SceneEnv::default(),
            paths: InteractionPaths::default(),
            cache: SceneCache::new(),
            scene_enabled,
            last_revision,
            last_scene_revision: 0,
        };
        if scene_enabled {
            harness.build_scene(&update)?;
        }
        Ok(harness)
    }

    /// 当前窗口 snapshot 中对 `label` 的节点身份。
    pub fn labeled_node(&self, label: &str) -> PerfResult<NodeId> {
        let snapshot = self
            .app
            .presentation_of(self.window)
            .map_err(|error| fail(error.to_string()))?;
        snapshot
            .find_label(label)
            .map(SnapshotElement::id)
            .ok_or_else(|| fail(format!("no published node labelled {label:?}")))
    }

    /// 设置环境滚动偏移；不触发运行时 turn。
    pub fn set_scroll_offset(&mut self, offset: i32) {
        self.env = self.env.with_scroll_offset(offset);
    }

    /// 在计时区外结算一次状态恢复：刷新、发布并更新场景基线。
    ///
    /// 用于在样本之间撤销计时 turn 的状态（例如退格清空输入），使每次
    /// 测量从同一条件开始；恢复本身不计入时间与计数。
    pub fn resync(&mut self) -> PerfResult<()> {
        let _ = self
            .app
            .refresh()
            .map_err(|error| fail(error.to_string()))?;
        let update = self
            .app
            .presentation_update_of(self.window)
            .map_err(|error| fail(error.to_string()))?;
        self.last_revision = update.snapshot().revision();
        if self.scene_enabled && self.last_revision != self.last_scene_revision {
            self.build_scene(&update)?;
        }
        Ok(())
    }

    fn build_scene(&mut self, update: &PresentationUpdate) -> PerfResult<usize> {
        let scene = Scene::build_cached(
            update.snapshot(),
            self.viewport,
            &self.env,
            &self.paths,
            &mut self.cache,
        )
        .map_err(|error| fail(error.to_string()))?;
        self.last_scene_revision = update.snapshot().revision();
        Ok(scene.elements().len())
    }

    /// 测一个完整 turn；`action` 内只做用户动作。
    pub fn measure<F>(&mut self, action: F) -> PerfResult<TurnSample>
    where
        F: FnOnce(&mut Runtime) -> PerfResult<()>,
    {
        let before = Metrics::read(&self.app);
        let allocations_before = alloc::allocations();
        let previous_revision = self.last_revision;
        let started = Instant::now();

        action(&mut self.app)?;
        // 动作已经完成自己的 turn（事件派发内部 flush）；这里不再额外
        // refresh，否则会把第二次全窗口绑定遍历算进同一个 turn。
        let update = self
            .app
            .presentation_update_of(self.window)
            .map_err(|error| fail(error.to_string()))?;
        let mut scene_elements = 0;
        if self.scene_enabled && update.snapshot().revision() != self.last_scene_revision {
            scene_elements = self.build_scene(&update)?;
        }

        let elapsed = started.elapsed();
        let after = Metrics::read(&self.app);
        let mut counters = Counters::new();
        put(
            &mut counters,
            "bindings_evaluated",
            after.bindings.saturating_sub(before.bindings),
        );
        put(
            &mut counters,
            "memo_recomputed",
            after.memos.saturating_sub(before.memos),
        );
        put(
            &mut counters,
            "snapshot_roots_built",
            after.builds.saturating_sub(before.builds),
        );
        put(
            &mut counters,
            "allocations",
            alloc::allocations().saturating_sub(allocations_before),
        );
        put(
            &mut counters,
            "changed_turns",
            u64::from(update.snapshot().revision() != previous_revision),
        );
        put(&mut counters, "mounted_nodes", after.live_nodes);
        put(
            &mut counters,
            "mounted_nodes_growth",
            after.live_nodes.saturating_sub(before.live_nodes),
        );
        put(&mut counters, "scene_builds", u64::from(scene_elements > 0));
        put(&mut counters, "scene_elements", scene_elements as u64);
        delta_stats(&mut counters, &update, previous_revision);

        self.last_revision = update.snapshot().revision();
        Ok(TurnSample { elapsed, counters })
    }

    /// 只重建场景（环境滚动、焦点瞬态等）；运行时 turn 必须为 0。
    pub fn measure_scene(&mut self) -> PerfResult<TurnSample> {
        let started = Instant::now();
        let allocations_before = alloc::allocations();
        let snapshot = self
            .app
            .presentation_of(self.window)
            .map_err(|error| fail(error.to_string()))?;
        let scene = Scene::build_cached(
            &snapshot,
            self.viewport,
            &self.env,
            &self.paths,
            &mut self.cache,
        )
        .map_err(|error| fail(error.to_string()))?;
        let elapsed = started.elapsed();
        let mut counters = Counters::new();
        put(&mut counters, "scene_builds", 1);
        put(
            &mut counters,
            "scene_elements",
            scene.elements().len() as u64,
        );
        put(
            &mut counters,
            "allocations",
            alloc::allocations().saturating_sub(allocations_before),
        );
        put(&mut counters, "changed_turns", 0);
        put(&mut counters, "snapshot_roots_built", 0);
        put(&mut counters, "snapshot_nodes_rebuilt", 0);
        put(&mut counters, "bindings_evaluated", 0);
        put(&mut counters, "memo_recomputed", 0);
        self.last_revision = snapshot.revision();
        Ok(TurnSample { elapsed, counters })
    }
}

struct MultiWindowState {
    window: WindowCtx,
    cache: SceneCache,
    revision: u64,
    scene_revision: u64,
}

/// 多个窗口共享一个 Runtime；用于主题切换与多窗口隔离。
pub struct MultiWindowHarness {
    pub app: Runtime,
    windows: Vec<MultiWindowState>,
    viewport: Viewport,
    env: SceneEnv,
    paths: InteractionPaths,
    scene_enabled: bool,
}

impl MultiWindowHarness {
    /// 打开全部窗口并完成首次发布与（可选）首帧场景构建。
    pub fn new(mut app: Runtime, windows: Vec<WindowCtx>, scene_enabled: bool) -> PerfResult<Self> {
        let viewport = Viewport::new(900.0, 700.0, 1.0).map_err(|error| fail(error.to_string()))?;
        let _ = app.refresh().map_err(|error| fail(error.to_string()))?;
        let mut states = Vec::with_capacity(windows.len());
        for window in windows {
            let update = app
                .presentation_update_of(window)
                .map_err(|error| fail(error.to_string()))?;
            let revision = update.snapshot().revision();
            let mut cache = SceneCache::new();
            if scene_enabled {
                Scene::build_cached(
                    update.snapshot(),
                    viewport,
                    &SceneEnv::default(),
                    &InteractionPaths::default(),
                    &mut cache,
                )
                .map_err(|error| fail(error.to_string()))?;
            }
            states.push(MultiWindowState {
                window,
                cache,
                revision,
                scene_revision: revision,
            });
        }
        Ok(Self {
            app,
            windows: states,
            viewport,
            env: SceneEnv::default(),
            paths: InteractionPaths::default(),
            scene_enabled,
        })
    }

    /// 测一个影响多个窗口的 turn；每个窗口都发布一次。
    pub fn measure<F>(&mut self, action: F) -> PerfResult<TurnSample>
    where
        F: FnOnce(&mut Runtime) -> PerfResult<()>,
    {
        let before = Metrics::read(&self.app);
        let allocations_before = alloc::allocations();
        let started = Instant::now();

        action(&mut self.app)?;
        // 动作内部已完成 turn；额外 refresh 会重复遍历绑定。
        let mut counters = Counters::new();
        let mut revised = 0_u64;
        let mut unchanged = 0_u64;
        let mut scene_builds = 0_u64;
        let mut scene_elements = 0_u64;
        let mut nodes_changed = 0_u64;
        let mut nodes_inserted = 0_u64;
        let mut nodes_removed = 0_u64;
        let mut layout_roots = 0_u64;
        let mut full_roots = 0_u64;
        let mut appearance = 0_u64;

        for state in &mut self.windows {
            let update = self
                .app
                .presentation_update_of(state.window)
                .map_err(|error| fail(error.to_string()))?;
            let revision = update.snapshot().revision();
            if revision == state.revision {
                unchanged += 1;
                continue;
            }
            revised += 1;
            let previous = state.revision;
            state.revision = revision;
            let mut one = Counters::new();
            delta_stats(&mut one, &update, previous);
            nodes_changed = nodes_changed.saturating_add(one["snapshot_nodes_changed"]);
            nodes_inserted = nodes_inserted.saturating_add(one["snapshot_nodes_inserted"]);
            nodes_removed = nodes_removed.saturating_add(one["snapshot_nodes_removed"]);
            layout_roots = layout_roots.saturating_add(one["layout_roots"]);
            full_roots = full_roots.saturating_add(one["delta_full_roots"]);
            appearance = appearance.saturating_add(one["appearance_changed_turns"]);
            if self.scene_enabled && revision != state.scene_revision {
                let scene = Scene::build_cached(
                    update.snapshot(),
                    self.viewport,
                    &self.env,
                    &self.paths,
                    &mut state.cache,
                )
                .map_err(|error| fail(error.to_string()))?;
                state.scene_revision = revision;
                scene_builds += 1;
                scene_elements = scene_elements.saturating_add(scene.elements().len() as u64);
            }
        }

        let elapsed = started.elapsed();
        let after = Metrics::read(&self.app);
        put(
            &mut counters,
            "bindings_evaluated",
            after.bindings.saturating_sub(before.bindings),
        );
        put(
            &mut counters,
            "memo_recomputed",
            after.memos.saturating_sub(before.memos),
        );
        put(
            &mut counters,
            "snapshot_roots_built",
            after.builds.saturating_sub(before.builds),
        );
        put(
            &mut counters,
            "allocations",
            alloc::allocations().saturating_sub(allocations_before),
        );
        put(&mut counters, "changed_turns", u64::from(revised > 0));
        put(&mut counters, "windows_revised", revised);
        put(&mut counters, "windows_unchanged", unchanged);
        put(
            &mut counters,
            "unchanged_window_snapshot_builds",
            after
                .builds
                .saturating_sub(before.builds)
                .saturating_sub(revised),
        );
        put(&mut counters, "scene_builds", scene_builds);
        put(&mut counters, "scene_elements", scene_elements);
        put(&mut counters, "snapshot_nodes_changed", nodes_changed);
        put(&mut counters, "snapshot_nodes_inserted", nodes_inserted);
        put(&mut counters, "snapshot_nodes_removed", nodes_removed);
        put(
            &mut counters,
            "snapshot_nodes_rebuilt",
            nodes_changed.saturating_add(nodes_inserted),
        );
        put(&mut counters, "layout_roots", layout_roots);
        put(&mut counters, "delta_full_roots", full_roots);
        put(&mut counters, "appearance_changed_turns", appearance);
        put(&mut counters, "revision_advanced", u64::from(revised > 0));
        put(&mut counters, "mounted_nodes", after.live_nodes);
        put(
            &mut counters,
            "mounted_nodes_growth",
            after.live_nodes.saturating_sub(before.live_nodes),
        );
        Ok(TurnSample { elapsed, counters })
    }
}

/// 运行预热与正式样本，保留每次的时间与计数最大值。
pub struct SampleSet {
    pub times_ns: Vec<u64>,
    pub max: Counters,
    pub clock_ns: u64,
}

impl SampleSet {
    /// 以固定预热数与正式样本数运行 `sample`。
    pub fn run<F>(mut sample: F) -> PerfResult<Self>
    where
        F: FnMut() -> PerfResult<TurnSample>,
    {
        for _ in 0..crate::WARMUP {
            let _ = sample()?;
        }
        let mut times_ns = Vec::with_capacity(crate::SAMPLES);
        let mut max = Counters::new();
        for _ in 0..crate::SAMPLES {
            let sample = sample()?;
            merge_max(&mut max, &sample.counters);
            times_ns.push(duration_ns(sample.elapsed));
        }
        Ok(Self {
            times_ns,
            max,
            clock_ns: clock_probe(),
        })
    }
}

/// 纳秒值；`Duration::as_nanos` 在 64 位平台足够，超出时饱和。
pub fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}
