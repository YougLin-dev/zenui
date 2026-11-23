# ZenUI

ZenUI 是一个 Rust 原生桌面界面框架；这个上下文覆盖 target SDK、headless runtime 与 Windows 宿主之间的共享语言。

## Language

**Activation**:
一个窗口实例的身份；关闭后旧 activation 的交付被丢弃，重开产生新 activation。
_Avoid_: window instance, window generation

**Application owner**:
归属应用而不是某个窗口的 owner 0；`Runtime::setup` 创建的 Signal、Memo 与 Store 在窗口关闭后仍保留。
_Avoid_: global state, app singleton

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
_Avoid_: file dialog window, file picker window, import dialog

**Host task**:
由后端服务而不是 worker 线程完成的任务；任务记录没有结果通道，只能经 `Runtime` 的宿主完成入口以恰好一次的方式完成，并在 `Delivery` turn 交付。文件对话框是当前的宿主任务；它与 worker 共享 Task 句柄和 continuation，但不被 `settle_tasks` 等待。
_Avoid_: host worker, UI task, dialog task

**Host**:
拥有原生窗口、事件循环、DPI 与 surface，并驱动 target runtime 的平台层。
_Avoid_: platform backend, shell

**Presentation snapshot**:
从已提交界面状态生成的一次性只读描述，含稳定 node ID、tag、渲染所需属性、文本与本地编辑状态；不含几何、闭包或句柄，也不是第二棵保留树。
_Avoid_: render tree, view tree, frame

**Windowed For**:
声明 `row-height` 的 `For`：只挂载宿主行窗口内的行，上下两个 spacer 保持 `总行数 × 行高` 的声明范围；不声明 `row-height` 的 `For` 继续全量挂载。
_Avoid_: virtual list, lazy list

**Row band**:
一个窗口化 `For` 的完整行范围与当前挂载窗口元数据：稳定 band key、顶端 spacer Node ID、统一行高、起始行与总行数。快照携带元数据，场景携带解析后的几何。
_Avoid_: list range, viewport band

**Row window**:
宿主从行带几何、已应用滚动偏移与视口高度换算、经 `Runtime::set_row_window` 应用的 `(first, count)` 挂载区间；运行时是唯一挂载或卸载行的一方。
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
应用的 `Light`/`Dark` 外观，属于已提交状态，随快照参与相等性并影响调色板与 `dark:`；不跟随操作系统设置。
_Avoid_: theme (when meaning the OS mode), color scheme

**Resolved style**:
场景构建时由 intrinsic 默认、class 与 inline 声明合并后的值，供布局、文本整形与绘制共用。
_Avoid_: computed style, final css

**Presentation transient**:
宿主拥有、逐帧变化且不进入快照的呈现状态，例如 hovered、pressed 与键盘模态。
_Avoid_: interaction state, hover state (when meaning committed state)

**Applied scroll offset**:
宿主保存的整页逻辑像素偏移，已按当前内容范围夹取并作为下一次 `SceneEnv` 的输入；滚动只平移场景几何，不改变快照、Node ID、焦点或 caret。它是持久的 presentation 环境值，不是 presentation transient，也不是应用状态。
_Avoid_: scroll position (when meaning business state), scroll signal

**Node ID**:
presentation snapshot 中用于命中测试与事件派发的稳定元素身份；不是 label，也不是公开的内部数组下标。
_Avoid_: element index, view ID

**Accessibility tree**:
每个已提交窗口从 presentation snapshot 与同一份 `Scene` 派生的只读 UIA 树；发布时整体替换，节点身份沿用 Node ID，provider 解析当前发布而不是保留第二份界面。写 pattern 从这棵树派生动作并排给 UI 线程，不在 provider 线程修改 Runtime；事件由相邻两次发布派生，见 accessibility event。
_Avoid_: a11y DOM, accessibility model (when meaning a separate retained tree)

**Accessibility action**:
provider 从当前 UIA 树派生、经窗口队列交给 UI 线程重验后执行的写操作（Invoke、Toggle、SetValue、SetFocus）；它最终调用 Runtime 的按 Node ID 命令，不保留第二份界面状态。
_Avoid_: UIA command, native click

**Accessibility event**:
相邻两次已发布 UIA 树按 runtime ID 比较后产生、在发布线程投递的一次通知；顺序为结构、属性、焦点。只有前台窗口投递焦点事件，后台焦点变化不在激活时补发；焦点清空以 `GetFocus` 为空表达，没有单独事件。hover/press 与滚动几何不进入树，因此不产生事件。
_Avoid_: accessibility notification (when meaning transient UI changes), screen reader message

**Text edit**:
在 caret/selection 上执行的平台无关编辑操作，例如插入、退格、删除；每次编辑派发一次 `on:input`。
_Avoid_: input event, keystroke

**Selection anchor**:
selection 的固定端；caret 是活动端（焦点）。Shift+方向键移动活动端而 anchor 不动，折叠时两者相等。
_Avoid_: origin, base offset

**Transaction frame**:
core 的一次事务提交单位；无修饰的 “frame” 不用于 presentation。
_Avoid_: frame

**Window scale**:
窗口逻辑坐标到物理像素的缩放；布局始终使用逻辑像素。
_Avoid_: DPI factor, zoom
