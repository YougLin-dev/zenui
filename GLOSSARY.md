# ZenUI

ZenUI 是一个 Rust 原生桌面界面框架；这个上下文覆盖 target SDK、headless runtime 与 Windows 宿主之间的共享语言。

## Language

**Activation**:
一个窗口实例的身份；关闭后旧 activation 的交付被丢弃，重开产生新 activation。目录键上的打开代际由 `ActivationId` 的 runtime 全局令牌承载。
_Avoid_: window instance, window generation

**ActivationId**:
runtime 全局唯一的窗口激活令牌 `{ slot, generation }`；slot 单调分配、不复用，generation 是目录键上的打开代际。provider 把 `(runtime, ActivationId)` 注入动作请求，UI 线程据此丢弃旧 activation 或其它 runtime 的竞态动作；`WindowCtx::activation_id()` 与 `NodeId::runtime_id()` 暴露同一身份。
_Avoid_: window handle, catalog key

**Application owner**:
归属应用而不是某个窗口的 owner 0；`Runtime::setup` 创建的 Signal、Memo 与 Store 在窗口关闭后仍保留。
_Avoid_: global state, app singleton

**Application session**:
一个 Runtime 拥有的应用级生命周期与最后窗口策略。默认 `QuitOnLastClose` 在最后一个窗口关闭后结束宿主消息循环；`StayResident` 显式允许零窗口会话继续，原生宿主要求同时声明托盘。`Runtime::session()` 返回的 Copy `SessionCtx` 在命令处理器中排队 `request_show` 与 `request_quit`；显式退出关闭全部 activation、释放待决关闭确认并让之后的 show 返回 `State`。直接调用 `Runtime::quit` 时同一 turn 尚未结算的窗口请求被丢弃；排队的 `request_quit` 在窗口请求之后结算。
_Avoid_: process lifetime, app loop

**Window catalog**:
启动时声明的静态窗口集合；每个窗口有稳定键、原生标题、初始逻辑尺寸、primary 标记、ClosePolicy、WindowRole 与可重复执行的 factory。声明本身不挂载界面，并拒绝重复键、缺失或多个 primary 以及无效的关闭确认绑定。
_Avoid_: window list, registry

**Primary window**:
目录中恰好一个的默认窗口；Host 首次显示以它为入口。
_Avoid_: main window (when meaning the catalog flag)

**Hidden window**:
保持 activation、树、焦点与本地编辑状态，但不接收输入、不呈现的窗口；show 恢复同一 activation。
_Avoid_: closed window, minimized window (when meaning runtime visibility)

**Window request**:
UI 处理器通过 `WindowCtx` 排队的 show/hide/close 生命周期动作；在事件或交付结算后由 `Runtime::apply_window_requests` 执行，来源 activation 关闭后失效。
_Avoid_: window command, deferred window event

**Close policy**:
目录窗口收到关闭请求时的处理方式：`Immediate` 直接释放 activation，`Confirm` 先创建一个 close epoch 并等待 Allow/Deny。
_Avoid_: close mode, quit policy

**Close epoch**:
一个父窗口 activation 的一次待决关闭请求；绑定父窗口、声明的确认窗口和挂起前的焦点，只有第一个有效 Allow/Deny 生效。待决期间父窗口输入挂起。
_Avoid_: close transaction, close session

**Window role**:
目录窗口在关闭协议中的角色：`Normal`，或绑定一个 `Confirm` 父窗口的 `CloseConfirmation { parent }`。每个父窗口至多声明一个确认窗口，确认窗口自身必须是 `Immediate`。
_Avoid_: dialog role, modal role

**Close confirmation window**:
目录中以 `CloseConfirmation { parent }` 声明并绑定到 `Confirm` 父窗口的目录窗口；其 X、Escape 或关闭请求在 epoch 待决时表示 Deny。
_Avoid_: dialog, modal (when meaning the catalog role)

**File dialog**:
一个由应用从 `WindowCtx::request_file_dialog` 请求、由宿主后端呈现的原生打开、保存或选择文件夹对话框；按 kind 携带标题、类型过滤器、建议文件名、默认扩展名与单选/多选选项，返回 `Picked`、`Cancelled` 或 `Failed`。保存与文件夹都是单路径契约；文件夹忽略过滤器、建议文件名与默认扩展名。它不是窗口目录条目，也不等于关闭确认窗口。
_Avoid_: file dialog window, file picker window, file drop, import dialog

**File drop**:
通过 OLE `IDropTarget` 从资源管理器或其它 OLE 拖放源送到一个可见窗口的一组有序路径。只有可见且在 factory 注册了 `on_drop` 的窗口拥有 target；源必须提供 `CF_HDROP` 并允许 COPY，窗口被禁用（例如模态对话框的 owner 或等待关闭确认）时返回 NONE。宿主在 UI 线程 drain 后以 `DropEvent` 运行一次处理器；它不是文件对话框，也不创建任务或 pending 状态，处理器自行选择 window/app task scope。路径保留顺序、重复、Unicode、非 BMP 与长路径；文件夹也是路径，非文件数据在处理器前拒绝。
_Avoid_: file picker, host task, drag event (when meaning the SDK event), drop task

**Host task**:
由后端服务而不是 worker 线程完成的任务；任务记录没有结果通道，只能经 `Runtime` 的宿主完成入口以恰好一次的方式完成，并在 `Delivery` turn 交付。文件对话框是当前的宿主任务；它与 worker 共享 Task 句柄和 continuation，但不被 `settle_tasks` 等待。
_Avoid_: host worker, UI task, dialog task, file drop

**Host**:
拥有原生窗口、事件循环、DPI 与 surface，并驱动 target runtime 的平台层。
_Avoid_: platform backend, shell

**turn**:
一次派发（用户输入、UIA 动作或任务交付）从 handler、模型/UI 写入、flush、快照发布到宿主动作结算的同步边界。一个 turn 可以包含多次提交，发布会累计这些变化；空 click、相等写入与纯 runtime 计数变化仍构成 turn，但不产生 effective change。
_Avoid_: frame, event loop iteration

**Intent**:
平台输入或测试助手表达的、尚未绑定具体处理器的动作：来源、目标、`IntentPayload`（Click、Activate、Press、KeyDown）与修饰键。runtime 在已提交树上按 closest-wins 解析目标并生成 `TurnReport`；它不是 UIA 的 `UiAction`。
_Avoid_: input event (when meaning the semantic action), UIA command

**UiAction**:
UIA provider 排入窗口动作队列的 Node 载荷：Invoke、Toggle、ReplaceValue、SetSelection、SetFocus。UI 线程按 FIFO 重验窗口、NodeId、启用、可见、挂起与组合状态后调用对应 runtime 命令；Scroll 不是 UiAction。
_Avoid_: UIA command (when meaning the Node payload), native click

**TurnReport**:
一次 turn 的公开报告：`TurnOutcome`、错误列表、`RevealFlags` 与 `changed`。`changed` 只在呈现有效变化时为真；宿主据 outcome 决定是否执行原生默认动作，据 reveal 置位显露。
_Avoid_: event result, frame report

**TurnOutcome**:
turn 的 `consumed` 与 `prevent_default` 两个布尔结果；处理器、窗口根 keydown 与宿主默认动作共同决定。它不表示模型事务是否提交成功，后者由错误与模型结果表达。
_Avoid_: handler result, default action flag

**effective change**:
会改变已发布呈现的提交：属性、文本、直接子序、焦点、外观或行带等。相等写入、空 click、只推进内部代数或计数的操作不是 effective change；它们不推进树 revision、不构建快照。
_Avoid_: dirty state, write

**closest-wins**:
派发沿“最深命中节点 → 祖先 → 窗口根”的顺序选择第一个拥有该事件处理器的元素；只有它运行，行内按钮自己的处理器因此优先于行处理器。原生交互控件没有处理器时阻断祖先穿透；未消费只影响默认动作，不会把同一事件交给下一个祖先处理器。capture/bubble 变体显式增加祖先或根观察；窗口根 keydown 是最后兜底。
_Avoid_: bubble order, event propagation

**PresentationKey**:
跨快照稳定的呈现身份，是 `NodeRef` 的 8 B 公开编码；元素与文本节点都携带。renderer 用它索引节点与应用 `PresentationDelta`，槽位复用时 generation 改变，旧 key 不会命中新节点。
_Avoid_: snapshot id, render id

**Read**:
intrinsic 数据位与组件 prop 接受的值源：`Signal`、`Memo`、`Field`、`Once` 或静态值。挂载期转换为绑定槽；动态源只在值变化代数或（文本输入的）写入尝试代数推进时求值，静态/once 值在挂载时提交后不再重算。
_Avoid_: binding value, observable

**ForSource**:
`For` 的行集读源抽象：`Rows` 投影、`RowSet` 读源或已预挂载的行来源。它决定行身份顺序；`store` 与 `key` 分别提供行数据与业务键，三者共同建立 keyed 行映射。
_Avoid_: list data, row provider

**RowView**:
`For` 行 factory 的参数。`id()` 是行身份，`key()` 是业务键，`get()` 是本次挂载的快照；行重新进入挂载窗口时得到新的 `RowView` 与本地状态。
_Avoid_: row handle, row item

**OwnerRef**:
owner arena 的 8 B 身份（32-bit slot + 32-bit generation），随 Owner 释放失效。元素、资源与子 Owner 通过它归属清理链；旧 OwnerRef 不能命中复用槽位。
_Avoid_: owner id, scope id

**Cleanup**:
`on_cleanup` 在挂载期登记的 owner 收尾回调。同层按登记逆序执行，子 owner 先于父；全部 cleanup 在资源释放前运行，因此可以读取本 owner 仍存活的 Signal/Memo。创建 Signal/Memo、挂载或写入被拒绝为 Context 错误；单条 panic 收敛为 `RuntimeDiagnostic::CleanupPanicked`，后续回调继续。见 [ADR-0027](docs/adr/0027-reactive-slot-recycling-and-cleanup.md)。
_Avoid_: destructor, drop hook

**Source generation**:
Signal/Memo 槽位的 32-bit 代际。槽位随 owner 释放进入 free list，复用时递增；句柄、SourceRef、绑定来源与 oracle 缓存都按代际校验。旧代际读取被拒绝（可失败读取为 `ReadFault::Disposed`），并由 `RuntimeDiagnostic::StaleSource` 上报。`DepKey` 仍以槽位索引为键，释放时的双向解绑保证索引不会命中新代际。
_Avoid_: handle id, source id

**BindingPart**:
绑定在节点上的稳定部位标识（文本、属性、样式、Show when、For each、值等）。绑定补丁按 `(tree, node, part)` 去重，同一 turn 的重复求值不会产生重复补丁。
_Avoid_: binding key, attribute id

**BindingTable**:
每棵树的类型化绑定槽表，按值类型分 arena；元素只保存 `BindingListId`，槽记录 source、current、fault、generation 与上次成功结算的源代数。槽位复用换 generation，旧 `BindingRef` 失效。
_Avoid_: binding map, slot table

**StoreId**:
Store 实例的稳定身份。`RowSetData` 记录来源 `StoreId`；`For` 拒绝来自其他 Store 的行集，跨 Store 的身份不会被当作同一行。
_Avoid_: store index, model id

**ReadFault**:
typed 读取失败：Disposed、FieldMissing、ComputedPanicked 或 GraphCycle。绑定保留最后已提交值并把 fault 记在槽位上，`RefreshReport::faults()` 报告本轮遇到的失败；故障槽下一轮继续重试。
_Avoid_: read error, missing value

**CloseOutcome**:
关闭处理器对一次 `CloseRequest` 的结算：Allow 释放父窗口与确认窗口，Deny 释放确认窗口并恢复父窗口，Pending 保持 epoch 待决并要求同一 closing turn 内显示确认窗口。它不同于窗口方法返回的 typed `CloseError`。
_Avoid_: close result, confirm result

**PresentationSnapshot**（presentation snapshot）:
从已提交界面状态生成的不可变、结构共享的派生描述，含稳定 node ID、tag、渲染所需属性、文本与本地编辑状态；不含几何、闭包或句柄，也不是第二棵保留树。对外唯一发布的单位是 `PresentationSnapshot`，它包裹内部的 `Arc<SnapshotRoot>`：同一树 revision 的读取共享同一个根，`Clone` 只增加引用计数，旧根在最后一个引用释放后回收；相等性先比较 revision 与指针身份，再退化到内容比较。元素与文本节点都携带跨快照稳定的 `PresentationKey`。
_Avoid_: render tree, view tree, frame

**Interaction paths**:
宿主逐帧维护的 hover/press/focus“最深节点 → 根”路径集合，按 `Arc<[NodeId]>` 共享。`hover:`/`active:` 条件对路径内任意元素成立，内建 hover/pressed 视觉只落在最深节点；路径不进入 presentation snapshot 或响应式图。路径内容变化推进 `generation`，与快照 revision、环境代际一起构成场景缓存键。
_Avoid_: hover node, hit path

**EnvGeneration**（environment generation）:
每个窗口独立的呈现环境代际，随系统偏好、DPI、滚动偏移与焦点 modality 变化递增；它不是树 revision，也不进入快照。文本缩放/DPI 变化据此重建布局、整形与行带，高对比度据此重建调色板与场景，而 `tree.revision` 保持不变。
_Avoid_: env version, ui revision

**RevealFlags**（reveal flags）:
一次 turn 报告（`TurnReport::reveal`）中的焦点、caret/selection 与组合变化标志；由 runtime 在 turn 开始与结算时比较同一棵树的焦点节点、caret 与 anchor（selection 由二者派生）以及 preedit 得出。宿主只在任一标志为真时置位待决显露，显式滚动清除它；标志不进入快照或响应式图。
_Avoid_: scroll request, focus event

**PresentationDelta**（presentation delta）:
一次提交相对上次成功发布的累计变化，按 `PresentationKey` 给出 changed（含祖先路径）、inserted、removed、removed subtrees 与 layout roots，以及焦点/外观/行带标志；不携带旧值或父链。renderer 用它失效增量缓存，UIA 不消费 delta。
_Avoid_: diff, patch

**PresentationUpdate**（presentation update）:
`PresentationSnapshot` 与可选 `PresentationDelta` 的发布单位。delta 为 `None` 表示完整根：首次发布、隐藏窗口重新显示后的首次发布、顶层根序列变化或无法安全累计时使用。renderer 侧的 `PresentationView` 维护 `applied_revision`，只接受与已应用 revision 连续且与快照同代的 delta；断档时丢弃 delta、按最新根全量重建。
_Avoid_: frame packet, sync message

**Windowed For**:
声明 `row-height` 的 `For`：只挂载宿主行窗口内的行，上下两个 spacer 保持 `总行数 × 行高` 的声明范围；不声明 `row-height` 的 `For` 继续全量挂载。
_Avoid_: virtual list, lazy list

**Row band**:
一个窗口化 `For` 的完整行范围与当前挂载窗口元数据：稳定 band key、顶端 spacer Node ID、统一行高、起始行与总行数。快照携带元数据，场景携带解析后的几何。
_Avoid_: list range, viewport band

**Row window**:
宿主从行带几何、已应用滚动偏移与视口高度换算、经 `Runtime::set_row_windows` 批量应用的 `(first, count)` 挂载区间；同一屏的全部行带（包括嵌套行带）在一个原子结构 turn 里预校验并应用，任一失败则整批不落地。运行时是唯一挂载或卸载行的一方。
_Avoid_: visible range, slice

**Row height**:
窗口化 `For` 声明的统一外层行槽高度；父容器必须零间距，每一行必须恰好占满它。
_Avoid_: item height, row extent

**Scene**:
`zenui-render` 根据 presentation snapshot 产出的布局几何、文本 run 与绘制指令；与 GPU 后端无关。
_Avoid_: display list, frame

**Viewport**:
一次场景构建使用的逻辑视口尺寸与 Window scale；布局始终使用逻辑像素。
_Avoid_: screen size, resolution

**Render target**:
GPU 层的一张目标纹理及其格式与物理像素尺寸；窗口呈现时由 Surface 提供，离屏测试使用独立纹理。
_Avoid_: surface (when meaning the texture target)

**Text run**:
场景中同一字体、字号与 shaping 结果的一段文本，携带绝对字形位置；不是平台文本句柄。
_Avoid_: glyph line, paragraph

**Surface**:
一个顶层窗口的呈现目标；随窗口尺寸、DPI 或设备代际变化重建。
_Avoid_: swapchain, render target

**Inline declaration**:
通过 `style:<property>` 给出的类型化样式声明；携带 `StyleOrigin::Inline`，覆盖同属性的 class 声明与状态变体。
_Avoid_: inline css, dynamic css

**Class token**:
`class` 字面量中一个空白分隔的项，展开期解析为属性、值与可选条件；词表封闭，未登记的 token 是编译错误。
_Avoid_: css class, utility string

**Condition**:
一条声明生效所需的已提交状态、应用外观或宿主瞬态，例如 `dark:`、`hover:`、`aria-pressed:`；同属性冲突先比条件 rank，再比源码顺序。
_Avoid_: pseudo-class, selector

**Appearance**:
应用的 `Light`/`Dark` 外观，属于已提交状态，随快照参与相等性并影响调色板与 `dark:`；不跟随操作系统设置。系统高对比度开启时，宿主用高对比度调色板覆盖角色颜色。
_Avoid_: theme (when meaning the OS mode), color scheme

**System preferences**:
宿主采样的 Windows 高对比度与辅助功能文本缩放，属于宿主输入，不进入快照或应用事务；高对比度覆盖角色调色板，文本缩放放大字号与 `em` 几何。
_Avoid_: accessibility theme, font scale (when meaning the OS setting)

**Resolved style**:
场景构建时由 intrinsic 默认、class 与 inline 声明合并后的值，供布局、文本整形与绘制共用。
_Avoid_: computed style, final css

**Presentation transient**:
宿主拥有、逐帧变化且不进入快照的呈现状态，例如 hovered、pressed 与键盘模态。
_Avoid_: interaction state, hover state (when meaning committed state)

**Applied scroll offset**:
宿主保存的整页逻辑像素偏移，已按当前内容范围夹取并作为下一次 `SceneEnv` 的输入；滚动只平移场景几何，不改变快照、Node ID、焦点或 caret。滚轮、翻页键、滚动条与 UIA `ScrollPattern` 写入同一个值。它是持久的 presentation 环境值，不是 presentation transient，也不是应用状态。
_Avoid_: scroll position (when meaning business state), scroll signal

**Node ID**:
presentation snapshot 中用于命中测试与事件派发的稳定元素身份；不是 label，也不是公开的内部数组下标。
_Avoid_: element index, view ID

**Accessibility tree**:
每个已提交窗口从 presentation snapshot 与同一份 `Scene` 派生的只读 UIA 树；发布时整体替换，不消费 `PresentationDelta`，而是在相邻两次已发布树之间做比较，节点身份沿用 Node ID，provider 解析当前发布而不是保留第二份界面。每个已发布窗口有自己的 fragment root（ADR-0012）。写 pattern、TextPattern 的 Select 与窗口根的 ScrollPattern 从这棵树派生动作并排给 UI 线程，不在 provider 线程修改 Runtime；事件由相邻两次发布派生，见 accessibility event。
_Avoid_: a11y DOM, accessibility model (when meaning a separate retained tree)

**Accessibility provider**:
每个已发布窗口的 UIA fragment root 与元素 provider；持有窗口发布单元的弱引用，按调用线程解析当前不可变树，不保存第二份界面，也不在 provider 线程修改 Runtime。写操作经每窗口动作队列交给 UI 线程重验；隐藏、过期或清除发布的查询返回 `UIA_E_ELEMENTNOTAVAILABLE`。
_Avoid_: a11y proxy, retained UIA model

**ProviderAction**（provider action）:
provider 线程排入、UI 线程按 FIFO 取出并重验的一条写请求：`Node(UiActionRequest)` 或 `Scroll(ScrollRequest)`，两者共享每窗口 1024 上限、顺序与隐藏/关闭/零客户区清空。请求只含 POD 身份与载荷，不含 `WindowCtx`；Scroll 请求不携带 NodeId。
_Avoid_: UIA queue item, host action

**UIA action**:
Provider action 的 Node 载荷：Invoke、Toggle、ReplaceValue、SetSelection、SetFocus。UI 线程按最新快照重验 NodeId、窗口身份、启用、可见、挂起与组合状态后调用对应的 Runtime 命令；SetFocus 额外要求前台，ReplaceValue 与 SetSelection 在组合中拒绝。Scroll 不是 UIA action，由宿主在显式滚动路径执行。
_Avoid_: UIA command (when meaning the Node payload), native click

**Accessibility event**:
相邻两次已发布 UIA 树按 runtime ID 比较后产生、在发布线程投递的一次通知；顺序为结构、属性、焦点。UIA 整树发布并比较相邻发布，`PresentationDelta` 只给 renderer，不参与事件派生。只有前台窗口投递焦点事件，后台焦点变化不在激活时补发；焦点清空以 `GetFocus` 为空表达，没有单独事件。hover/press 与滚动几何不进入树，因此不产生事件。
_Avoid_: accessibility notification (when meaning transient UI changes), screen reader message

**ScrollPattern**:
窗口根上的最小 UIA `IScrollProvider`：只报告垂直可滚动状态、垂直 view size（viewport 与 content 的百分比）和由 applied scroll offset 算出的垂直百分比，水平接口固定为 `NoScroll`。`Scroll` 与 `SetScrollPercent` 派生 ScrollCommand 进入 provider action 队列，UI 线程按当前场景夹取后走宿主显式滚动路径；百分比按最近整数逻辑像素映射到 `[0, max]`。
_Avoid_: scrollbar provider, scroll model (when meaning a second offset)

**TextPattern**:
文本输入 Edit 节点上的最小 UIA 文本接口：已提交文档（不含 preedit）、当前 selection、文本、场景矩形与 Select。对外偏移是 UTF-16 code unit，运行时仍是字符偏移；Select 经 provider action 的 `SetSelection` 调用 `set_selection_node`，不派发 `on:input`。查找、移动、扩展、比较、多选与滚动等 range 方法不属于本切片，显式返回 `UIA_E_NOTSUPPORTED`。
_Avoid_: rich text provider, text model (when meaning the committed value)

**Text edit**:
在 caret/selection 上执行的平台无关编辑操作，例如插入、退格、删除；每次编辑派发一次 `on:input`。
_Avoid_: input event, keystroke

**Edit lineage**:
一个已挂载文本输入自己的本地编辑血统：origin 取挂载代际（NodeId generation），revision 随每次本地编辑单调前进；血统同时记录在途派发的摘要、已结算水位线与上次观察的已提交值。刷新时用它把已提交值分类为这次编辑的结算（含 handler 规范化或截断的值）、旧 revision 的 stale settlement 或外部 reset。应用值仍是普通 `String`，origin 与 revision 不是 SDK 值类型；同 key 重新挂载得到新 origin、新 revision 与空历史。
_Avoid_: TextValue, model revision, editor protocol value

**Text edit history**:
每个文本输入自己在运行时保存的有界撤销/重做历史：条目记录编辑前后的已提交值、caret、anchor 与归一化 selection；连续单字符输入按 run 合并（至多 64 个字符，空白并入前一个 run），移动、焦点变化、删除、多字符插入、整体替换、组合提交、撤销/重做与外部替换开启新步。撤销/重做通过一次 `on:input` 写回，模型仍是值的权威；精确恢复只对原样提交值的处理器成立。它不是应用层的业务撤销，也不跨输入共享。
_Avoid_: application undo, model undo (when meaning text-input history), shared edit history

**Clipboard**:
Windows 系统剪贴板上的 Unicode 文本交换，不是应用模型、第二套编辑器状态或跨输入共享的历史。可见宿主安装 `ClipboardProvider` 的 `CF_UNICODETEXT` 原生实现，隐藏宿主默认没有；聚焦启用输入上的 Ctrl+C 写出已提交选区且不派发 `on:input`，Ctrl+X 只在 provider 确认写入成功后才删除选区，Ctrl+V 读取文本、过滤控制字符后走同一插入 reducer。组合中、无焦点、禁用输入与读写失败都是静默无操作。它是宿主能力，不是 SDK 的剪贴板 API。
_Avoid_: text edit history, selection anchor, clipboard model, rich clipboard

**Selection anchor**:
selection 的固定端；caret 是活动端（焦点）。Shift+方向键移动活动端而 anchor 不动，折叠时两者相等。
_Avoid_: origin, base offset

**Transaction frame**:
core 的一次事务提交单位；无修饰的 “frame” 不用于 presentation。
_Avoid_: frame

**Application menu**:
窗口目录通过 `WindowCatalog::menu` 声明的原生菜单栏：每个窗口一份 `HMENU`，标签、分隔线、子菜单、快捷键与默认 enabled/checked 来自 `MenuSpec`，原生数字 id 只在窗口内按声明前序分配。选择以 `CommandSource::MenuBar` 进入运行时命令路径；它不是全局菜单，也不包含窗口级上下文菜单。
_Avoid_: global menu, app shell menu

**Menu command**:
`MenuCommand` 声明的一项菜单动作：稳定 id、标签、可选快捷键与默认 enabled/checked；它是声明而不是处理器。菜单栏、上下文菜单与快捷键按 id 进入同一命令路径，窗口处理器优先，窗口没有同一 id 时回退应用级处理器。子菜单与分隔线属于 `MenuSpec`，不是命令。
_Avoid_: menu action, menu item id

**Menu shortcut**:
`MenuCommand` 声明的 Ctrl/Alt 组合与键位；目录拒绝同一窗口的重复归一化快捷键，宿主为每个 HWND 建 `HACCEL`，并在 `TranslateMessage` 之前翻译成 `CommandSource::Accelerator` 命令。聚焦文本输入的 IME 组合与保留编辑键优先，禁用命令不派发。
_Avoid_: global hotkey, raw accelerator string

**Menu command state**:
窗口在菜单打开时通过 `WindowCtx::on_menu_state` 提交的只读 enabled/checked 结果；宿主在 `WM_INITMENU`/`WM_INITMENUPOPUP` 与派发前用 `Runtime::menu_state` 求值，没有回调时使用 `MenuCommand` 的声明默认值。回调内的 Signal 写与 Store 事务被拒绝为 typed error，模型保持已提交值。
_Avoid_: menu flag, UI state snapshot

**Context menu**:
窗口目录通过 `WindowCatalog::context_menu` 声明的窗口级菜单；与菜单栏共享命令处理器但命令 id 不能重复，也不声明快捷键。宿主从 `WM_CONTEXTMENU` 打开，presenter 在 `&mut Host` 借用之外用 `TrackPopupMenuEx(TPM_RETURNCMD)` 展示，返回命令以 `CommandSource::ContextMenu` 派发一次；右键与键盘（Apps/Shift+F10）命中同一份声明。
_Avoid_: node menu, right-click handler, popup window state

**Application command**:
通过 `Runtime::on_app_command` 在首个窗口打开或根树挂载前注册的应用级处理器；没有窗口时也能运行，事件通过 `CommandEvent::window` 暴露可选的目标 activation。窗口菜单声明项可以由它承接，窗口目标派发只在窗口没有注册同一 id 时回退到它，`CommandSource::Tray` 从不运行窗口处理器。处理器在 `Phase::Event` 中可写应用 Signal/Store 并排队窗口请求；启动后注册、重复 id 与 panic 返回 typed error。
_Avoid_: global command, background command

**Application undo**:
应用自己的业务状态撤销或事务回滚，由应用实现；文本编辑历史只撤销单个输入内的本地编辑，不等于应用撤销，也不会撤销窗口请求、模型事务或其它输入。本里程碑不提供通用应用撤销。
_Avoid_: text edit history, editor undo (when meaning application state)

**Tray icon**:
应用级托盘声明 `TraySpec`：一个 tooltip 与一份右键 `MenuSpec`，在首个窗口打开或根树挂载前通过 `Runtime::declare_tray` 注册，全部菜单命令必须有应用级处理器。宿主用一个从不显示的顶层托盘 HWND 接住自己的回调消息与 Explorer 的 `TaskbarCreated` 广播；左键或键盘激活恢复可见焦点窗口，没有可见焦点时恢复 primary，primary 不在时选择任一存活窗口；右键在宿主借用之外展示菜单并以 `CommandSource::Tray` 派发恰好一次。默认会话策略下图标随最后一个窗口关闭移除；`StayResident` 会话保留图标并可在零窗口时恢复 primary。`TaskbarCreated` 后重新添加。第一切片使用共享系统图标，不承诺自定义图标资产；零窗口的托盘独存由应用会话策略显式决定，不是隐式默认，隐藏到托盘由应用的 hide 请求进入。
_Avoid_: notification area widget, tray window, background app icon

**Tray presenter**:
`TrayPresenter` 是宿主执行 `Shell_NotifyIconW` 与右键菜单的 seam。可见宿主安装原生 presenter；隐藏宿主默认没有，测试注入 fake 记录 add/update/remove/menu，CI 不触碰真实通知区域。
_Avoid_: tray backend, shell hook

**Window scale**:
窗口逻辑坐标到物理像素的缩放；布局始终使用逻辑像素。
_Avoid_: DPI factor, zoom
