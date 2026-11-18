# 多领域评审的执行记录与证据范围

日期：2026-11-18。工作区为 `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui`。实际执行环境是 Windows 11、MSVC Rust 工具链、headless wgpu/Vulkan。下文将**实际执行、源码事实、目标设计和未来验证**分开；agent 的评审意见本身不算运行证据。

## 1. 评审过程与其证据地位

1. **第一轮**：A1/A2/A3、E1/E2/E3、U1/U2/U3、X1/X2、S 共 12 个独立视角。首轮有 10 位正常返回结构化报告；A1 发生结构化输出调用失败，E2 没有正常结构化收尾。之后分别完成独立补充审查，初次失败没有被改写成最初成功。综合记录见 `round1-experts.md`，E2 的实际布局/文字专项见 `text-layout-review.md`。
2. **第二轮**：A2+E1+X2 审查状态/语法，A1+E2+S 审查更新/渲染/样式，A3+E3+U 类审查真实桌面体验，另设主控反方。见 `round2-debate.md`；候选 D1–D12 不自动成为最终规则。
3. **样式源码补充核验**：独立样式审查读取 Tailwind `variants.ts` 固定 commit `ac7fd4007178123269cc6c8e03c8d957646331fa` 和 CSS Cascade。确认 `aria-pressed` 生成 `[aria-pressed="true"]`，简单语法没有对应的 false variant。普通 author style attribute 优先于普通 class/状态声明是 CSS origin 规则，不是 Tailwind 特有 specificity；important/layer 的精确反转仍要求固定版本差分。
4. **第三轮**：E1/X2、U1/U2、E3+A3+U3、S+A2 分别阅读 FINAL/Todo 后提交 Rust、初学者、桌面可访问性与红队复审，见 `round3-review.md`。随后目标示例已修正 Copy+String、Result mount、field/query 接线、列表角色、TextEdit origin、错误写入等建议。
5. **第四轮验证规划**：从已完成的工作流 journal 恢复契约矩阵到 `verification-contract.json`。它是**历史规划及验证 fixture 计划**，不是新建 crate 或运行测试。顶层 `status.supersededRules` 和修正后的 StyleIR fixture 明确指出：旧提议“hover 激活后胜过 ordinary inline”已经被 FINAL §6 取代，hover=false 与 true 均应保留普通 inline 的胜出。
6. **最终定点校对与复跑**：统一为 `MountResult<MountPlan>` 的目标函数形状，消除事件 setter 错误被悄悄忽略的问题，清理 Gate A0–D 的验收文字，再执行下列仓库探针和临时安全 Rust 夹具。没有实现整个框架，也没有建立统一 workspace。

## 2. 本轮最终实际执行的仓库测试

以下命令均在仓库根目录执行，结果为**本轮重新运行**，不是转述历史记录。

```sh
cargo test --manifest-path examples/desktop_poc/Cargo.toml --lib
# exit 0；7 passed：
# 局部 sink、source 切换的 edge 清理、batch 基础去重、dispose，
# 以及少量 typed utility/class/variant/未知 token 定位测试。

cargo test --manifest-path examples/macro_poc/Cargo.toml --workspace
# exit 0；2 integration tests + 1 trybuild test runner。
# trybuild runner 内 3 compile-fail cases：html_element、script_event、unknown_class。
# 其余 library unit/doc test target 为 0 tests，不能把这些 0 当完成证明。

cargo test --manifest-path examples/native_poc/Cargo.toml --lib
# exit 0；4 passed：
# ICU 复杂文字边界、CPU premultiply、Taffy+Parley 测量/改变文字失效、
# CJK shaping + CPU glyph pixels。

cargo test --manifest-path Cargo.toml
# exit 0；0 tests passed。只证明根 hello-world package 的测试构建。
```

因此，仓库当前有 **13 个实际通过测试，另加 macro trybuild runner 内的 3 个 compile-fail fixture**。宏 workspace 的一层 1-pass 是 runner，不用含糊的“17 个”冒充完整 UI 能力。这些测试都不证明生产 Memo/Store/For/UI/a11y 已实现。

## 3. 本轮最终实际执行的两条 GPU 路径

```sh
cargo run --manifest-path examples/native_poc/Cargo.toml
# exit 0；Adapter: NVIDIA GeForce RTX 4060 Laptop GPU / Vulkan。
# PASS: sparse-strips GPU, real Parley/Glifo CJK pixel runs,
# ordinary external texture binding.
# Transparent semi-red GPU pixel: [128,0,0,128]; CPU reference matches.

cargo run --manifest-path examples/desktop_poc/Cargo.toml
# exit 0；同机 NVIDIA/Vulkan。
# PASS: Vello underlay, foreign wgpu LoadOp::Load,
# straight-alpha overlay compositing.

```

源码/当前路径的区别必须保留：Classic 独立 composited overlay 使用 straight alpha；native sparse-strips 半透明红的 RGBA readback 是 premultiplied `[128,0,0,128]`。两者不能被推导为所有 Vello 版本/纹理的通用默认。

这两条命令仅证明本机 headless RGBA 图中的文字、纹理互操作和当次 readback。**不证明** 真实 winit 窗口、surface/resize/DPI、Mac/Metal、Linux X11/Wayland、device-loss 恢复、完整字体能力、生产 retained paint cache、性能、glTF、PBR 或 3D 深度。不同 clip/mask/filter 仍须显式能力，不能以 panic 检测能力。

## 4. 独立安全 Rust / rStml 夹具

下列位置均在 `C:\Users\Y\AppData\Local\Temp\` 下，是此前由本任务创建的临时验证代码，未纳入仓库的正式测试。它们只验证小形状，不验证生产实现。

### 4.1 `zenui-type-contract`

```sh
cargo run --manifest-path C:/Users/Y/AppData/Local/Temp/zenui-type-contract/Cargo.toml --example type_contract
# 最终 exit 0：
# planned title descriptor is owned once: 任务
# PASS: mount-body ? and Copy-handle closure ownership are valid plain Rust

cargo run --manifest-path C:/Users/Y/AppData/Local/Temp/zenui-type-contract/Cargo.toml
# 最终 exit 0：
# PASS parse-only: target node/component/attribute shapes fit rstml
# RED AS EXPECTED: nested batch leaks an intermediate value; A0 remains unfulfilled
```

`type_contract.rs` 验证普通 Rust `Result<MountPlan,MountError>` 函数能在 body 中写 `let title = title?;`，同时 Copy handle 可被 move closure 所有。它**不证明**未来 `#[component]` 或 `view!` 会保留这种行为，也不验证 Props lowering、Plan 创建/错误路径或 runtime commit；这些仍是 A1。

RSX 程序仅把目标 node/component/attribute 形状交给真实 `rstml`，验证可以 parse；这不是完整组件宏完成。嵌套 batch 则复用现有 PoC：outer 写 `a=true`，inner 写 `b=true`，outer 再写 `b=false`，实际组合出包含 `true` 的两条 patch，揭示现实现缺少嵌套事务深度。这个**反例仍保持红**，没有因为本夹具成功运行而宣称 A0 事务已完成。

此临时目录在最终复查时只有 `transaction_contract` 的 example 注册；已为其补回早已存在源文件 `type_contract.rs` 的 example 条目后重新执行。当前 manifest 状态不能替代先前历史输出，故本文明确记录此次复跑。

### 4.2 `zenui-final-contract` 与延迟命令形状

本轮首次复跑 `zenui-final-contract` **编译失败**：`WriteError` 无 `Debug` 导致 `unwrap()` 的 E0277，`dyn Command` 的按值调用导致 E0161。本文不把它称为先前通过。只对临时自写 fixture 做最小修正：为错误派生 `Debug`，把 command trait 定义为 `fn apply(self: Box<Self>, ...)`，然后执行：

```sh
cargo run --manifest-path C:/Users/Y/AppData/Local/Temp/zenui-final-contract/Cargo.toml
# 修正后的 exit 0，另有 Insert 未使用 String 字段的 1 条 warning：
# PASS: Result mount + Copy handle + deferred transaction API in safe Rust
# SCOPE: shape only, not production rollback, macro lowering, GPU or UI
```

`zenui-type-contract --example transaction_contract` 首次复跑也失败，因为泛型 `S` 没有 `Default` bound。补上 `AppPending: Default` 与 `impl<S: Default>` 后：

```text
PASS: command shape collects writes explicitly; actual staging/commit remains A0 work
```

这里的 `finish()` 仅在 scratch `Model::default()` 或 scratch state 上运行 command，并抽取第一个 Result 错误。它不模拟真正 current model、journal、stable topo 或 View/Taffy/focus checkpoint。**"deferred writes API 可以编译"不等于事务回滚已验证。**

### 4.3 `Copy+String` 与自动 example 发现

早期的独立 `zenui-copy-probe` 曾实际失败于 E0204：`#[derive(Copy)]` 的结构不可能持有一个 `String`。当前 TodoRow 改为 stable id + `CompletedField`，title 是 mount 时返回 Result 的 owned read。

在根 manifest 默认 auto-discover examples 时，cargo 会把设计稿 `examples/todolist/main.rs` 当二进制并发现不存在的 facade/API；现设为 `autoexamples=false`。正确的根构建/0-test 仅说明 design-only 文件不再被误当成可运行应用。新 crate/feature/example 应显式登记，而不是利用这个开关掩盖真正的产品编译错误。

## 5. 目标示例与当前实现的关系

`examples/todolist/main.rs` 是**无法直接编译的目标 API 预览**。它展示目标 screen structure、三个静态 filter buttons、共享 RowSet 的 For、Result mount、源自同一状态的可访问 toggle、独立 empty 状态、TextInput 来源策略及显式错误分支。`Signal::set` 失败进入 runtime diagnostic；业务命令失败进入可见 `error` Signal；错误 Signal 自身失败仍须处理，且不靠 panic 当业务错误机制。

其符号、props 和未来 launcher 不是根 crate 已实现的方法。`MountResult<MountPlan>` 的普通 Rust body 通过小类型夹具并不证明整段目标 RSX 可以宏展开或真正产生 Todo 窗口。编译与运行规划在 A0/A1，不能将 `cargo run --bin todolist --features windows-desktop --locked` 作为当前存在的命令。

## 6. 已有 native PoC 尚未修复的实际问题

E2 源码检查和实际文件表明：

- `examples/native_poc/src/layout.rs` 中 `Paragraph::measure()` 计算了 `first_baseline`，但回调只传 `.size`，Taffy leaf 输出的 `Baselines::NONE`。现有测量测试绿不等于完整 baseline 正确。
- known dimensions 与 available space 不能仅以一个 `Option<f32>` 代替；当前 `calculate_content_widths()` 过快早退及 `break_all_lines(Some(0))`/`None` 近似不能证明 min/max intrinsic。须分别测收缩、wrap、混合 bidi、字体和 DPI。
- `text.rs` 的 glyph draw 尚没有将 Parley run 的 Font synthesis 传给 embolden/skew 处理。普通 face 的假 bold/oblique 和相应 cache key 是 B Gate。
- GPU external binding 用固定绿色 clear target 的探针不代表 Alpha correctness 全场景、独立 depth、View/resource generation、mask/filter 或动态资源生命周期。绿色 clear 不等于加载真实模型。

这些局限不因 library unit tests 或 GPU readback 退出 0 而消失。`text-layout-review.md` 是此处的源码专项依据。

## 7. 未完成工作与验证门（全部未完成）

- **A0**：generational core/Owner、field/query 反向依赖、RowSet、稳定的 Memo、嵌套事务与 staging tree、EditReducer/FocusReducer 和可观察计数。另有 sink 空 bucket: 现 PoC `detach` 移除 observer id 却可能保留空 source key，`subscriber_count()` 只数 bucket 会误统计；新 core 必须删除空 bucket并把活跃 source/edge 与 mount/unmount 压力计数纳入测试。
- **A1**：单 Windows Todo、实际 MountPlan/widget/component/For/Show、正确事件/IME/错误 sink、typed macro lowering 与 renamed facade golden。现有宏写死 `::zenui_macro_runtime`；真实只 rename 不存在原包的构建尚未证明。Windows semantic dump 为间接验证，真实 NVDA 归 C。
- **B**：baseline/intrinsic/Font synthesis/bidi、StyleIR fixed-version N/D/R/X、重要性与 layer 差分、GeometrySnapshot 与 backend capabilities；每项需真实 fixture，未测为 N/A。
- **C**：同一 locked Todo 在 Windows/macOS/Linux，分 Wayland/X11、IME/NVDA/VoiceOver/AT-SPI、resize/DPI/device loss、assets/licenses/package。没有 runner 就是 blocked，不能复制结果。
- **D**：最小合法 glTF 的 mesh/depth 遮挡、独立 offscreen metadata、多 viewport、device/owner generation 与完整 resource lifecycle；当前绿色纹理和 foreign triangle 都不能算 glTF。

性能协议只使用固定 workload、commit、工具链、font/ICU artifact、OS/driver/硬件、warmup 和多次样本，报告 trace 与 p50/p95/p99，而不是填未测试的具体预算。Idle 要真窗 trace，资源回收要数 live graph/GPU resources，不要求 allocator RSS 机械归零。缺数据是 N/A/blocked，不是 0ms 或 green。

## 8. 当前文档的权威性与结束结论

`docs/review/FINAL.md` 是当前唯一推荐目标，本文件是当前执行记录。`docs/research/` 保留前期研究与被修订的候选（例如 `class:if-*`、旧 mount 形状或历史条件优先级），入口已加入 superseded-status 指引。

评审本身也不是正确性证明，引用源码也仅在具体指出的 API/版本范围内成立；之后的 Gate 必须以实际代码和可复现测试为准。**下一项是 A0 的最小纯 Rust 垂直闭环，而不是继续扩展设计、宣称全 Tailwind 或立即开发 3D。**
