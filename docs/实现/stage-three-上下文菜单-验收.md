# Stage-three 窗口级上下文菜单验收（实机）

本页记录 #196：窗口目录声明窗口级上下文菜单，宿主从 `WM_CONTEXTMENU` 打开它。菜单由 presenter 在 `&mut Host` 借用之外展示；原生 presenter 使用 `TrackPopupMenuEx(TPM_RETURNCMD)`，返回的命令只经运行时命令路径派发一次，不产生第二次 `WM_COMMAND`。节点级上下文菜单、托盘（#198）与应用级命令（#197）不在本页范围。

**状态：可见自动驱动完成（100% 与 150%）；隐藏 fake presenter 测试只是部分证据，原生弹出菜单的绘制与选择以可见运行为准。**

验收日期：2026-11-24。功能基线为 `main` 的 `4cc0e12`。可见运行使用仓库内的 `crates/windows/examples/menu_host.rs`；右键与键盘（Shift+F10、Apps）由真实 `SendInput` 注入，截图与窗口查询脚本放在仓库外。

## 环境

| 项目 | 100% | 150% |
| --- | --- | --- |
| 显示缩放 | 96 DPI | 144 DPI |
| 窗口位置 | 屏幕 (52,52) | 屏幕 (190,190) |
| 外框（`GetWindowRect`） | 576×379 | 862×566 |
| 客户区（`GetClientRect`） | 560×320 | 840×480 |
| 客户区原点 | (60,103) | (201,265) |
| 右键弹出（类名 `#32768`） | (400,300) 158×81 | (600,400) 222×122 |
| 键盘弹出（类名 `#32768`） | (340,263) 158×81 | (621,505) 222×122 |

示例窗口的目录尺寸是 560×320 逻辑像素，两个缩放都保持这一逻辑客户区。键盘打开的坐标是客户区中心；右键打开的坐标是点击的物理像素。弹出菜单按运行 DPI 绘制，150% 的菜单宽高约为 100% 的 1.5 倍。

## 方法

- `WindowCatalog::context_menu` 为目录窗口附加一份窗口级菜单声明。它与菜单栏共享命令处理器，但两者的命令 id 不能重复；上下文菜单不声明快捷键，也不进入 `HACCEL`。未知窗口、重复声明、重复命令 id、快捷键与超限都在 `Runtime::declare_windows` 完成时校验。
- `Runtime::window_context_menu` 只为存活的 activation 返回声明；命令处理器注册与 `dispatch_command` 都同时认菜单栏与上下文菜单的命令 id。宿主把选择映射回稳定声明 id 后，以 `CommandSource::ContextMenu` 派发一次。
- `WM_CONTEXTMENU` 的 `lParam` 是屏幕坐标；键盘调用携带 `(-1,-1)`，宿主换成客户区中心的屏幕坐标。右键与键盘因此进入同一份声明。
- 窗口过程先把 presenter 与声明从 `Host` 取出，再在**不持有宿主借用**的情况下调用 presenter。菜单自己的模态循环期间到达的唤醒与窗口消息仍可重入窗口过程；关闭 owner 的 `WM_CLOSE` 也可以同步完成。返回后宿主重新借出，把 presenter 放回，并且只在窗口仍接受输入时派发命令；弹窗期间关闭、隐藏或进入关闭确认的 owner 丢弃返回值。
- 原生 presenter 在 `SetForegroundWindow(owner)` 后调用 `TrackPopupMenuEx(TPM_RETURNCMD | TPM_RIGHTBUTTON)`，随后投递 `WM_NULL` 完成前台清理。`TPM_RETURNCMD` 直接返回选中命令的 id，系统不再发送第二次 `WM_COMMAND`。
- 弹出菜单打开期间，宿主跳过 `WM_INITMENU`/`WM_INITMENUPOPUP` 的菜单栏状态应用。上下文菜单与菜单栏各自从 1 编号命令 id；若按数字 id 查菜单栏映射，两个菜单的同号命令会互相改写 enabled/checked。上下文菜单因此保持声明默认状态，菜单栏的状态回调仍只作用于自己的 `HMENU`。
- 弹出菜单使用 `CreatePopupMenu` 构建；菜单栏继续使用 `CreateMenu`。可见运行发现把菜单栏句柄交给 `TrackPopupMenuEx` 时，本机 Windows 11 只显示 6px 宽的空白窗口条；弹出菜单句柄恢复完整标签与命中。两种构建共享同一份命令 id 映射与声明项遍历。
- 可见宿主在创建时安装原生 presenter；隐藏宿主默认没有 presenter，CI 不会打开真实菜单。测试与嵌入方可以用 `Host::set_context_menu_presenter` 注入 fake，记录请求并返回命令。
- 可见证据在 note 输入中键入 `abc`，右键打开菜单并选择 `Clear note`；再次用 Shift+F10 打开同一菜单并选择 `Copy`。150% 下另用 Apps 键确认第三键盘入口。

## 隐藏测试

`zenui-windows` 的 159 个库测试包含：

- `context_menu_pointer_and_keyboard_pass_the_same_declaration`：fake presenter 记录两次打开；右键坐标原样到达，键盘打开是客户区中心，两份请求的 `MenuSpec` 相同；每次打开只派发一次命令。
- `cancelling_the_context_menu_dispatches_nothing_and_restores_the_presenter`：返回 `None` 不派发；presenter 归还宿主并可再次打开。
- `a_worker_wake_during_the_open_menu_is_delivered`：presenter 在“弹窗期间”同步投递宿主唤醒，任务 continuation 排队的窗口请求在菜单关闭前完成并创建新 HWND。
- `closing_the_owner_during_the_open_menu_discards_the_command`：presenter 同步派发 owner 的 `WM_CLOSE`；返回的命令被丢弃，窗口与 activation 关闭，presenter 归还。
- `a_panicking_context_menu_presenter_is_contained_and_restored`：panic 转成 `HostError::ContextMenuPresenter`，不穿过窗口过程；presenter 归还，下一次打开成功派发。
- `an_undeclared_presenter_command_is_a_host_error`：presenter 返回未声明 id 时作为 `HostError::Command(UnknownCommand)` 上报，不运行处理器，presenter 归还。
- `context_popup_ignores_menu_bar_state_with_colliding_numeric_ids`：窗口同时声明菜单栏与上下文菜单；菜单栏命令 1 禁用、命令 2 勾选，弹出菜单加载时向 owner 发送 `WM_INITMENUPOPUP`，弹出菜单的同号项保持 enabled/unchecked。
- `a_hidden_window_does_not_open_the_context_menu`、`a_hidden_host_never_installs_the_native_context_menu_presenter`、`a_window_without_a_context_menu_leaves_the_presenter_installed`：隐藏窗口、隐藏宿主与未声明窗口都不打开菜单，也不消耗 presenter。
- `menus::tests::a_popup_menu_keeps_declared_items_and_command_ids`：`CreatePopupMenu` 构建的菜单保留命令、分隔线与子菜单，命令 id 与前序一致，注销后不再是菜单。

根包的 8 个 `menu_context` 集成测试覆盖：未知/重复声明、菜单栏与上下文菜单共享命令 id、上下文菜单拒绝快捷键、65535 项上限、上下文菜单只对存活 activation 可读、命令注册与 `ContextMenu` 来源派发、缺失处理器导致打开失败、声明顺序无关。全部 159 个 `zenui-windows` 库测试与 workspace 758 个测试在 2026-11-24 通过。

## 可见读数

| 缩放 | 初始 | 右键弹出 | 清空后 | Shift+F10 弹出 | 复制后 |
| --- | --- | --- | --- | --- | --- |
| 100% | [100-initial.png](证据/host-context-menu/100-initial.png) | [100-popup.png](证据/host-context-menu/100-popup.png) | [100-cleared.png](证据/host-context-menu/100-cleared.png) | [100-keyboard-popup.png](证据/host-context-menu/100-keyboard-popup.png) | [100-copied.png](证据/host-context-menu/100-copied.png) |
| 150% | [150-initial.png](证据/host-context-menu/150-initial.png) | [150-popup.png](证据/host-context-menu/150-popup.png) | [150-cleared.png](证据/host-context-menu/150-cleared.png) | [150-keyboard-popup.png](证据/host-context-menu/150-keyboard-popup.png) | [150-copied.png](证据/host-context-menu/150-copied.png) |

150% 另有 [150-apps-popup.png](证据/host-context-menu/150-apps-popup.png)：Apps 键打开同一菜单。两个缩放都显示 `Copy / Paste / Clear note`；选择 `Clear note` 后状态文本从 `Ready` 变为 `Note cleared` 且输入清空，键盘选择 `Copy` 后变为 `Copied`。完整读数见 [`states.txt`](证据/host-context-menu/states.txt)。

## 判据对照

| #196 要求 | 状态 | 证据 |
| --- | --- | --- |
| 右键与 Apps/Shift+F10 到达同一菜单 | 通过 | 可见图；`context_menu_pointer_and_keyboard_pass_the_same_declaration` |
| `TPM_RETURNCMD` 恰好一次派发，没有第二次 `WM_COMMAND` | 通过 | 原生 presenter 使用 `TPM_RETURNCMD`；可见每次选择只改变一次状态；fake presenter 测试每次打开一次派发 |
| 弹窗期间的 worker 唤醒仍送达宿主 | 通过 | `a_worker_wake_during_the_open_menu_is_delivered` |
| 弹窗期间关闭 owner 取消且不派发 stale 命令 | 通过 | `closing_the_owner_during_the_open_menu_discards_the_command` |
| fake presenter 记录菜单并返回命令；CI 不打开原生菜单 | 通过 | 10 个宿主测试；隐藏宿主无原生 presenter |
| 可见验收采集 `#32768` 弹出菜单与状态变化 | 通过 | 十一张可见图；`states.txt` 记录类名与矩形 |
| 目录拒绝共享命令 id、快捷键与超限 | 通过 | `menu_context` 的 8 个集成测试 |
| `cargo fmt --all -- --check`、workspace clippy、workspace tests | 通过 | PR 的 12 个必需检查与本地门禁 |

## 边界

- **节点级菜单：** 本切片只有窗口级声明；命令没有目标节点，`CommandEvent::target()` 为 `None`。按节点筛选的上下文菜单不在本工单。
- **菜单状态：** 上下文菜单使用 `MenuCommand` 的声明默认 enabled/checked；`on_menu_state` 的每次打开求值仍只作用于原生菜单栏。上下文菜单的命令派发不经过菜单栏的禁用门。
- **快捷键：** 上下文菜单不声明快捷键，也不创建 `HACCEL`；同一命令 id 不能同时出现在菜单栏与上下文菜单。
- **采集：** 完整窗口图来自 `PrintWindow(PW_RENDERFULLCONTENT)`，右/下的黑色条是采集产物；弹出菜单用 DPI-aware 屏幕读取裁剪，菜单圆角外带有一圈窗口背景或桌面像素，不作为应用绘制。隐藏测试的 fake presenter 不证明原生菜单的绘制、命中或 `TPM_RETURNCMD` 的系统行为。
- **创建入口：** 菜单栏用 `CreateMenu`，`TrackPopupMenuEx` 用 `CreatePopupMenu`。可见运行记录了错误入口的 6px 空白窗口条和修正后的完整菜单；隐藏测试只校验弹出菜单的声明项、id 与注销。
- **单屏：** 只在单屏 2560×1600 上运行；100% 是临时切换，结束时恢复 150%。不声明多显示器、混合 DPI 或触摸输入。

## 后续状态

本页关闭 #196 的窗口级上下文菜单声明、`WM_CONTEXTMENU` 路由、presenter seam 与可见证据。#197 的应用级命令契约见 [SDK 窗口与任务](../SDK/窗口与任务.md) 与 [验证重点](验证重点.md)，#198 的应用托盘见 [stage-three 托盘验收](stage-three-托盘-验收.md)；节点级上下文菜单与 #200 的契约汇总仍由后续工单交付。
