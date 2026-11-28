# ZenUI

ZenUI 使用 Rust 编写原生桌面界面。它保留控件及其编辑状态，通过响应式更新修改受影响的部分，而不是每次数据变化都重建整棵界面。

你可以用 Rust 管理模型和派生逻辑，用 `view!` 声明结构，用静态 class 书写样式。ZenUI 不嵌入浏览器，也不依赖 JavaScript、HTML DOM 或通用 CSS 引擎。2D 是主干，3D 作为布局中的可选内容接入。

## 开始一个新应用

新应用从 [`templates/minimal-app`](templates/minimal-app/README.md) 开始。克隆仓库后，从仓库根运行：

```sh
cargo run --manifest-path templates/minimal-app/Cargo.toml --locked
```

窗口标题为 `ZenUI Starter`，`Increment` 按钮增加 `Count`，关闭窗口退出。模板是独立 workspace，拥有自己的 `Cargo.lock` 与 `rust-toolchain.toml`。[从空目录开始](docs/SDK/从空目录开始.md)说明如何把模板复制到仓库外、改用 git 依赖、准备 Windows 构建环境并生成可双击运行的 release exe；[快速开始](docs/SDK/快速开始.md)说明仓库结构、内部示例与开发检查。

ZenUI 仍在 1.0 之前，根包与模板都设置 `publish = false`，没有发布到 crates.io；依赖只能来自仓库路径或 git，API 与模板会随仓库变化。

## 当前能力

ZenUI 处在开发阶段。阶段一的 target API、阶段二的 Windows 原生切片，以及阶段三的多窗口、关闭确认、应用会话策略与显式退出、原生打开/保存/文件夹对话框、整页滚动、焦点显露、覆盖层滚动条、窗口化 `For`、只读 UIA 树、UIA Invoke/Toggle/Value/SetFocus 动作、UIA 焦点/状态/结构事件、UIA TextPattern 与窗口根 ScrollPattern、原生菜单栏/快捷键/窗口级上下文菜单、应用托盘与文件拖放已可运行；节点级上下文菜单与跨平台支持仍在开发中。跨显示器混合 DPI 仍是 #63 的开放证据缺口；实机 device loss 已在单屏可见窗口上尝试（Win+Ctrl+Shift+B 与同适配器 GPU 挂起），未观察到丢失，记录见 [设备丢失实机尝试](docs/实现/stage-three-设备丢失-验收.md)。

仓库目前提供：

- headless Runtime：模型、行集、代际句柄、Owner、Memo 和事务；
- 阶段一 target API：Copy Signal/Memo/Store Field、mount-once 组件、`view!` 数据位、`Show`/`For` 与共享 RowSet；
- headless 与原生窗口的 activation 和任务交付边界：app/window 所有权、Send worker、代际检查后的 UI continuation；窗口关闭取消其交付，共享模型仍可由 app 所有权任务提交；
- Windows 宿主：每个存活窗口一个 HWND、Scene 与 DX12 surface，共享 device 和模型；原生输入、IME、DPI 与隐藏窗口 smoke（CI 在没有硬件适配器时回退 WARP）；
- 多窗口窗口目录：show/hide/close/reopen、关闭确认 epoch、父窗口输入挂起与声明的确认窗口；
- 应用会话：启动前声明 `QuitOnLastClose`（默认）或 `StayResident`；`SessionCtx` 在命令处理器中排队恢复窗口与显式退出。常驻会话允许零窗口消息循环，原生宿主要求声明托盘，托盘恢复 primary 或已关闭窗口，Quit 关闭全部 activation 并释放待决关闭确认；
- 原生打开/保存/文件夹对话框：窗口锚定、单选/多选、标题与类型过滤器、建议文件名与默认扩展名、取消/失败结果，以及显式的窗口或应用任务所有权；
- 宿主任务：文件对话框结果与 worker 共用 Task/continuation 交付，但没有 worker 通道，也不会阻塞 `settle_tasks`；
- 文件拖放：每个可见且注册 `on_drop` 的窗口拥有 OLE `IDropTarget`，只接受启用窗口、COPY 与 `CF_HDROP`。路径按源顺序以 `DropEvent` 在 `Phase::Event` 交付，保留重复、Unicode、非 BMP 与长路径；隐藏、模态选择器、关闭确认与非文件数据在处理器前拒绝。拖放事件本身不创建任务，读取工作由应用显式选择 window/app scope（[ADR-0016](docs/adr/0016-ole-file-drop-target.md)、[stage-three 文件拖放验收](docs/实现/stage-three-文件拖放-验收.md)）；
- 长列表场景构建：跨重建复用未变化的文本 layout 与测量结果，viewport 外的元素保留几何但跳过 glyph 与绘制；声明 `row-height` 的窗口化 `For` 只挂载可见行窗口，完整行范围由上下 spacer 保持（[ADR-0011](docs/adr/0011-windowed-for-mounting.md)）；变量行高仍不在本里程碑；
- UIA：每个窗口从已提交快照与场景发布根、角色、名称、AutomationId、状态与物理边界，`WM_GETOBJECT` 交回 fragment-root provider；滚动只平移边界并保持 runtime ID。Invoke、Toggle、Value 与 SetFocus 把动作排队到 UI 线程，按最新快照重验后复用 Runtime 命令路径。相邻两次发布按 runtime ID 比较，聚焦、启用、ToggleState、Value 与子节点增删/重排各投递一次事件；后台窗口不投递聚焦事件，指针 hover/press 瞬态不产生事件（[只读树验收](docs/实现/stage-three-辅助技术-验收.md)、[动作验收](docs/实现/stage-three-辅助技术动作-验收.md)、[事件验收](docs/实现/stage-three-辅助技术事件-验收.md)、[合同与朗读验收](docs/实现/stage-three-辅助技术总验收.md)）；
- UIA 文本：`text-input` 的 Edit 节点暴露最小 `ITextProvider`/`ITextRangeProvider`。文档取已提交值并排除 preedit，对外偏移是 UTF-16 code unit，运行时仍用字符偏移；GetText、GetSelection、GetVisibleRanges、GetBoundingRectangles 与 Select 可用，Select 经动作队列调用 `set_selection_node` 且不派发 `on:input`；查找、移动、扩展、比较、多选与滚动等 range 方法返回 `UIA_E_NOTSUPPORTED`，组合中、禁用或无 `on:input` 的输入保持只读（[文本验收](docs/实现/stage-three-辅助技术文本-验收.md)）；
- UIA 滚动：可滚动窗口的根暴露垂直 `IScrollProvider`。`VerticallyScrollable` 取 `max_scroll_offset > 0`，垂直 view size 为 viewport/content 百分比，垂直百分比由已应用偏移换算；水平属性固定为 `NoScroll`。`Scroll` 的大小步长复用键盘翻页与滚轮行步长，`SetScrollPercent` 把 0..100 四舍五入到最近的整数逻辑像素后夹取。provider 只入队，UI 线程按当前场景重验；隐藏、禁用、零最大偏移与关闭中的窗口拒绝写入。滚动只改宿主偏移，Node ID、焦点、caret、selection、draft、组合、快照相等性与运行时计数不变（[滚动验收](docs/实现/stage-three-辅助技术滚动-验收.md)）。
- 文本编辑历史：每个 `text-input` 在运行时保存有界 undo/redo；Windows 宿主把 Ctrl+Z/Y 接到聚焦启用输入，连续单字符输入合并为至多 64 个字符的撤销步（空白并入前一个 run），移动、焦点、删除、多字符插入、整体替换、组合、撤销/重做与外部替换开启新步；撤销/重做各恰好一次 `on:input`，模型仍是值的权威（[ADR-0014](docs/adr/0014-text-edit-history.md)）。
- 剪贴板复制、剪切与粘贴：Windows 宿主把 Ctrl+C/X/V 接到可注入的 `ClipboardProvider`；可见宿主读写 `CF_UNICODETEXT`，隐藏宿主默认不触碰系统剪贴板。复制精确写出已提交选区且不派发 `on:input`，剪切只在写入成功后删除选区，粘贴过滤控制字符后通过同一插入 reducer 替换选区或插入 caret；剪切与粘贴各恰好一次 `on:input`，AltGr 组合不触发快捷键（[ADR-0015](docs/adr/0015-clipboard-host-capability.md)、[stage-three 剪贴板验收](docs/实现/stage-three-剪贴板-验收.md)）。
- 有界运行时存储：最近发布快照按固定窗口保留，释放的 Owner 节点回到存活规模，Todo 的列表 reconciliation 与行句柄查找使用索引而不是逐行扫描；
- 可复现的 Todo 场景：插入、删除、筛选、重排、焦点与拒绝句柄后的恢复；
- 独立的宏、文本、布局和 GPU 原型；
- 目标界面语法的预览。

仓库内的示例用于了解已有能力，不是新应用的起点：

```sh
cargo run -p zenui-app --example todolist_headless --locked
cargo run -p zenui --example minimal_todo --locked
cargo run -p zenui-windows --example todo_host --locked
```

原生示例的主窗口工具栏可以打开共享模型的详情窗口；隐藏会保留详情窗口的本地编辑状态，关闭后可以重新打开。`Import file` 打开锚定到主窗口的原生打开文件对话框，把每个选中 UTF-8 文本文件的非空行提交为任务，并在主窗口关闭后仍由应用所有权任务完成导入；把文本文件拖入主窗口或 Details 会走同一条应用所有权导入。主窗口有非空草稿时，关闭按钮会显示 Continue editing / Discard draft 确认窗口。

target API 的公开行为测试位于根包 `tests/`，其中 `tests/tasks.rs` 覆盖窗口关闭、重开代际、过期交付与 app 所有权模型提交，`tests/ime.rs` 覆盖 headless composition 的 preedit、候选确认与取消；这些测试不代替原生窗口或真实 IME 验收，原生路径由 `crates/windows` 的隐藏 smoke 与可见窗口验收覆盖。

根包的 `src/main.rs` 只是占位入口（打印 `Hello, world!`），不是新应用的起点；`zenui-windows` 的 `todo_host` 是仓库内示例，用于演示已实现的原生能力。目标语法的预览文件不参与默认 workspace 构建，因为只有部分所需 API 已经存在。

## 阅读路径

### 产品与路线

- [愿景](docs/产品/愿景.md)：产品定位、原则与边界。
- [路线图](docs/产品/路线图.md)：以 Windows 为首个完整产品平台的实现顺序。

### SDK

- [快速开始](docs/SDK/快速开始.md)：模板入口、工具链、工作区结构与内部示例。
- [从空目录开始](docs/SDK/从空目录开始.md)：在仓库外复制模板或使用 git 依赖，准备 Windows 构建环境、运行第一个窗口，并生成可双击运行的 release exe。
- [界面与组件](docs/SDK/界面与组件.md)：组件、属性、条件、列表和 `view!`。
- [数据与更新](docs/SDK/数据与更新.md)：Signal、Memo、Store、Field 和异步边界。
- [窗口与任务](docs/SDK/窗口与任务.md)：窗口实例、关闭确认、对话框和任务归属。
- [样式与输入](docs/SDK/样式与输入.md)：静态 class、typed style、文本编辑和 IME。

### 实现

- [技术架构](docs/实现/技术架构.md)：目标技术模型、研究借鉴与架构边界。
- [渲染与平台](docs/实现/渲染与平台.md)：布局、文本、绘制、内容接入和宿主能力。
- [验证重点](docs/实现/验证重点.md)：目标模型落地时需要验证的关键语义。
- [性能门禁](docs/实现/性能门禁.md)：固定负载、基线文件、计数硬上限、时间噪声预算与固定 runner 运行方式。
- [门禁覆盖](docs/实现/门禁覆盖.md)：方案 §9.3/§20 的正确性、性能、文档与 ADR 门禁到测试/页面/剩余工作的映射。
- [可见验收矩阵](docs/实现/stage-three-可见验收矩阵.md)：Phase 0–5 可见 Windows 要求与既有证据等级。
- [Windows 手动检查清单](docs/实现/windows-手动检查清单.md)：真实桌面逐项执行与证据记录入口。
- [ADR-0007](docs/adr/0007-window-catalog-and-close-epoch.md)：窗口目录、per-activation 权威与关闭 epoch。
- [ADR-0008](docs/adr/0008-file-picker-host-tasks.md)：打开文件选择器、宿主任务、显式所有权与 STA 模态对话框。
- [ADR-0009](docs/adr/0009-file-dialog-family.md)：打开/保存/文件夹对话框家族、kind 矩阵与单路径校验。
- [ADR-0010](docs/adr/0010-scroll-is-presentation-state.md)：滚动是宿主拥有的整页呈现偏移，几何、命中与 IME 共享同一份已平移场景。
- [ADR-0011](docs/adr/0011-windowed-for-mounting.md)：窗口化 `For` 只挂载可见行窗口，spacer 保持完整行范围，宿主是窗口换算方。
- [ADR-0012](docs/adr/0012-uia-derived-tree-and-action-queue.md)：UIA 树按 HWND 派生，provider 只读发布，写操作经每窗口动作队列。
- [ADR-0013](docs/adr/0013-menu-and-tray-host-services.md)：菜单、快捷键、上下文菜单与托盘是运行时命令路径上的宿主服务；原生 `HMENU`/`HACCEL`/`HICON`/`NOTIFYICONDATAW` 生命周期归宿主。
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
- [Stage-three 辅助技术滚动验收](docs/实现/stage-three-辅助技术滚动-验收.md)：窗口根垂直 ScrollPattern 的读取、百分比映射、动作队列重验与 100%/150% 可见证据。
- [Stage-three 辅助技术合同与朗读验收](docs/实现/stage-three-辅助技术总验收.md)：每窗口 fragment root、树与角色映射、动作队列、三窗口 Inspect 属性与 Narrator Speech Recap 的 100%/150% 证据。
- [Stage-three 菜单栏验收](docs/实现/stage-three-菜单栏-验收.md)：原生菜单栏、命令路由与弹窗时 enabled/checked 状态在 100%/150% 下的可见证据。
- [Stage-three 快捷键验收](docs/实现/stage-three-快捷键-验收.md)：`HACCEL` 翻译、编辑器让位与 Ctrl+S 在 100%/150% 下的可见证据。
- [Stage-three 上下文菜单验收](docs/实现/stage-three-上下文菜单-验收.md)：右键、Shift+F10 与 Apps 打开同一窗口级菜单的 100%/150% 可见证据。
- [Stage-three 托盘验收](docs/实现/stage-three-托盘-验收.md)：系统托盘图标、tooltip、点击恢复、右键应用命令、`TaskbarCreated` 与 Windows 11 溢出的可见证据。
- [Stage-three 文本编辑历史验收](docs/实现/stage-three-文本编辑历史-验收.md)：按输入的撤销/重做、Ctrl+Z/Y、连续输入合并、选择替换、Backspace/Delete、中文组合提交、两窗口隔离与 150% 可见证据。
- [Stage-three 剪贴板验收](docs/实现/stage-three-剪贴板-验收.md)：Ctrl+C/X/V、Notepad 双向互操作、应用内剪切与跨窗口粘贴、空/非文本剪贴板、组合 no-op、焦点保持与 100%/150% 可见证据。
- [Stage-three 文件拖放验收](docs/实现/stage-three-文件拖放-验收.md)：真实资源管理器拖放的单文件/多文件、Unicode/非 BMP/长路径、文件夹、非文件、无处理器、关闭提示、模态选择器与关闭重开的 100%/150% 可见证据与边界。
- [Stage-three 空目录启动验收](docs/实现/stage-three-空目录启动-验收.md)：仓库外模板副本的可见窗口、UIA 计数与 150% 证据；CI copy-out 与可见运行分开记录。
- [Stage-three 发布打包验收](docs/实现/stage-three-发布打包-验收.md)：release exe 的 GUI 子系统、复制到另一目录后的可见运行与 UIA 计数。
- [Stage-three 最终跨方向验收](docs/实现/stage-three-最终跨方向-验收.md)：模板、滚动、UIA、菜单/托盘、剪贴板/编辑历史、拖放与设备丢失的单提交汇总索引与 100%/150% 整合运行。

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

## 许可证

ZenUI 项目自身编写的代码与文档以 `MIT OR Apache-2.0` 双重许可发布，你可以任选其一使用。许可全文见 [LICENSE-MIT](LICENSE-MIT) 与 [LICENSE-APACHE](LICENSE-APACHE)。

仓库中的第三方内容与资源保留各自许可，例如 `.agents/skills` 中的技能与 `examples/native_poc` 的 Tuffy 字体；上游声明见对应文件。
