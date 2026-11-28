//! 独立差分 oracle（仅测试构建）。
//!
//! 每个 turn 之后，oracle 绕过生产的 binding、属性与 memo 缓存：
//!
//! - 逐个求值**每棵树**（包括隐藏窗口）的已挂载绑定槽来源，与槽里已提交的值比较；
//! - 忽略 dirty 标记，按依赖关系重新计算全部 mounted memo，并与生产值比较；
//! - 按参考状态重新挂载一份参考窗口，把每个窗口的规范化快照（tag、属性、样式、
//!   语义、文本、行顺序）与它逐节点比较；
//! - 随机事件序列由一个独立的状态机生成期望值，逐窗口比较可见状态、焦点、
//!   caret/selection、行身份、行带元数据与 revision。
//!
//! 参考状态机只描述应用语义与 API 契约（计数器、草稿、显隐、勾选、行集合、
//! 行窗口、每个窗口的焦点与编辑状态、运行时输入焦点窗口），不复制快照构建、
//! 绑定求值或 memo 结算的实现；它们正是被比较的对象。失败信息包含种子、
//! 完整操作序列与路径级 diff，可直接复现。

#![allow(unused_braces)]

use std::any::Any;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

use zenui_core::PanicInfo;
use zenui_core::catch_panic;

use crate::node::{BindingValue, TreeKey};
use crate::prelude::*;
use crate::presentation::{PresentationSnapshot, SnapshotElement, SnapshotNode, SnapshotValue};
use crate::runtime::{Runtime, TreeSlot};
use crate::signal::{MemoCompute, ReactiveValue};
use crate::view::{ReadFault, ReadValue};

// ---------------------------------------------------------------------------
// memo 重算：oracle 自己的拓扑缓存，生产 `MemoCell` 保持只读。
// ---------------------------------------------------------------------------

struct OracleMemos {
    runtime: u32,
    values: BTreeMap<u32, Rc<dyn Any>>,
    computing: BTreeSet<u32>,
    poisoned: BTreeSet<u32>,
}

thread_local! {
    static ORACLE: RefCell<Option<OracleMemos>> = const { RefCell::new(None) };
}

enum Step {
    Ready,
    Cycle,
    Missing,
    Disposed,
    Poisoned,
    Compute(Rc<dyn MemoCompute>),
}

/// `Memo::read_result` 的 oracle 入口；oracle 未安装时返回 `None`。
pub(crate) fn memo_read<T: ReactiveValue>(runtime: u32, id: u32) -> Option<Result<T, ReadFault>> {
    let active = ORACLE.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|oracle| oracle.runtime == runtime)
    });
    active.then(|| memo_read_installed::<T>(id))
}

fn memo_read_installed<T: ReactiveValue>(id: u32) -> Result<T, ReadFault> {
    loop {
        match memo_step(id) {
            Step::Ready => {
                let value = oracle_value(id).expect("a ready oracle memo has a value");
                return Ok((*value.downcast::<T>().expect("oracle memo value type")).clone());
            }
            Step::Cycle => return Err(ReadFault::GraphCycle),
            Step::Missing => return Err(ReadFault::ComputedPanicked),
            Step::Disposed => return Err(ReadFault::Disposed),
            // 生产合同：毒化的 memo 保留最后一次已提交值，界面不回滚。独立
            // 重算只能知道“会 panic”，无法从源推出被保留的旧值，因此按合同
            // 读取生产保留值；没有旧值时仍是 typed fault。
            Step::Poisoned => {
                return production_memo_value::<T>(id).ok_or(ReadFault::ComputedPanicked);
            }
            Step::Compute(compute) => {
                let outcome = catch_panic(|| compute.call());
                memo_complete(id, outcome)?;
            }
        }
    }
}

fn memo_step(id: u32) -> Step {
    let (compute, disposed) = crate::context::with_state(|state| {
        let cell = &state.memos[id as usize];
        (cell.compute.clone(), cell.disposed)
    });
    let mut oracle = ORACLE
        .with(|cell| cell.borrow_mut().take())
        .expect("oracle evaluation is active");
    let step = if disposed {
        oracle.computing.remove(&id);
        Step::Disposed
    } else if oracle.poisoned.contains(&id) {
        oracle.computing.remove(&id);
        Step::Poisoned
    } else if oracle.values.contains_key(&id) {
        Step::Ready
    } else if !oracle.computing.insert(id) {
        Step::Cycle
    } else {
        match compute {
            Some(compute) => Step::Compute(compute),
            None => {
                oracle.computing.remove(&id);
                Step::Missing
            }
        }
    };
    ORACLE.with(|cell| *cell.borrow_mut() = Some(oracle));
    step
}

fn memo_complete(id: u32, outcome: Result<Box<dyn Any>, PanicInfo>) -> Result<(), ReadFault> {
    let mut oracle = ORACLE
        .with(|cell| cell.borrow_mut().take())
        .expect("oracle evaluation is active");
    oracle.computing.remove(&id);
    let result = match outcome {
        Ok(value) => {
            oracle.values.insert(id, Rc::from(value));
            Ok(())
        }
        Err(_panic) => {
            oracle.poisoned.insert(id);
            Err(ReadFault::ComputedPanicked)
        }
    };
    ORACLE.with(|cell| *cell.borrow_mut() = Some(oracle));
    result
}

fn oracle_value(id: u32) -> Option<Rc<dyn Any>> {
    ORACLE.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|oracle| oracle.values.get(&id).cloned())
    })
}

struct OracleSession {
    _not_send: PhantomData<*const ()>,
}

impl OracleSession {
    fn new(runtime: &Runtime) -> Self {
        let runtime = runtime.state.borrow().id;
        ORACLE.with(|cell| {
            *cell.borrow_mut() = Some(OracleMemos {
                runtime,
                values: BTreeMap::new(),
                computing: BTreeSet::new(),
                poisoned: BTreeSet::new(),
            });
        });
        Self {
            _not_send: PhantomData,
        }
    }
}

impl Drop for OracleSession {
    fn drop(&mut self) {
        ORACLE.with(|cell| *cell.borrow_mut() = None);
    }
}

// ---------------------------------------------------------------------------
// 独立参考状态机。
// ---------------------------------------------------------------------------

/// 应用级状态；两个窗口共享同一份，与 `Runtime::setup` 里的 Signal/Store 对应。
#[derive(Clone, Debug, PartialEq)]
struct Model {
    counter: i64,
    draft: String,
    shown: bool,
    checked: bool,
    filter: Filter,
    next_row: u64,
    rows: Vec<RowData>,
    /// 窗口化 `For` 的请求窗口；两个窗口共享同一请求，运行时按行数夹取。
    row_first: usize,
    row_count: usize,
    /// 运行时全局的输入焦点窗口索引：`move_focus` 与 `show` 指向目标窗口，
    /// `hide` 聚焦窗口时退到第一个可见窗口。它不是任何一棵树的快照字段。
    focused_window: usize,
    windows: [WindowState; WINDOW_COUNT],
}

/// 一行待办；`id` 是参考状态机自己的稳定身份，与 Store 的 `TodoId` 数字无关。
#[derive(Clone, Debug, PartialEq)]
struct RowData {
    id: u64,
    title: String,
    completed: bool,
}

/// 一个文本输入的本地编辑状态。
#[derive(Clone, Debug, Default, PartialEq)]
struct Edit {
    caret: usize,
    /// 选择的活动端；折叠时与 `caret` 相同。
    anchor: usize,
    /// 归一化的选择区间；`set_caret` 折叠为 `Some((offset, offset))`。
    selection: Option<(usize, usize)>,
}

/// 每个窗口自己的焦点与编辑状态；内容共享，本地状态不共享。
#[derive(Clone, Debug, PartialEq)]
struct WindowState {
    focus: Option<FocusTarget>,
    edit: Edit,
    visible: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusTarget {
    Draft,
    Inc,
}

impl FocusTarget {
    const fn label(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Inc => "inc",
        }
    }
}

const WINDOW_COUNT: usize = 2;
const WINDOW_KEYS: [&str; WINDOW_COUNT] = ["main", "side"];
const MAX_ROWS: usize = 6;

impl Default for Model {
    fn default() -> Self {
        Self {
            counter: 0,
            draft: String::new(),
            shown: true,
            checked: false,
            filter: Filter::All,
            next_row: 0,
            rows: Vec::new(),
            // 行数上限小于运行时默认窗口（32），初始请求等价于全量挂载。
            row_first: 0,
            row_count: MAX_ROWS,
            // `mount_incremental` 最后显示 side 窗口，`show` 把输入焦点移过去。
            focused_window: 1,
            windows: std::array::from_fn(|_| WindowState {
                focus: None,
                edit: Edit::default(),
                visible: true,
            }),
        }
    }
}

impl Model {
    fn window(&self, index: usize) -> &WindowState {
        &self.windows[index]
    }

    /// 当前过滤条件下按 Store 顺序可见的行。
    fn visible_rows(&self) -> Vec<&RowData> {
        self.rows
            .iter()
            .filter(|row| self.filter.matches(row.completed))
            .collect()
    }

    fn row(&self, id: u64) -> Option<&RowData> {
        self.rows.iter().find(|row| row.id == id)
    }

    /// 窗口化 `For` 的有效行窗口：运行时把请求夹到当前过滤结果内。
    fn effective_row_window(&self) -> (usize, usize, usize) {
        let total = self.visible_rows().len();
        let first = self.row_first.min(total);
        let count = self.row_count.min(total - first);
        (first, count, total)
    }

    /// 实际挂载的行；窗口外的行没有节点，也没有可派发的目标。
    fn mounted_rows(&self) -> Vec<&RowData> {
        let (first, count, _) = self.effective_row_window();
        self.visible_rows()
            .into_iter()
            .skip(first)
            .take(count)
            .collect()
    }

    /// 呈现相关的行状态：总数、首行与已挂载行的标题/完成状态。
    fn row_state(&self) -> (usize, usize, Vec<(String, bool)>) {
        let (first, _, total) = self.effective_row_window();
        let mounted = self
            .mounted_rows()
            .into_iter()
            .map(|row| (row.title.clone(), row.completed))
            .collect();
        (total, first, mounted)
    }
}

fn text_len(text: &str) -> usize {
    text.chars().count()
}

/// 视图里的过滤按钮 label；与 `Filter::as_str` 的词表无关。
const fn filter_label(filter: Filter) -> &'static str {
    match filter {
        Filter::All => "filter-all",
        Filter::Active => "filter-active",
        Filter::Completed => "filter-done",
    }
}

/// 所有窗口都渲染同一份共享状态；只有内容（而非窗口可见性）参与比较。
fn app_content_changed(before: &Model, after: &Model) -> bool {
    if before.counter != after.counter
        || before.draft != after.draft
        || before.shown != after.shown
        || before.checked != after.checked
        || before.filter != after.filter
    {
        return true;
    }
    before.row_state() != after.row_state()
}

/// 窗口本地的焦点/编辑状态变化；窗口可见性单独结算。
fn window_content_changed(before: &Model, after: &Model, index: usize) -> bool {
    before.window(index).focus != after.window(index).focus
        || before.window(index).edit != after.window(index).edit
}

// ---------------------------------------------------------------------------
// 操作与确定性 RNG。
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Op {
    Inc(usize),
    ToggleShow(usize),
    ToggleCheck(usize),
    SetDraft(usize, String),
    Reset(usize),
    AddRow(usize),
    Reorder(usize),
    /// 行窗口请求；两个窗口共享同一请求，`window` 只用于操作描述。
    Scroll(usize, usize, usize),
    ToggleRow(usize, u64),
    RemoveRow(usize, u64),
    Filter(usize, Filter),
    Focus(usize, FocusTarget),
    SetCaret(usize, usize),
    SetSelection(usize, usize, usize),
    Hide(usize),
    Show(usize),
}

impl Op {
    const fn window(&self) -> usize {
        match self {
            Self::Inc(window)
            | Self::ToggleShow(window)
            | Self::ToggleCheck(window)
            | Self::SetDraft(window, _)
            | Self::Reset(window)
            | Self::AddRow(window)
            | Self::Reorder(window)
            | Self::Scroll(window, _, _)
            | Self::ToggleRow(window, _)
            | Self::RemoveRow(window, _)
            | Self::Filter(window, _)
            | Self::Focus(window, _)
            | Self::SetCaret(window, _)
            | Self::SetSelection(window, _, _)
            | Self::Hide(window)
            | Self::Show(window) => *window,
        }
    }

    fn describe(&self) -> String {
        let window = WINDOW_KEYS[self.window()];
        match self {
            Self::Inc(_) => format!("inc {window}"),
            Self::ToggleShow(_) => format!("toggle show {window}"),
            Self::ToggleCheck(_) => format!("toggle check {window}"),
            Self::SetDraft(_, text) => format!("set draft {window} {text:?}"),
            Self::Reset(_) => format!("reset {window}"),
            Self::AddRow(_) => format!("add row {window}"),
            Self::Reorder(_) => format!("reorder rows {window}"),
            Self::Scroll(_, first, count) => {
                format!("scroll rows {first}..{} in both windows", first + count)
            }
            Self::ToggleRow(_, id) => format!("toggle row {id} in {window}"),
            Self::RemoveRow(_, id) => format!("remove row {id} in {window}"),
            Self::Filter(_, filter) => format!("filter {window} {}", filter.as_str()),
            Self::Focus(_, target) => format!("focus {window} {}", target.label()),
            Self::SetCaret(_, offset) => format!("set caret {window} {offset}"),
            Self::SetSelection(_, anchor, focus) => {
                format!("set selection {window} {anchor}..{focus}")
            }
            Self::Hide(_) => format!("hide {window}"),
            Self::Show(_) => format!("show {window}"),
        }
    }
}

/// SplitMix64：确定性、无外部依赖；同一种子产生同一序列。
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        value ^ (value >> 31)
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    fn pick(&mut self, bound: usize) -> usize {
        self.below(bound as u64) as usize
    }
}

/// 文本同时包含单字节与多字节字符：caret/offset 断言必须按字符计数，
/// 纯 ASCII 会让 `chars().count()` 与 `len()` 无法区分。
const TEXT_ALPHABET: [char; 5] = ['a', 'b', '你', '界', '🙂'];

fn random_text(rng: &mut Rng) -> String {
    let length = rng.below(5) as usize;
    (0..length)
        .map(|_| TEXT_ALPHABET[rng.pick(TEXT_ALPHABET.len())])
        .collect()
}

/// 应用一步参考转移。调用方必须先克隆出 `after`，runtime 用 `after` 派发。
fn apply_model(model: &mut Model, op: &Op) {
    match op {
        Op::Inc(_) => model.counter += 1,
        Op::ToggleShow(_) => model.shown = !model.shown,
        Op::ToggleCheck(_) => model.checked = !model.checked,
        Op::SetDraft(window, text) => set_draft(model, *window, text),
        Op::Reset(_) => reset_model(model),
        Op::AddRow(_) => {
            let id = model.next_row;
            model.next_row += 1;
            model.rows.push(RowData {
                id,
                title: format!("row {id}"),
                completed: false,
            });
        }
        Op::Reorder(_) => model.rows.reverse(),
        Op::Scroll(_, first, count) => {
            model.row_first = *first;
            model.row_count = *count;
        }
        Op::ToggleRow(_, id) => {
            if let Some(row) = model.rows.iter_mut().find(|row| row.id == *id) {
                row.completed = !row.completed;
            }
        }
        Op::RemoveRow(_, id) => model.rows.retain(|row| row.id != *id),
        Op::Filter(_, filter) => model.filter = *filter,
        Op::Focus(window, target) => {
            model.windows[*window].focus = Some(*target);
            model.focused_window = *window;
        }
        Op::SetCaret(window, offset) => {
            let offset = (*offset).min(text_len(&model.draft));
            model.windows[*window].edit = Edit {
                caret: offset,
                anchor: offset,
                selection: Some((offset, offset)),
            };
        }
        Op::SetSelection(window, anchor, focus) => {
            let length = text_len(&model.draft);
            let anchor = (*anchor).min(length);
            let focus = (*focus).min(length);
            model.windows[*window].edit = Edit {
                caret: focus,
                anchor,
                selection: Some((anchor.min(focus), anchor.max(focus))),
            };
        }
        Op::Hide(window) => {
            model.windows[*window].visible = false;
            if model.focused_window == *window {
                model.focused_window = first_visible_window(model);
            }
        }
        Op::Show(window) => {
            model.windows[*window].visible = true;
            model.focused_window = *window;
        }
    }
}

/// `show` 与隐藏焦点窗口后的回落都选择目录顺序里的第一个可见窗口。
fn first_visible_window(model: &Model) -> usize {
    (0..WINDOW_COUNT)
        .find(|index| model.windows[*index].visible)
        .expect("at least one window stays visible")
}

/// 一次整体替换：目标窗口采用 `Replace` 语义（caret 落在新值末尾），
/// 其余窗口走外部替换的夹取规则。
fn set_draft(model: &mut Model, window: usize, text: &str) {
    let length = text_len(text);
    model.windows[window].edit = Edit {
        caret: length,
        anchor: length,
        selection: None,
    };
    for (index, state) in model.windows.iter_mut().enumerate() {
        if index == window {
            continue;
        }
        let edit = &mut state.edit;
        if length == 0 {
            edit.caret = 0;
            edit.anchor = 0;
            edit.selection = None;
        } else if edit.selection.is_some_and(|(_, end)| end > length) {
            edit.caret = length;
            edit.anchor = length;
            edit.selection = None;
        } else {
            edit.caret = edit.caret.min(length);
            edit.anchor = edit.anchor.min(length);
        }
    }
    model.draft.clear();
    model.draft.push_str(text);
}

/// `Reset` 按钮一次写三个 Signal；草稿清空对两个窗口都是外部替换。
fn reset_model(model: &mut Model) {
    let draft_changed = !model.draft.is_empty();
    model.counter = 0;
    model.shown = true;
    if !draft_changed {
        return;
    }
    model.draft.clear();
    for state in &mut model.windows {
        state.edit = Edit::default();
    }
}

/// 生成下一步操作。`set_caret`/`set_selection` 也会生成原位重复：它们
/// 现在是只在编辑状态真的变化时才提交 revision 的直接原语。
fn random_op(rng: &mut Rng, model: &Model) -> Op {
    for _ in 0..24 {
        if let Some(op) = try_random_op(rng, model) {
            return op;
        }
    }
    let window = (0..WINDOW_COUNT)
        .find(|index| model.windows[*index].visible)
        .expect("at least one window stays visible");
    Op::Inc(window)
}

fn try_random_op(rng: &mut Rng, model: &Model) -> Option<Op> {
    let visible: Vec<usize> = (0..WINDOW_COUNT)
        .filter(|index| model.windows[*index].visible)
        .collect();
    let window = visible[rng.pick(visible.len())];
    match rng.below(17) {
        0..=2 => Some(Op::Inc(window)),
        3 => Some(Op::ToggleShow(window)),
        4 => Some(Op::ToggleCheck(window)),
        5 => {
            let mut text = random_text(rng);
            if text == model.draft {
                text.push('a');
            }
            Some(Op::SetDraft(window, text))
        }
        6 => Some(Op::Reset(window)),
        7 => (model.rows.len() < MAX_ROWS).then_some(Op::AddRow(window)),
        8 => Some(Op::Reorder(window)),
        9 => {
            // Scroll 同时应用到两个窗口；隐藏/关闭挂起窗口会拒绝结构输入，
            // 因此只在两个窗口都可见时生成该操作。
            if visible.len() < WINDOW_COUNT {
                return None;
            }
            let (_, _, total) = model.effective_row_window();
            Some(Op::Scroll(
                window,
                rng.below(total as u64 + 2) as usize,
                rng.below(4) as usize,
            ))
        }
        10 => {
            let rows = model.mounted_rows();
            if rows.is_empty() {
                return None;
            }
            let row = rows[rng.pick(rows.len())];
            Some(Op::ToggleRow(window, row.id))
        }
        11 => {
            let rows = model.mounted_rows();
            if rows.is_empty() {
                return None;
            }
            let row = rows[rng.pick(rows.len())];
            Some(Op::RemoveRow(window, row.id))
        }
        12 => {
            let filter = [Filter::All, Filter::Active, Filter::Completed][rng.pick(3)];
            (filter != model.filter).then_some(Op::Filter(window, filter))
        }
        13 => {
            let target = [FocusTarget::Draft, FocusTarget::Inc][rng.pick(2)];
            Some(Op::Focus(window, target))
        }
        14 => {
            let length = text_len(&model.draft);
            Some(Op::SetCaret(window, rng.below(length as u64 + 1) as usize))
        }
        15 => {
            let length = text_len(&model.draft);
            Some(Op::SetSelection(
                window,
                rng.below(length as u64 + 1) as usize,
                rng.below(length as u64 + 1) as usize,
            ))
        }
        _ => {
            if let Some(hidden) = (0..WINDOW_COUNT).find(|index| !model.windows[*index].visible) {
                Some(Op::Show(hidden))
            } else if visible.len() > 1 {
                Some(Op::Hide(window))
            } else {
                None
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 被测视图与按参考状态重挂载的参考视图。
// ---------------------------------------------------------------------------

/// 应用级共享状态；所有窗口渲染同一份句柄。
#[derive(Clone, Copy)]
struct AppSignals {
    counter: Signal<i64>,
    draft: Signal<String>,
    shown: Signal<bool>,
    checked: Signal<bool>,
    filter: Signal<Filter>,
    seq: Signal<u64>,
    store: Store,
}

impl AppSignals {
    fn fresh() -> Self {
        Self {
            counter: signal(0),
            draft: signal(String::new()),
            shown: signal(true),
            checked: signal(false),
            filter: signal(Filter::All),
            seq: signal(0),
            store: Store::new(),
        }
    }

    fn initial(model: &Model) -> Self {
        Self {
            counter: signal(model.counter),
            draft: signal(model.draft.clone()),
            shown: signal(model.shown),
            checked: signal(model.checked),
            filter: signal(model.filter),
            seq: signal(model.next_row),
            store: Store::new(),
        }
    }
}

/// 把参考状态机的行灌入参考运行时的 Store。
///
/// 事务只在 Event 阶段执行，所以参考界面挂载后通过 `seed` 按钮触发一次。
/// 先插入全部行取得业务键，再按原始顺序设置完成标记；过滤条件不参与。
fn seed_rows(store: Store, rows: &[(String, bool)]) {
    let mut transaction = Transaction::new();
    for (title, _) in rows {
        transaction = transaction.write(store.insert(title.clone()));
    }
    let _ = transaction.finish();
    let keys = store.keys(Filter::All);
    let mut transaction = Transaction::new();
    for ((_, completed), key) in rows.iter().zip(keys) {
        if *completed {
            transaction = transaction.write(store.set_completed(key, true));
        }
    }
    let _ = transaction.finish();
}

fn oracle_view(signals: AppSignals, seed: Rc<Vec<(String, bool)>>) -> MountResult<MountPlan> {
    let label = memo(move || format!("count={}", signals.counter.get()));
    let detail = memo(move || format!("{}/{}", label.get(), signals.draft.get()));
    let tint = memo(move || {
        if signals.checked.get() {
            Color::Role(ColorRole::TextMuted)
        } else {
            Color::Role(ColorRole::TextDefault)
        }
    });
    let bar_height = memo(move || {
        if signals.shown.get() {
            px(4.0)
        } else {
            px(8.0)
        }
    });
    let fade = memo(move || {
        if signals.counter.get() % 2 == 0 {
            Opacity::percent(100)
        } else {
            Opacity::percent(60)
        }
    });
    let shown_branch = memo(move || signals.shown.get());
    let all_pressed = memo(move || signals.filter.get() == Filter::All);
    let active_pressed = memo(move || signals.filter.get() == Filter::Active);
    let done_pressed = memo(move || signals.filter.get() == Filter::Completed);
    // For 的行集读源；Store 句柄通过 `store` 属性提供行数据。
    let visible = memo(move || signals.store.rows(signals.filter.get()));

    view! {
        <view>
            <text aria:label="label">{label}</text>
            <text aria:label="detail" style:color={tint}>{detail}</text>
            <view
                aria:label="bar"
                style:height={bar_height}
                style:opacity={fade}
            />
            <text-input
                aria:label="draft"
                value={signals.draft}
                on:input={move |next: String| signals.draft.set(next)}
            />
            <checkbox
                aria:label="check"
                checked={signals.checked}
                on:change={move |next: bool| signals.checked.set(next)}
            />
            <button
                aria:label="inc"
                on:click={move || signals.counter.set(signals.counter.get() + 1)}
            >"Inc"</button>
            <button
                aria:label="show"
                on:click={move || signals.shown.set(!signals.shown.get())}
            >"Show"</button>
            <button
                aria:label="reset"
                on:click={move || {
                    signals.counter.set(0);
                    signals.draft.set(String::new());
                    signals.shown.set(true);
                }}
            >"Reset"</button>
            <button
                aria:label="add"
                on:click={move || {
                    let next = signals.seq.get();
                    signals.seq.set(next + 1);
                    let _ = Transaction::new()
                        .write(signals.store.insert(format!("row {next}")))
                        .finish();
                }}
            >"Add"</button>
            <button
                aria:label="reorder"
                on:click={move || {
                    let mut keys = signals.store.keys(Filter::All);
                    keys.reverse();
                    let _ = Transaction::new()
                        .write(signals.store.reorder(&keys))
                        .finish();
                }}
            >"Reorder"</button>
            <button
                aria:label="filter-all"
                aria:pressed={all_pressed}
                on:click={move || signals.filter.set(Filter::All)}
            >"All"</button>
            <button
                aria:label="filter-active"
                aria:pressed={active_pressed}
                on:click={move || signals.filter.set(Filter::Active)}
            >"Active"</button>
            <button
                aria:label="filter-done"
                aria:pressed={done_pressed}
                on:click={move || signals.filter.set(Filter::Completed)}
            >"Done"</button>
            // 增量窗口携带空的 seed 列表，从不点击；参考运行时点击一次，
            // 把参考状态的行灌入自己的 Store。
            <button
                aria:label="seed"
                on:click={move || seed_rows(signals.store, &seed)}
            >"Seed"</button>
            <Show
                when={shown_branch}
                fallback={|| view! { <text>"hidden"</text> }}
            >
                <text aria:label="shown">"visible"</text>
            </Show>
            <view role="list">
                <For
                    each={visible}
                    store={signals.store}
                    key={|row: &TodoRow| row.id}
                    // 零行高让 spacer 高度不随总数变化：行带元数据必须自己
                    // 参与可观察变化，否则 total_rows 的变化会被漏掉。
                    row-height={px(0.0)}
                >
                    {move |row: RowView<TodoRow>| {
                        let id = row.key();
                        let title = row.get().title.clone();
                        let done_label = format!("done {title}");
                        let remove_label = format!("remove {title}");
                        let completed = signals.store.completed(id);
                        view! {
                            <view role="listitem">
                                <text>{once(title.clone())}</text>
                                <checkbox
                                    aria:label={once(done_label)}
                                    checked={completed}
                                    on:change={move |done: bool| {
                                        let _ = Transaction::new()
                                            .write(signals.store.set_completed(id, done))
                                            .finish();
                                    }}
                                />
                                <button
                                    aria:label={once(remove_label)}
                                    on:click={move || {
                                        let _ = Transaction::new()
                                            .write(signals.store.remove(id))
                                            .finish();
                                    }}
                                >"Remove"</button>
                            </view>
                        }
                    }}
                </For>
            </view>
        </view>
    }
}

fn mount_incremental() -> (Runtime, [WindowCtx; 2]) {
    let mut app = Runtime::new();
    let signals = app
        .setup(AppSignals::fresh)
        .expect("application setup for the oracle");
    app.declare_windows(move |catalog| {
        let seed_main: Rc<Vec<(String, bool)>> = Rc::new(Vec::new());
        let seed_side = seed_main.clone();
        catalog.window(WindowSpec::new("main").primary(), move |_| {
            oracle_view(signals, seed_main.clone())
        })?;
        catalog.window(WindowSpec::new("side"), move |_| {
            oracle_view(signals, seed_side.clone())
        })?;
        Ok(())
    })
    .expect("window catalog for the oracle");
    let main = app.show("main").expect("show main window");
    let side = app.show("side").expect("show side window");
    (app, [main, side])
}

/// 从参考状态重新挂载一份窗口，用于生成参考快照。
///
/// 参考运行时只读取参考状态构造的 Signal/Store；它不复用被测运行时的
/// 绑定表、属性缓存或 memo 值。
fn reference_snapshot(model: &Model) -> Result<PresentationSnapshot, String> {
    let mut app = Runtime::new();
    let signals = app
        .setup(|| AppSignals::initial(model))
        .map_err(|error| format!("reference setup: {error}"))?;
    let seed: Rc<Vec<(String, bool)>> = Rc::new(
        model
            .rows
            .iter()
            .map(|row| (row.title.clone(), row.completed))
            .collect(),
    );
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("ref").primary(), move |_| {
            oracle_view(signals, seed.clone())
        })?;
        Ok(())
    })
    .map_err(|error| format!("reference catalog: {error}"))?;
    let window = app
        .show("ref")
        .map_err(|error| format!("reference show: {error}"))?;
    let report = app
        .click_label("seed")
        .map_err(|error| format!("reference seed: {error}"))?;
    if !report.is_ok() {
        return Err(format!(
            "reference seed reported errors: {:?}",
            report.errors()
        ));
    }
    // 参考运行时必须看到与模型相同的窗口化 For 请求；有效窗口由同一运行
    // 时规则夹取，因此两侧挂载的行与行带元数据一致。
    let band = band_key(&app, window)?;
    app.set_row_window(window, band, model.row_first, model.row_count)
        .map_err(|error| format!("reference row window: {error}"))?;
    app.presentation_of(window)
        .map_err(|error| format!("reference snapshot: {error}"))
}

// ---------------------------------------------------------------------------
// 操作驱动：全部走公开 API，目标按窗口内的 label 解析。
// ---------------------------------------------------------------------------

fn node_by_label(app: &Runtime, window: WindowCtx, label: &str) -> Result<NodeId, String> {
    app.presentation_of(window)
        .map_err(|error| format!("snapshot: {error}"))?
        .find_label(label)
        .map(SnapshotElement::id)
        .ok_or_else(|| format!("no element labelled {label}"))
}

/// 窗口里唯一的窗口化 `For` 行带键；隐藏窗口也能读取已提交快照。
fn band_key(app: &Runtime, window: WindowCtx) -> Result<u64, String> {
    let snapshots = app.state.borrow().oracle_tree_snapshots();
    let snapshot = snapshots
        .iter()
        .find_map(|(slot, _, snapshot)| match slot {
            TreeSlot::Window(key) if *key == window.key() => Some(snapshot),
            TreeSlot::Root | TreeSlot::Window(_) => None,
        })
        .ok_or_else(|| format!("window {} has no committed snapshot", window.key()))?;
    snapshot
        .row_bands()
        .first()
        .map(|band| band.key())
        .ok_or_else(|| format!("window {} has no windowed For", window.key()))
}

fn check_report(report: &TurnReport, op: &Op) -> Result<(), String> {
    if report.is_ok() {
        Ok(())
    } else {
        Err(format!(
            "{op:?}: turn reported errors: {:?}",
            report.errors()
        ))
    }
}

fn apply_op(
    app: &mut Runtime,
    windows: &[WindowCtx; WINDOW_COUNT],
    before: &Model,
    after: &Model,
    op: &Op,
) -> Result<(), String> {
    let window = windows[op.window()];
    match op {
        Op::Inc(_) => {
            let node = node_by_label(app, window, "inc")?;
            check_report(
                &app.click_node(node).map_err(|error| error.to_string())?,
                op,
            )
        }
        Op::ToggleShow(_) => {
            let node = node_by_label(app, window, "show")?;
            check_report(
                &app.click_node(node).map_err(|error| error.to_string())?,
                op,
            )
        }
        Op::ToggleCheck(_) => {
            let node = node_by_label(app, window, "check")?;
            // on:change 的目标就是参考状态转移后的值。
            check_report(
                &app.change_node(node, after.checked)
                    .map_err(|error| error.to_string())?,
                op,
            )
        }
        Op::SetDraft(_, text) => {
            let node = node_by_label(app, window, "draft")?;
            check_report(
                &app.replace_text_node(node, text)
                    .map_err(|error| error.to_string())?,
                op,
            )
        }
        Op::Reset(_) => {
            let node = node_by_label(app, window, "reset")?;
            check_report(
                &app.click_node(node).map_err(|error| error.to_string())?,
                op,
            )
        }
        Op::AddRow(_) => {
            let node = node_by_label(app, window, "add")?;
            check_report(
                &app.click_node(node).map_err(|error| error.to_string())?,
                op,
            )
        }
        Op::Reorder(_) => {
            let node = node_by_label(app, window, "reorder")?;
            check_report(
                &app.click_node(node).map_err(|error| error.to_string())?,
                op,
            )
        }
        Op::Scroll(_, first, count) => {
            // 请求同时应用到两个窗口：模型只维护一份行窗口，参考挂载也只需一份。
            for window in windows {
                let band = band_key(app, *window)?;
                app.set_row_window(*window, band, *first, *count)
                    .map_err(|error| format!("{op:?}: {error}"))?;
            }
            Ok(())
        }
        Op::ToggleRow(_, id) => {
            let row = after
                .row(*id)
                .ok_or_else(|| format!("row {id} disappeared from the reference state"))?;
            let node = node_by_label(app, window, &format!("done {}", row.title))?;
            check_report(
                &app.change_node(node, row.completed)
                    .map_err(|error| error.to_string())?,
                op,
            )
        }
        Op::RemoveRow(_, id) => {
            let row = before
                .row(*id)
                .ok_or_else(|| format!("row {id} is missing from the reference state"))?;
            let node = node_by_label(app, window, &format!("remove {}", row.title))?;
            check_report(
                &app.click_node(node).map_err(|error| error.to_string())?,
                op,
            )
        }
        Op::Filter(_, filter) => {
            let node = node_by_label(app, window, filter_label(*filter))?;
            check_report(
                &app.click_node(node).map_err(|error| error.to_string())?,
                op,
            )
        }
        Op::Focus(_, target) => check_report(
            &app.focus_node(node_by_label(app, window, target.label())?)
                .map_err(|error| error.to_string())?,
            op,
        ),
        Op::SetCaret(_, offset) => {
            let node = node_by_label(app, window, "draft")?;
            check_report(
                &app.set_caret_node(node, *offset)
                    .map_err(|error| error.to_string())?,
                op,
            )
        }
        Op::SetSelection(_, anchor, focus) => {
            let node = node_by_label(app, window, "draft")?;
            check_report(
                &app.set_selection_node(node, *anchor, *focus)
                    .map_err(|error| error.to_string())?,
                op,
            )
        }
        Op::Hide(_) => app.hide(window).map_err(|error| format!("{op:?}: {error}")),
        Op::Show(_) => app
            .show(WINDOW_KEYS[op.window()])
            .map(|_| ())
            .map_err(|error| format!("{op:?}: {error}")),
    }
}

// ---------------------------------------------------------------------------
// 绑定与 memo 的独立重算（所有树，包括隐藏窗口）。
// ---------------------------------------------------------------------------

fn verify_state(runtime: &Runtime) -> Result<(), String> {
    let _session = OracleSession::new(runtime);
    crate::context::install(&runtime.state, || {
        verify_bindings(runtime)?;
        verify_memos(runtime)
    })
}

fn verify_bindings(runtime: &Runtime) -> Result<(), String> {
    let trees: Vec<TreeKey> = {
        let state = runtime.state.borrow();
        state.oracle_trees()
    };
    for tree in trees {
        check_slots::<String>(runtime, tree)?;
        check_slots::<bool>(runtime, tree)?;
        check_slots::<Color>(runtime, tree)?;
        check_slots::<Length>(runtime, tree)?;
        check_slots::<Opacity>(runtime, tree)?;
        check_slots::<RowSet>(runtime, tree)?;
    }
    Ok(())
}

fn check_slots<T>(runtime: &Runtime, tree: TreeKey) -> Result<(), String>
where
    T: BindingValue + fmt::Debug,
{
    let entries = {
        let state = runtime.state.borrow();
        let Some(table) = state.oracle_binding_table(tree) else {
            return Ok(());
        };
        table.oracle_entries::<T>()
    };
    for (reference, source, current, fault) in entries {
        if source.is_static() {
            // 静态/once 值只在挂载时写入；它们没有可重算的来源。
            continue;
        }
        match source.evaluate::<T>() {
            ReadValue::Value(expected) => match &current {
                Some(current) if *current == expected => {}
                other => {
                    return Err(format!(
                        "binding {tree:?} {reference:?} ({source:?}): \
                         fresh value {expected:?}, committed {other:?}"
                    ));
                }
            },
            ReadValue::Fault(expected_fault) => {
                if fault.as_ref() != Some(&expected_fault) {
                    return Err(format!(
                        "binding {tree:?} {reference:?} ({source:?}): \
                         fresh fault {expected_fault:?}, committed fault {fault:?}"
                    ));
                }
            }
        }
    }
    Ok(())
}

fn verify_memos(runtime: &Runtime) -> Result<(), String> {
    let live: BTreeSet<u32> = {
        let state = runtime.state.borrow();
        state.live_memo_set(&BTreeSet::new())
    };
    let poisoned: BTreeSet<u32> = {
        let state = runtime.state.borrow();
        live.iter()
            .copied()
            .filter(|id| state.memos[*id as usize].poisoned.is_some())
            .collect()
    };
    for id in &live {
        // 毒化是生产合同的一部分：重算必须同样 panic，值则保留最后一次提交。
        match (force_memo(*id)?, poisoned.contains(id)) {
            (Fresh::Panicked, true) | (Fresh::Value, false) => {}
            (Fresh::Panicked, false) => {
                return Err(format!(
                    "memo {id}: panicked during independent recomputation but production \
                     is not poisoned"
                ));
            }
            (Fresh::Value, true) => {
                return Err(format!(
                    "memo {id}: production is poisoned but an independent recomputation \
                     succeeded"
                ));
            }
        }
    }
    let state = runtime.state.borrow();
    for id in &live {
        let cell = &state.memos[*id as usize];
        // abort_memo 故意让毒化 memo 保持 dirty，直到依赖变化后重新求值；
        // 读取路径先看 poisoned，旧值仍然有效。
        if cell.dirty && !poisoned.contains(id) {
            return Err(format!(
                "memo {id}: live memo is still dirty after the turn"
            ));
        }
        let Some(committed) = &cell.value else {
            return Err(format!("memo {id}: live memo has no committed value"));
        };
        if poisoned.contains(id) {
            // 毒化后保留旧值是合同，不能与一次必然 panic 的重算比较。
            continue;
        }
        let Some(compute) = &cell.compute else {
            return Err(format!("memo {id}: live memo has no compute"));
        };
        let fresh = oracle_value(*id).ok_or_else(|| format!("memo {id}: no fresh oracle value"))?;
        if !compute.eq(&**committed, &*fresh) {
            return Err(format!(
                "memo {id}: committed value differs from an independent recomputation"
            ));
        }
    }
    Ok(())
}

/// 一次独立重算的结果；poisoned memo 的重算只能以 panic 结束。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fresh {
    Value,
    Panicked,
}

fn force_memo(id: u32) -> Result<Fresh, String> {
    loop {
        match memo_step(id) {
            Step::Ready => return Ok(Fresh::Value),
            Step::Cycle => {
                return Err(format!("memo {id}: cycle during independent recomputation"));
            }
            Step::Missing => {
                return Err(format!(
                    "memo {id}: compute missing during independent recomputation"
                ));
            }
            Step::Disposed => {
                return Err(format!(
                    "memo {id}: disposed during independent recomputation"
                ));
            }
            Step::Poisoned => return Ok(Fresh::Panicked),
            Step::Compute(compute) => match catch_panic(|| compute.call()) {
                Ok(value) => {
                    memo_complete(id, Ok(value)).map_err(|fault| format!("memo {id}: {fault}"))?;
                }
                Err(panic) => {
                    let _ = memo_complete(id, Err(panic));
                    return Ok(Fresh::Panicked);
                }
            },
        }
    }
}

/// 生产 memo 最后一次已提交值；只用于毒化 memo 的合同回落。
fn production_memo_value<T: ReactiveValue>(id: u32) -> Option<T> {
    crate::context::with_state(|state| {
        let cell = &state.memos[id as usize];
        if cell.disposed {
            return None;
        }
        cell.value
            .as_ref()
            .map(|value| (*value.clone().downcast::<T>().expect("memo value type")).clone())
    })
}

// ---------------------------------------------------------------------------
// 规范化快照比较。
// ---------------------------------------------------------------------------

/// 规范化后的快照节点。`edit` 只在“可观察状态”模式里出现：参考挂载没有
/// 回放输入历史，结构比较只覆盖 tag、属性、样式、文本与行顺序。
#[derive(Clone, Debug, PartialEq, Eq)]
enum Projected {
    Element {
        tag: &'static str,
        attrs: Vec<(String, String)>,
        styles: Vec<String>,
        semantics: String,
        edit: Option<String>,
        children: Vec<Projected>,
    },
    Text(String),
}

fn project(snapshot: &PresentationSnapshot, local: bool) -> Vec<Projected> {
    snapshot
        .roots()
        .iter()
        .map(|node| project_node(node, local))
        .collect()
}

fn project_node(node: &SnapshotNode, local: bool) -> Projected {
    match node {
        SnapshotNode::Text(text) => Projected::Text(text.text().to_owned()),
        SnapshotNode::Element(element) => Projected::Element {
            tag: element.tag(),
            attrs: element
                .attrs()
                .iter()
                .map(|attr| {
                    let value = match attr.value() {
                        SnapshotValue::Text(text) => format!("text:{text}"),
                        SnapshotValue::Bool(flag) => format!("bool:{flag}"),
                    };
                    (attr.name().to_owned(), value)
                })
                .collect(),
            styles: element
                .styles()
                .iter()
                .map(|style| format!("{style:?}"))
                .collect(),
            // 语义投影（可聚焦性与已挂载动作）也是快照的一部分；参考视图与
            // 增量视图声明相同的处理器，因此两侧必须一致。
            semantics: format!("{:?}", element.semantics()),
            edit: local.then(|| projected_edit(element)).flatten(),
            children: element
                .children()
                .iter()
                .map(|child| project_node(child, local))
                .collect(),
        },
    }
}

fn projected_edit(element: &SnapshotElement) -> Option<String> {
    let edit = element.edit()?;
    Some(format!(
        "caret={} anchor={} selection={:?} preedit={:?} composition={:?}",
        edit.caret(),
        edit.anchor(),
        edit.selection(),
        edit.preedit(),
        edit.composition()
    ))
}

/// 一棵树的可观察状态：外观、焦点与结构/编辑投影。
///
/// 这是 revision 断言的比较对象——它覆盖快照里所有可能变化的字段。
#[derive(Clone, Debug, PartialEq, Eq)]
struct Observed {
    appearance: String,
    focus: Option<String>,
    /// 行带元数据也是快照的一部分；`total_rows` 变化可能不改变任何节点。
    row_bands: Vec<String>,
    roots: Vec<Projected>,
}

fn observed(snapshot: &PresentationSnapshot) -> Observed {
    Observed {
        appearance: format!("{:?}", snapshot.appearance()),
        focus: snapshot.focus().map(|id| {
            snapshot
                .node(id)
                .and_then(|element| element.label().map(str::to_owned))
                .unwrap_or_else(|| format!("node-{}", id.slot()))
        }),
        row_bands: snapshot
            .row_bands()
            .iter()
            .map(|band| {
                format!(
                    "{}:{:?}:{}:{}",
                    band.key(),
                    band.row_height(),
                    band.first_row(),
                    band.total_rows()
                )
            })
            .collect(),
        roots: project(snapshot, true),
    }
}

fn verify_projection(
    snapshot: &PresentationSnapshot,
    reference: &[Projected],
    window: &str,
) -> Result<(), String> {
    let incremental = project(snapshot, false);
    if incremental == reference {
        return Ok(());
    }
    let detail = projection_diff("root", &incremental, reference)
        .unwrap_or_else(|| String::from("the normalized projections differ"));
    Err(format!(
        "window {window}: normalized snapshot differs: {detail}
  incremental: {incremental:#?}
  reference:   {reference:#?}"
    ))
}

fn projection_diff(path: &str, left: &[Projected], right: &[Projected]) -> Option<String> {
    if left.len() != right.len() {
        return Some(format!(
            "{path}: {} children, reference has {}",
            left.len(),
            right.len()
        ));
    }
    left.iter()
        .zip(right)
        .enumerate()
        .find_map(|(index, (left, right))| diff_node(&format!("{path}/{index}"), left, right))
}

fn diff_node(path: &str, left: &Projected, right: &Projected) -> Option<String> {
    match (left, right) {
        (Projected::Text(left), Projected::Text(right)) => {
            (left != right).then(|| format!("{path}: text {left:?}, reference {right:?}"))
        }
        (
            Projected::Element {
                tag: left_tag,
                attrs: left_attrs,
                styles: left_styles,
                semantics: left_semantics,
                edit: left_edit,
                children: left_children,
            },
            Projected::Element {
                tag: right_tag,
                attrs: right_attrs,
                styles: right_styles,
                semantics: right_semantics,
                edit: right_edit,
                children: right_children,
            },
        ) => {
            if left_tag != right_tag {
                return Some(format!("{path}: tag {left_tag}, reference {right_tag}"));
            }
            if left_attrs != right_attrs {
                return Some(format!(
                    "{path} ({left_tag}) attrs {left_attrs:?}, reference {right_attrs:?}"
                ));
            }
            if left_styles != right_styles {
                return Some(format!(
                    "{path} ({left_tag}) styles {left_styles:?}, reference {right_styles:?}"
                ));
            }
            if left_semantics != right_semantics {
                return Some(format!(
                    "{path} ({left_tag}) semantics {left_semantics}, reference {right_semantics}"
                ));
            }
            if left_edit != right_edit {
                return Some(format!(
                    "{path} ({left_tag}) edit {left_edit:?}, reference {right_edit:?}"
                ));
            }
            projection_diff(&format!("{path}/children"), left_children, right_children)
        }
        _ => Some(format!(
            "{path}: node kind differs: {left:?}, reference {right:?}"
        )),
    }
}

// ---------------------------------------------------------------------------
// 参考状态断言与行身份。
// ---------------------------------------------------------------------------

fn verify_window_state(
    snapshot: &PresentationSnapshot,
    model: &Model,
    window: usize,
) -> Result<(), String> {
    let name = WINDOW_KEYS[window];
    let label = format!("count={}", model.counter);
    let actual = element_text(snapshot, "label")?;
    if actual != label {
        return Err(format!(
            "window {name}: label text: model {label:?}, snapshot {actual:?}"
        ));
    }
    let detail = format!("{}/{}", label, model.draft);
    let actual = element_text(snapshot, "detail")?;
    if actual != detail {
        return Err(format!(
            "window {name}: detail text: model {detail:?}, snapshot {actual:?}"
        ));
    }
    let draft = snapshot
        .find_label("draft")
        .ok_or_else(|| format!("window {name}: the draft input is missing"))?;
    if draft.text_value() != Some(model.draft.as_str()) {
        return Err(format!(
            "window {name}: draft value: model {:?}, snapshot {:?}",
            model.draft,
            draft.text_value()
        ));
    }
    let edit = draft
        .edit()
        .ok_or_else(|| format!("window {name}: the draft input has no edit state"))?;
    let expected = &model.window(window).edit;
    if (edit.caret(), edit.anchor(), edit.selection())
        != (expected.caret, expected.anchor, expected.selection)
    {
        return Err(format!(
            "window {name}: draft edit: model caret={} anchor={} selection={:?}, \
             snapshot caret={} anchor={} selection={:?}",
            expected.caret,
            expected.anchor,
            expected.selection,
            edit.caret(),
            edit.anchor(),
            edit.selection()
        ));
    }
    if edit.preedit().is_some() || edit.composition().is_some() {
        return Err(format!(
            "window {name}: unexpected preedit/composition in the draft input"
        ));
    }
    let checked = snapshot
        .find_label("check")
        .and_then(SnapshotElement::checked)
        .ok_or_else(|| format!("window {name}: the checkbox has no committed checked state"))?;
    if checked != model.checked {
        return Err(format!(
            "window {name}: checkbox: model {}, snapshot {checked}",
            model.checked
        ));
    }
    for filter in [Filter::All, Filter::Active, Filter::Completed] {
        let label = filter_label(filter);
        let pressed = snapshot
            .find_label(label)
            .map(SnapshotElement::pressed)
            .ok_or_else(|| format!("window {name}: the {label} button is missing"))?;
        if pressed != (model.filter == filter) {
            return Err(format!(
                "window {name}: {label} pressed={pressed}, model filter {:?}",
                model.filter
            ));
        }
    }
    let shown = snapshot.find_label("shown").is_some();
    if shown != model.shown {
        return Err(format!(
            "window {name}: Show branch: model shown={}, snapshot shown={shown}",
            model.shown
        ));
    }
    let interactive: [(&str, bool, &[SnapshotAction]); 11] = [
        ("draft", true, &[SnapshotAction::Input]),
        ("check", true, &[SnapshotAction::Change]),
        ("inc", true, &[SnapshotAction::Click]),
        ("show", true, &[SnapshotAction::Click]),
        ("reset", true, &[SnapshotAction::Click]),
        ("add", true, &[SnapshotAction::Click]),
        ("reorder", true, &[SnapshotAction::Click]),
        ("seed", true, &[SnapshotAction::Click]),
        ("filter-all", true, &[SnapshotAction::Click]),
        ("filter-active", true, &[SnapshotAction::Click]),
        ("filter-done", true, &[SnapshotAction::Click]),
    ];
    for (label, focusable, actions) in interactive {
        let element = snapshot
            .find_label(label)
            .ok_or_else(|| format!("window {name}: the {label} element is missing"))?;
        expect_semantics(element, label, focusable, actions)?;
    }
    for label in ["label", "detail", "bar"] {
        let element = snapshot
            .find_label(label)
            .ok_or_else(|| format!("window {name}: the {label} element is missing"))?;
        expect_semantics(element, label, false, &[])?;
    }
    verify_rows(snapshot, model, name)?;
    let (first, _, total) = model.effective_row_window();
    let bands = snapshot.row_bands();
    if bands.len() != 1 {
        return Err(format!(
            "window {name}: expected one row band, snapshot has {}",
            bands.len()
        ));
    }
    let band = bands[0];
    if band.first_row() != first || band.total_rows() != total || band.row_height() != px(0.0) {
        return Err(format!(
            "window {name}: row band: model first={first} total={total} height={:?}, \
             snapshot first={} total={} height={:?}",
            px(0.0),
            band.first_row(),
            band.total_rows(),
            band.row_height()
        ));
    }
    let expected_focus = model.window(window).focus.map(FocusTarget::label);
    let actual_focus = snapshot.focus().map(|id| {
        snapshot
            .node(id)
            .and_then(SnapshotElement::label)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("node-{}", id.slot()))
    });
    if actual_focus.as_deref() != expected_focus {
        return Err(format!(
            "window {name}: focus: model {expected_focus:?}, snapshot {actual_focus:?}"
        ));
    }
    Ok(())
}

fn verify_rows(snapshot: &PresentationSnapshot, model: &Model, window: &str) -> Result<(), String> {
    let expected: Vec<&str> = model
        .mounted_rows()
        .into_iter()
        .map(|row| row.title.as_str())
        .collect();
    let mut actual: Vec<String> = Vec::new();
    for element in snapshot.elements() {
        let text = element.text();
        if !text.starts_with("row ") {
            continue;
        }
        if !expected.contains(&text.as_str()) {
            return Err(format!(
                "window {window}: unexpected row title {text:?}; expected {expected:?}"
            ));
        }
        actual.push(text);
    }
    if actual.len() != expected.len()
        || actual
            .iter()
            .map(String::as_str)
            .ne(expected.iter().copied())
    {
        return Err(format!(
            "window {window}: row order/text: model {expected:?}, snapshot {actual:?}"
        ));
    }
    for title in &expected {
        let done = snapshot
            .find_label(&format!("done {title}"))
            .ok_or_else(|| format!("window {window}: row {title:?} has no complete checkbox"))?;
        expect_semantics(
            done,
            &format!("done {title}"),
            true,
            &[SnapshotAction::Change],
        )?;
        let remove = snapshot
            .find_label(&format!("remove {title}"))
            .ok_or_else(|| format!("window {window}: row {title:?} has no remove button"))?;
        expect_semantics(
            remove,
            &format!("remove {title}"),
            true,
            &[SnapshotAction::Click],
        )?;
        let text = snapshot
            .elements()
            .find(|element| element.tag() == "text" && element.text() == *title)
            .ok_or_else(|| format!("window {window}: row {title:?} has no title text"))?;
        expect_semantics(text, title, false, &[])?;
    }
    Ok(())
}

/// 断言元素的固有可聚焦性与已挂载动作集合；参考快照由同一段投影代码生成，
/// 不能发现系统性的语义错误，因此模型要独立声明它的期望。
fn expect_semantics(
    element: &SnapshotElement,
    name: &str,
    focusable: bool,
    actions: &[SnapshotAction],
) -> Result<(), String> {
    let semantics = element.semantics();
    if semantics.focusable() != focusable {
        return Err(format!(
            "element {name:?}: focusable={}, expected {focusable}",
            semantics.focusable()
        ));
    }
    for action in [
        SnapshotAction::Click,
        SnapshotAction::Change,
        SnapshotAction::Input,
    ] {
        let expected = actions.contains(&action);
        if semantics.has_action(action) != expected {
            return Err(format!(
                "element {name:?}: action {action:?} mounted={}, expected {expected}",
                semantics.has_action(action)
            ));
        }
    }
    Ok(())
}

fn element_text(snapshot: &PresentationSnapshot, label: &str) -> Result<String, String> {
    snapshot
        .find_label(label)
        .map(SnapshotElement::text)
        .ok_or_else(|| format!("missing element labelled {label}"))
}

/// 一行里三个承载身份的节点：标题文本、完成复选框与删除按钮。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RowKeys {
    title: PresentationKey,
    done: PresentationKey,
    remove: PresentationKey,
}

/// 每个窗口连续可见的行的呈现身份；行消失后重新出现不要求复用旧身份。
#[derive(Default)]
struct RowIdentity {
    per_window: [BTreeMap<String, RowKeys>; WINDOW_COUNT],
}

impl RowIdentity {
    fn update(&mut self, window: usize, snapshot: &PresentationSnapshot) -> Result<(), String> {
        let mut titles: BTreeMap<String, PresentationKey> = BTreeMap::new();
        let mut done: BTreeMap<String, PresentationKey> = BTreeMap::new();
        let mut remove: BTreeMap<String, PresentationKey> = BTreeMap::new();
        for element in snapshot.elements() {
            if let Some(label) = element.label() {
                if let Some(title) = label.strip_prefix("done ") {
                    done.insert(title.to_owned(), element.key());
                } else if let Some(title) = label.strip_prefix("remove ") {
                    remove.insert(title.to_owned(), element.key());
                }
            }
            let text = element.text();
            if text.starts_with("row ") {
                titles.insert(text, element.key());
            }
        }
        let current: BTreeMap<String, RowKeys> = titles
            .into_iter()
            .filter_map(|(title, title_key)| {
                let keys = RowKeys {
                    title: title_key,
                    done: *done.get(&title)?,
                    remove: *remove.get(&title)?,
                };
                Some((title, keys))
            })
            .collect();
        for (title, keys) in &current {
            if let Some(previous) = self.per_window[window].get(title)
                && previous != keys
            {
                return Err(format!(
                    "window {}: row {title:?} changed presentation identity across \
                     consecutive turns ({previous:?} -> {keys:?})",
                    WINDOW_KEYS[window]
                ));
            }
        }
        self.per_window[window] = current;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 每个窗口的树状态与有效变化。
// ---------------------------------------------------------------------------

struct TreeState {
    slot: TreeSlot,
    window: Option<usize>,
    visible: bool,
    revision: u64,
    observed: Observed,
    snapshot: PresentationSnapshot,
}

fn window_index(slot: TreeSlot) -> Option<usize> {
    match slot {
        TreeSlot::Window(key) => WINDOW_KEYS.iter().position(|candidate| *candidate == key),
        TreeSlot::Root => None,
    }
}

fn slot_name(slot: TreeSlot) -> String {
    match slot {
        TreeSlot::Root => String::from("root"),
        TreeSlot::Window(key) => format!("window {key}"),
    }
}

fn observe(app: &Runtime) -> Vec<TreeState> {
    app.state
        .borrow()
        .oracle_tree_snapshots()
        .into_iter()
        .map(|(slot, visible, snapshot)| TreeState {
            slot,
            window: window_index(slot),
            visible,
            revision: snapshot.revision(),
            observed: observed(&snapshot),
            snapshot,
        })
        .collect()
}

/// 树集合必须恰好是目录里的两个窗口；多出的 Root 或未知树不能被静默跳过。
fn verify_tree_set(trees: &[TreeState]) -> Result<(), String> {
    for tree in trees {
        if tree.window.is_none() {
            return Err(format!(
                "unexpected mounted tree {} outside the window catalog",
                slot_name(tree.slot)
            ));
        }
    }
    for window in 0..WINDOW_COUNT {
        tree_for(trees, window)?;
    }
    if trees.len() != WINDOW_COUNT {
        return Err(format!(
            "expected {WINDOW_COUNT} mounted trees, observed {}",
            trees.len()
        ));
    }
    Ok(())
}

/// 运行时全局的输入焦点窗口；它不是任何一棵树的快照字段。
fn verify_focused_window(app: &Runtime, model: &Model) -> Result<(), String> {
    let actual = app.focused_window().and_then(|window| {
        WINDOW_KEYS
            .iter()
            .position(|candidate| *candidate == window.key())
    });
    if actual != Some(model.focused_window) {
        return Err(format!(
            "focused window: model {:?}, runtime {:?}",
            WINDOW_KEYS.get(model.focused_window),
            actual.map(|index| WINDOW_KEYS[index])
        ));
    }
    Ok(())
}

fn tree_for(trees: &[TreeState], window: usize) -> Result<&TreeState, String> {
    trees
        .iter()
        .find(|tree| tree.window == Some(window))
        .ok_or_else(|| format!("window {} has no mounted tree", WINDOW_KEYS[window]))
}

/// 比较一步操作前后的每棵树：可观察变化、可见性翻转与 revision 必须一致。
fn verify_transition(
    before: &Model,
    after: &Model,
    previous: &[TreeState],
    current: &[TreeState],
) -> Result<(), String> {
    let app_changed = app_content_changed(before, after);
    for (window, name) in WINDOW_KEYS.iter().copied().enumerate() {
        let old = tree_for(previous, window)?;
        let new = tree_for(current, window)?;
        let projection_changed = old.observed != new.observed;
        let visibility_flipped = old.visible != new.visible;
        let revision_advanced = old.revision != new.revision;
        let expected_projection = app_changed || window_content_changed(before, after, window);
        let expected_visibility = before.window(window).visible != after.window(window).visible;
        if projection_changed != expected_projection {
            return Err(format!(
                "window {name}: effective change: reference state changed={expected_projection}, \
                 normalized snapshot changed={projection_changed}; diff: {}",
                observed_diff(&old.observed, &new.observed)
            ));
        }
        if visibility_flipped != expected_visibility {
            return Err(format!(
                "window {name}: visibility changed={visibility_flipped}, \
                 reference state changed={expected_visibility}"
            ));
        }
        if revision_advanced != (projection_changed || visibility_flipped) {
            return Err(format!(
                "window {name}: tree revision advanced={revision_advanced}, \
                 snapshot changed={projection_changed}, visible={visible}; diff: {}",
                observed_diff(&old.observed, &new.observed),
                visible = new.visible
            ));
        }
    }
    Ok(())
}

/// 全量验证：绑定/memo、参考快照、参考状态与行身份。
/// 一次有效变化断言的路径级 diff：优先指出第一个不同的可观察字段。
fn observed_diff(old: &Observed, new: &Observed) -> String {
    if old.appearance != new.appearance {
        return format!("appearance {} -> {}", old.appearance, new.appearance);
    }
    if old.focus != new.focus {
        return format!("focus {:?} -> {:?}", old.focus, new.focus);
    }
    if old.row_bands != new.row_bands {
        return format!("row bands {:?} -> {:?}", old.row_bands, new.row_bands);
    }
    projection_diff("root", &old.roots, &new.roots)
        .unwrap_or_else(|| String::from("observable structures are identical"))
}

fn verify_all(
    app: &Runtime,
    model: &Model,
    trees: &[TreeState],
    identity: &mut RowIdentity,
) -> Result<(), String> {
    verify_tree_set(trees)?;
    verify_focused_window(app, model)?;
    verify_state(app)?;
    let reference = reference_snapshot(model)?;
    let reference_projection = project(&reference, false);
    for (window, name) in WINDOW_KEYS.iter().copied().enumerate() {
        let tree = tree_for(trees, window)?;
        verify_projection(&tree.snapshot, &reference_projection, name)?;
        verify_window_state(&tree.snapshot, model, window)?;
        identity.update(window, &tree.snapshot)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 随机序列。
// ---------------------------------------------------------------------------

fn run_sequence(seed: u64, steps: usize) -> Result<(), String> {
    let mut rng = Rng::new(seed);
    let mut model = Model::default();
    let (mut app, windows) = mount_incremental();
    let mut identity = RowIdentity::default();
    let mut ops: Vec<String> = Vec::new();
    let mut trees = observe(&app);
    // 初始挂载也必须是参考状态的固定点。
    match catch_panic(|| verify_all(&app, &model, &trees, &mut identity)) {
        Ok(Ok(())) => {}
        Ok(Err(detail)) => return Err(failure(seed, &ops, &detail)),
        Err(panic) => {
            return Err(failure(seed, &ops, &format!("verification panic: {panic}")));
        }
    }
    for step in 0..steps {
        let op = random_op(&mut rng, &model);
        let before = model.clone();
        let mut after = before.clone();
        apply_model(&mut after, &op);
        ops.push(format!("{step}: {}", op.describe()));
        // runtime 内部 panic 也带上种子与操作序列；发现崩溃后不再继续使用
        // 这个可能半提交的运行时的状态。
        match catch_panic(|| apply_op(&mut app, &windows, &before, &after, &op)) {
            Ok(Ok(())) => {}
            Ok(Err(detail)) => return Err(failure(seed, &ops, &detail)),
            Err(panic) => {
                return Err(failure(seed, &ops, &format!("runtime panic: {panic}")));
            }
        }
        let verified = catch_panic(|| -> Result<Vec<TreeState>, String> {
            let current = observe(&app);
            verify_transition(&before, &after, &trees, &current)?;
            verify_all(&app, &after, &current, &mut identity)?;
            Ok(current)
        });
        let current = match verified {
            Ok(Ok(current)) => current,
            Ok(Err(detail)) => return Err(failure(seed, &ops, &detail)),
            Err(panic) => {
                return Err(failure(seed, &ops, &format!("verification panic: {panic}")));
            }
        };
        model = after;
        trees = current;
    }
    Ok(())
}

fn failure(seed: u64, ops: &[String], detail: &str) -> String {
    format!(
        "differential oracle failed\nseed = {seed:#x}\noperations:\n{}\ndetail:\n{detail}",
        ops.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn randomized_sequences_match_the_independent_reference() {
        for seed in 0..32u64 {
            if let Err(detail) = run_sequence(seed, 48) {
                panic!("\n{detail}");
            }
        }
    }

    #[test]
    #[ignore = "long randomized sweep; run locally before release"]
    fn long_randomized_sweep() {
        for seed in 0..256u64 {
            if let Err(detail) = run_sequence(seed, 96) {
                panic!("\n{detail}");
            }
        }
    }

    #[test]
    fn hidden_windows_keep_their_snapshots_and_revisions_in_the_oracle() {
        let (mut app, windows) = mount_incremental();
        let before = observe(&app);
        app.hide(windows[1]).expect("hide the side window");
        let node = node_by_label(&app, windows[0], "inc").expect("inc in main");
        app.click_node(node).expect("increment from main");
        let after = observe(&app);
        for (window, name) in WINDOW_KEYS.iter().copied().enumerate() {
            let old = tree_for(&before, window).expect("before tree");
            let new = tree_for(&after, window).expect("after tree");
            assert!(
                new.revision > old.revision,
                "window {name} flushes the shared write while hidden or visible"
            );
            let mut model = Model {
                counter: 1,
                // 隐藏原来的焦点窗口后，输入焦点回落到 main。
                focused_window: 0,
                ..Model::default()
            };
            model.windows[1].visible = false;
            verify_window_state(&new.snapshot, &model, window).expect("the hidden tree is current");
        }
    }

    #[test]
    fn invisible_store_writes_do_not_advance_revisions() {
        let (mut app, windows) = mount_incremental();
        let done = node_by_label(&app, windows[0], "filter-done").expect("the Done filter");
        app.click_node(done).expect("switch to the Done filter");
        let before = observe(&app);

        let add = node_by_label(&app, windows[0], "add").expect("the Add button");
        app.click_node(add).expect("add an active row");
        let after = observe(&app);
        for window in 0..WINDOW_COUNT {
            let old = tree_for(&before, window).expect("before tree");
            let new = tree_for(&after, window).expect("after tree");
            assert_eq!(
                old.observed, new.observed,
                "an active row added under Done changes no window"
            );
            assert_eq!(
                old.revision, new.revision,
                "an invisible store write never advances a revision"
            );
        }

        let mut model = Model {
            filter: Filter::Completed,
            next_row: 1,
            ..Model::default()
        };
        model.rows.push(RowData {
            id: 0,
            title: String::from("row 0"),
            completed: false,
        });
        for window in 0..WINDOW_COUNT {
            let tree = tree_for(&after, window).expect("after tree");
            verify_window_state(&tree.snapshot, &model, window)
                .expect("the filtered row is invisible but committed");
        }
    }

    #[test]
    fn a_row_band_total_change_without_node_changes_is_effective() {
        let (mut app, windows) = mount_incremental();
        for _ in 0..4 {
            let add = node_by_label(&app, windows[0], "add").expect("the Add button");
            app.click_node(add).expect("add a row");
        }
        for window in windows {
            let band = band_key(&app, window).expect("the windowed For");
            app.set_row_window(window, band, 0, 2)
                .expect("mount only two rows");
        }
        let before = observe(&app);
        let mut before_model = Model {
            next_row: 4,
            row_first: 0,
            row_count: 2,
            ..Model::default()
        };
        before_model.rows = (0..4)
            .map(|id| RowData {
                id,
                title: format!("row {id}"),
                completed: false,
            })
            .collect();
        for window in 0..WINDOW_COUNT {
            let tree = tree_for(&before, window).expect("tree");
            verify_window_state(&tree.snapshot, &before_model, window)
                .expect("the initial window is correct");
        }

        let add = node_by_label(&app, windows[0], "add").expect("the Add button");
        app.click_node(add)
            .expect("append a row outside the window");
        let after = observe(&app);
        let mut after_model = before_model.clone();
        after_model.next_row = 5;
        after_model.rows.push(RowData {
            id: 4,
            title: String::from("row 4"),
            completed: false,
        });
        verify_transition(&before_model, &after_model, &before, &after)
            .expect("a row-band total change is an effective change");
        for window in 0..WINDOW_COUNT {
            let old = tree_for(&before, window).expect("before tree");
            let new = tree_for(&after, window).expect("after tree");
            assert_eq!(
                old.observed.roots, new.observed.roots,
                "the zero-height band keeps every node identical"
            );
            assert_ne!(
                old.observed.row_bands, new.observed.row_bands,
                "only the row-band metadata changes"
            );
            assert!(new.revision > old.revision);
            verify_window_state(&new.snapshot, &after_model, window)
                .expect("the new total is committed");
        }
        let mut identity = RowIdentity::default();
        verify_all(&app, &after_model, &after, &mut identity)
            .expect("reference, bindings and memos agree");
    }

    #[test]
    fn reset_clears_both_window_editors_with_the_draft() {
        let (mut app, windows) = mount_incremental();
        let draft = node_by_label(&app, windows[0], "draft").expect("draft in main");
        app.replace_text_node(draft, "abc")
            .expect("type into the main window");
        let side_draft = node_by_label(&app, windows[1], "draft").expect("draft in side");
        app.set_selection_node(side_draft, 1, 3)
            .expect("select inside the side window");
        let reset = node_by_label(&app, windows[1], "reset").expect("reset in side");
        app.click_node(reset).expect("reset the shared state");

        let trees = observe(&app);
        let model = Model::default();
        for window in 0..WINDOW_COUNT {
            let tree = tree_for(&trees, window).expect("tree");
            verify_window_state(&tree.snapshot, &model, window)
                .expect("the reset draft clears both editors");
        }
    }

    #[component]
    fn PoisonedText() -> MountResult<MountPlan> {
        let flag = signal(false);
        let poisoned: Memo<String> = memo(move || {
            if flag.get() {
                panic!("oracle poison probe");
            }
            String::from("ok")
        });
        view! {
            <view>
                <text aria:label="poisoned">{poisoned}</text>
                <button aria:label="break" on:click={move || flag.set(true)}>"Break"</button>
            </view>
        }
    }

    #[test]
    fn a_poisoned_live_memo_keeps_its_last_value_under_the_oracle() {
        let mut app = Runtime::new();
        app.mount(|| view! { <PoisonedText /> })
            .expect("mount the poisoned fixture");
        app.click_label("break").expect("poison the memo");
        verify_state(&app).expect("a poisoned memo retains its last committed value");
    }

    #[test]
    fn the_oracle_detects_a_tampered_binding() {
        let (mut app, _windows) = mount_incremental();
        app.click_label("inc").expect("increment");
        {
            let mut state = app.state.borrow_mut();
            let tree = *state.oracle_trees().first().expect("one tree");
            let reference = {
                let table = state.oracle_binding_table(tree).expect("binding table");
                table
                    .oracle_entries::<String>()
                    .into_iter()
                    .find(|(_, source, current, _)| !source.is_static() && current.is_some())
                    .map(|(reference, _, _, _)| reference)
                    .expect("a dynamic text binding")
            };
            let table = state.oracle_binding_table_mut(tree).expect("binding table");
            table
                .slot_mut::<String>(reference)
                .expect("live text slot")
                .current = Some(String::from("tampered"));
        }
        let error = verify_state(&app).expect_err("the oracle must catch the tampering");
        assert!(
            error.contains("tampered") || error.contains("fresh value"),
            "unexpected oracle error: {error}"
        );
    }

    #[test]
    fn the_oracle_detects_an_unsettled_live_memo() {
        let (mut app, _windows) = mount_incremental();
        app.click_label("inc").expect("increment");
        let live = {
            let state = app.state.borrow();
            state.live_memo_set(&BTreeSet::new())
        };
        let id = *live.iter().next().expect("a live memo");
        app.state.borrow_mut().memos[id as usize].dirty = true;
        let error = verify_state(&app).expect_err("the oracle must catch the dirty memo");
        assert!(
            error.contains("still dirty"),
            "unexpected oracle error: {error}"
        );
    }

    #[test]
    fn the_oracle_detects_a_caret_mismatch() {
        let (mut app, windows) = mount_incremental();
        let draft = node_by_label(&app, windows[0], "draft").expect("the draft input");
        app.set_caret_node(draft, 0).expect("place the caret");
        let trees = observe(&app);
        let error = verify_window_state(&trees[0].snapshot, &Model::default(), 0)
            .expect_err("the oracle must compare caret/selection");
        assert!(
            error.contains("draft edit"),
            "unexpected oracle error: {error}"
        );
    }

    #[test]
    fn the_oracle_detects_a_focus_mismatch() {
        let (mut app, windows) = mount_incremental();
        let inc = node_by_label(&app, windows[0], "inc").expect("the increment button");
        app.focus_node(inc).expect("focus the button");
        let trees = observe(&app);
        let error = verify_window_state(&trees[0].snapshot, &Model::default(), 0)
            .expect_err("the oracle must compare focus per window");
        assert!(error.contains("focus:"), "unexpected oracle error: {error}");
    }

    #[test]
    fn the_oracle_detects_a_snapshot_mismatch() {
        let (mut app, _windows) = mount_incremental();
        app.click_label("inc").expect("increment");
        let trees = observe(&app);
        let tree = tree_for(&trees, 0).expect("main tree");
        let reference = reference_snapshot(&Model::default()).expect("reference snapshot");
        let error = verify_projection(&tree.snapshot, &project(&reference, false), "main")
            .expect_err("the oracle must catch a stale reference state");
        assert!(
            error.contains("normalized snapshot differs"),
            "unexpected oracle error: {error}"
        );
    }

    #[test]
    fn oracle_failures_carry_the_seed_and_operations() {
        let message = failure(
            0x1234_5678,
            &[String::from("0: Inc"), String::from("1: ToggleShow")],
            "normalized snapshot differs",
        );
        assert!(message.contains("0x12345678"), "seed must be reproducible");
        assert!(message.contains("0: Inc"));
        assert!(message.contains("1: ToggleShow"));
        assert!(message.contains("normalized snapshot differs"));
    }
}
