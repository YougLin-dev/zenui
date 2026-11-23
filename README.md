# ZenUI

ZenUI 使用 Rust 编写原生桌面界面。它保留控件及其编辑状态，通过响应式更新修改受影响的部分，而不是每次数据变化都重建整棵界面。

你可以用 Rust 管理模型和派生逻辑，用 `view!` 声明结构，用静态 class 书写样式。ZenUI 不嵌入浏览器，也不依赖 JavaScript、HTML DOM 或通用 CSS 引擎。2D 是主干，3D 作为布局中的可选内容接入。

## 当前能力

ZenUI 处在开发阶段。阶段一的 target API、阶段二的 Windows 原生切片，以及阶段三的多窗口、关闭确认、原生打开/保存/文件夹对话框、整页滚动、焦点显露、覆盖层滚动条与窗口化 `For` 已可运行；菜单/托盘、辅助技术和跨平台支持仍在开发中。跨显示器混合 DPI 与实机 device loss 仍是 #63 的开放证据缺口。

仓库目前提供：

- headless Runtime：模型、行集、代际句柄、Owner、Memo 和事务；
- 阶段一 target API：Copy Signal/Memo/Store Field、mount-once 组件、`view!` 数据位、`Show`/`For` 与共享 RowSet；
- headless 与原生窗口的 activation 和任务交付边界：app/window 所有权、Send worker、代际检查后的 UI continuation；窗口关闭取消其交付，共享模型仍可由 app 所有权任务提交；
- Windows 宿主：每个存活窗口一个 HWND、Scene 与 DX12 surface，共享 device 和模型；原生输入、IME、DPI 与隐藏窗口 smoke（CI 回退 WARP）；
- 多窗口窗口目录：show/hide/close/reopen、关闭确认 epoch、父窗口输入挂起与声明的确认窗口；
- 原生打开/保存/文件夹对话框：窗口锚定、单选/多选、标题与类型过滤器、建议文件名与默认扩展名、取消/失败结果，以及显式的窗口或应用任务所有权；
- 宿主任务：文件对话框结果与 worker 共用 Task/continuation 交付，但没有 worker 通道，也不会阻塞 `settle_tasks`；
- 长列表场景构建：跨重建复用未变化的文本 layout 与测量结果，viewport 外的元素保留几何但跳过 glyph 与绘制；声明 `row-height` 的窗口化 `For` 只挂载可见行窗口，完整行范围由上下 spacer 保持（[ADR-0011](docs/adr/0011-windowed-for-mounting.md)）；变量行高仍不在本里程碑；
- 有界运行时存储：最近发布快照按固定窗口保留，释放的 Owner 节点回到存活规模，Todo 的列表 reconciliation 与行句柄查找使用索引而不是逐行扫描；
- 可复现的 Todo 场景：插入、删除、筛选、重排、焦点与拒绝句柄后的恢复；
- 独立的宏、文本、布局和 GPU 原型；
- 目标界面语法的预览。

你可以运行 headless Todo、target API 的最小 Todo，或 Windows 原生多窗口 Todo：

```sh
cargo run -p zenui-app --example todolist_headless --locked
cargo run -p zenui --example minimal_todo --locked
cargo run -p zenui-windows --example todo_host --locked
```

原生示例的主窗口工具栏可以打开共享模型的详情窗口；隐藏会保留详情窗口的本地编辑状态，关闭后可以重新打开。`Import file` 打开锚定到主窗口的原生打开文件对话框，把每个选中 UTF-8 文本文件的非空行提交为任务，并在主窗口关闭后仍由应用所有权任务完成导入。主窗口有非空草稿时，关闭按钮会显示 Continue editing / Discard draft 确认窗口。

target API 的公开行为测试位于根包 `tests/`，其中 `tests/tasks.rs` 覆盖窗口关闭、重开代际、过期交付与 app 所有权模型提交，`tests/ime.rs` 覆盖 headless composition 的 preedit、候选确认与取消；这些测试不代替原生窗口或真实 IME 验收，原生路径由 `crates/windows` 的隐藏 smoke 与可见窗口验收覆盖。

根工程的二进制只保留最小入口；原生窗口由 `zenui-windows` 的 `todo_host` 示例打开。目标语法的预览文件不参与默认 workspace 构建，因为只有部分所需 API 已经存在。

## 阅读路径

### 产品与路线

- [愿景](docs/产品/愿景.md)：产品定位、原则与边界。
- [路线图](docs/产品/路线图.md)：以 Windows 为首个完整产品平台的实现顺序。

### SDK

- [快速开始](docs/SDK/快速开始.md)：工具链、运行方式、工作区结构。
- [界面与组件](docs/SDK/界面与组件.md)：组件、属性、条件、列表和 `view!`。
- [数据与更新](docs/SDK/数据与更新.md)：Signal、Memo、Store、Field 和异步边界。
- [窗口与任务](docs/SDK/窗口与任务.md)：窗口实例、关闭确认、对话框和任务归属。
- [样式与输入](docs/SDK/样式与输入.md)：静态 class、typed style、文本编辑和 IME。

### 实现

- [技术架构](docs/实现/技术架构.md)：目标技术模型、研究借鉴与架构边界。
- [渲染与平台](docs/实现/渲染与平台.md)：布局、文本、绘制、内容接入和宿主能力。
- [验证重点](docs/实现/验证重点.md)：目标模型落地时需要验证的关键语义。
- [ADR-0007](docs/adr/0007-window-catalog-and-close-epoch.md)：窗口目录、per-activation 权威与关闭 epoch。
- [ADR-0008](docs/adr/0008-file-picker-host-tasks.md)：打开文件选择器、宿主任务、显式所有权与 STA 模态对话框。
- [ADR-0009](docs/adr/0009-file-dialog-family.md)：打开/保存/文件夹对话框家族、kind 矩阵与单路径校验。
- [ADR-0010](docs/adr/0010-scroll-is-presentation-state.md)：滚动是宿主拥有的整页呈现偏移，几何、命中与 IME 共享同一份已平移场景。
- [ADR-0011](docs/adr/0011-windowed-for-mounting.md)：窗口化 `For` 只挂载可见行窗口，spacer 保持完整行范围，宿主是窗口换算方。
- [Stage-one 验收](docs/实现/stage-one-验收.md)：headless Todo 的可执行完成判据、边界证据与原生缺口。
- [Stage-two 验收](docs/实现/stage-two-验收.md)：Windows 原生切片、IME、样式与 150% 实机证据。
- [Stage-three 多窗口验收](docs/实现/stage-three-多窗口-验收.md)：主窗口与详情窗口、hide/show、close/reopen 与关闭确认在 100%/150% 下的可见证据。
- [Stage-three 文件对话框验收](docs/实现/stage-three-文件对话框-验收.md)：原生 `IFileOpenDialog`、取消、单选/多选、导入、错误与 Unicode 路径的可见证据。
- [Stage-three 保存与文件夹验收](docs/实现/stage-three-保存与文件夹-验收.md)：原生 `IFileSaveDialog`、覆盖确认、过滤扩展名优先级、写入拒绝、Unicode 路径与文件夹选择的可见证据。
- [Stage-three 宿主滚动验收](docs/实现/stage-three-宿主滚动-验收.md)：真实滚轮与翻页键、边界夹取、hover/press 复核、聚焦编辑器与缩放在 100%/150% 下的可见证据。
- [Stage-three 焦点显露验收](docs/实现/stage-three-焦点显露-验收.md)：Tab/编辑后的焦点与 caret 显露、滚动不移动焦点、组合中滚动与候选窗锚点在 100%/150% 下的可见证据。
- [Stage-three 滚动条验收](docs/实现/stage-three-滚动条-验收.md)：覆盖层滚动条的 thumb 拖动、轨道翻页与键鼠共享偏移在 100%/150% 下的可见证据。
- [Stage-three 窗口化 For 验收](docs/实现/stage-three-窗口化-for-验收.md)：10,000 行窗口化列表的滚动、编辑、重排与过滤在 100%/150% 下的可见证据。
- [Stage-three 滚动总验收](docs/实现/stage-three-滚动总验收.md)：滚轮单位与精度累积、边界、翻页、显露、IME 锚点、滚动条、hide/show、close/reopen 与窗口化列表的汇总证据与边界。

## 界面语法

`view!` 描述结构，普通 Rust 代码负责逻辑。数据位传递句柄或明确的一次性值，不使用任意零参数闭包充当持续绑定。

```rust
let remaining = signal(3);
let remaining_txt = memo(move || format!("{} remaining", remaining.get()));

view! {
    <text class="text-sm text-muted">{remaining_txt}</text>
}
```

`once(value)` 表示在挂载时使用一次，不建立后续订阅。窗口、菜单、托盘和系统对话框由独立的平台 API 管理，不进入 `view!`。

## 开发环境

使用仓库固定的 Rust 工具链：

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run --release -p zenui-app --example reconcile_bench --locked
```

`reconcile_bench` 是固定负载的时间探针：扫描 500/1000/2000/4000 行，分别测量未变化列表 reconciliation 与空提交，并打印发布窗口和 Owner 存储规模。它只提供测量，不作为 wall-clock 断言。

场景构建的同类探针：

```sh
cargo run --release -p zenui-render --example scene_bench --locked
```

它导入 100/1,000/10,000 行，在 32 行窗口下分别测量无缓存构建、缓存冷启动、稳定缓存重建、输入后的重建、跨行窗口更新，以及窗口内的滚动构建；只打印测量、提交元素数与可见行窗口，不作绝对时间断言。

独立的原型工作区使用各自的 `Cargo.toml`。宏原型可以这样运行：

```sh
cargo test --manifest-path examples/macro_poc/Cargo.toml --workspace --locked
```

开发约定见 [CONTRIBUTING.md](CONTRIBUTING.md)。
