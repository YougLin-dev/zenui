# Stage-three 原生菜单栏与菜单状态验收（实机）

本页记录 #193 与 #194：目录声明的静态菜单在 Windows 宿主上构建为每窗口一个原生菜单栏，参与非客户区尺寸计算；菜单选择经 `WM_COMMAND`（`HIWORD=0`、`lParam=0`）回到运行时命令路径；每次打开菜单时，应用以只读回调提交的 enabled/checked 状态经 `WM_INITMENU`/`WM_INITMENUPOPUP` 写回原生项。快捷键（#195）、上下文菜单（#196）与托盘（#198）不在本页范围。

**状态：可见自动驱动完成（100% 与 150%）；隐藏 `GetMenu`/`GetMenuState` 测试与运行时集成测试只是部分证据。**

验收日期：2026-11-24。功能基线为 `main` 的 `02f7e1a`。可见运行使用仓库内的 `crates/windows/examples/menu_host.rs`；菜单由真实 `SendInput` 打开和选择，截图与窗口查询脚本放在仓库外。

## 环境

| 项目 | 100% | 150% |
| --- | --- | --- |
| 显示缩放 | 96 DPI | 144 DPI |
| 窗口位置 | 屏幕 (208,208) | 屏幕 (114,114) |
| 外框（`GetWindowRect`） | 576×379 | 862×566 |
| 客户区（`GetClientRect`） | 560×320 | 840×480 |
| 外框减客户区 | 16×59 | 22×86 |
| 菜单栏 | 560×19 | 840×29 |
| 菜单栏项 | File 33×19；View 41×19 | File 52×29；View 63×29 |

示例窗口的目录尺寸是 560×320 逻辑像素；两种缩放都保持这一逻辑客户区。外框包含原生标题栏与菜单栏，尺寸来自 `AdjustWindowRectExForDpi`；菜单栏与菜单项矩形来自 `GetMenuBarInfo(OBJID_MENU)`。

## 方法

- `WindowCatalog::menu` 的声明由 `Runtime::window_menu` 交给宿主。宿主在 `CreateWindowExW` 前构建 `HMENU`，把句柄作为窗口菜单传入；创建成功后菜单归窗口所有并由 `DestroyWindow` 注销，创建失败时由 `NativeMenu` 注销。`NativeMenu` 保留命令 id 到 `MenuCommand` 的映射供 `WM_COMMAND` 使用。
- 创建、`Host::resize` 与 `WM_GETDPISCALEDSIZE` 三处非客户区计算都用菜单存在性作为 `bMenu`；`WM_GETDPISCALEDSIZE` 直接查询 `GetMenu`，不在 DPI 计算中借出宿主。
- 命令 id 按声明前序从 1 分配，分隔线与子菜单不占 id。`WM_COMMAND` 只接受 `HIWORD=0` 且 `lParam=0`；宿主把数字 id 映射回 `MenuCommand`，先重新求值菜单状态，禁用命令不进入派发，再调用 `Runtime::dispatch_command(..., CommandSource::MenuBar)`。隐藏、待决关闭与已关闭窗口在宿主和运行时两层都被拒绝。
- 窗口在 factory 中用 `WindowCtx::on_menu_state` 登记至多一个只读回调。`Runtime::menu_state` 在只读作用域内调用回调：Signal 写与 Store 事务被拒绝并返回 `MenuStateError::Context`/`Write`，回调 panic 被 contain 为 `HandlerPanicked`，模型保持已提交值。没有回调时使用 `MenuCommand::default_state` 的声明默认值。
- 宿主在 `WM_INITMENU` 与 `WM_INITMENUPOPUP` 上先求值该菜单全部直接命令项，再统一调用 `EnableMenuItem`/`CheckMenuItem`；分隔线与子菜单项跳过，系统菜单跳过，任一回调失败时原生菜单保持原状并把 typed error 交给消息循环。菜单打开期间不轮询状态，下一次打开或再次进入弹出菜单时重新求值。
- 可见证据用 `SendInput` 鼠标点击菜单栏打开 File/View，按 Down、Enter 选择菜单项；完整窗口用 `PrintWindow(PW_RENDERFULLCONTENT)` 采集，打开中的弹出菜单用 DPI-aware 的屏幕读取窗口区域。

## 隐藏测试

`zenui-windows` 的 140 个库测试包含：

- `hidden_window_reports_the_declared_menu_bar`：`GetMenu` 非空；三级标签、分隔线、命令 id 1/2/3、`MF_GRAYED` 与 `MF_CHECKED` 默认状态来自声明。
- `menu_commands_dispatch_once_and_reject_non_menu_messages`：`HIWORD=0` 的命令恰好运行一次；`lParam!=0`、未知 id 与分隔线 id 0 都不派发。`HIWORD=1` 的快捷键形式由 [快捷键验收](stage-three-快捷键-验收.md) 覆盖。
- `menu_commands_are_ignored_while_hidden_or_closing`：隐藏窗口与等待关闭确认的父窗口不运行命令。
- `entering_the_menu_loop_cancels_composition_and_pointer_transients`：`WM_ENTERMENULOOP` 取消 IME 组合，清除 pointer/hover/pressed/thumb 拖动与 leave 跟踪，并释放延迟的捕获。
- `menu_bar_keeps_the_logical_client_size_at_96_and_144_dpi`：`resize` 在运行机实际 DPI 下保持精确逻辑客户区；`WM_GETDPISCALEDSIZE` 在 96/144 下返回含菜单栏的外框，且高于无菜单外框。
- `menu_handle_dies_with_the_window`：窗口关闭后 `IsMenu` 为 false；已挂接菜单由 `DestroyWindow` 注销。
- `an_empty_menu_declaration_creates_no_native_menu_bar`：空声明不创建原生菜单栏，也不占非客户区。
- `menu_init_messages_reapply_committed_state`：初次与状态改变后，`WM_INITMENU` 与 `WM_INITMENUPOPUP` 都把 enabled/checked 应用到顶层命令与子菜单项；状态改变在两次弹出菜单之间可见；禁用项经 `WM_COMMAND` 不派发，恢复可用后派发一次。
- `hidden_windows_leave_menu_items_unchanged`：状态改变后隐藏窗口，再用 `WM_INITMENU`/`WM_INITMENUPOPUP` 以新状态求值也不会改写原生项；重新显示后同一状态才生效。
- `menu_state_errors_leave_the_menu_unchanged`：回调尝试 Signal 写时以 `HostError::MenuState` 暴露 `Signal::set`，`GetMenuState` 保持声明默认值。
- `reopening_rebuilds_the_menu_and_reregisters_state`：关闭后旧 `HMENU` 随窗口销毁；重新显示创建新 HWND 与新菜单，新 activation 登记的状态回调继续驱动勾选与启用。
- `menus::tests::an_unassigned_menu_is_destroyed_on_drop`：窗口没有接收菜单时由构建值注销。

根包的 17 个 `menu_commands` 集成测试包含：

- `the_window_menu_is_readable_for_a_live_window_only`：存活窗口返回声明，关闭、无菜单与跨 Runtime 的上下文返回 `None`。
- `menu_state_reads_committed_state_and_rejects_writes`：回调读取已提交的 Signal 与 Store；Signal 写返回 `MenuStateError::Context { operation: "Signal::set", phase: Idle }`，Store 事务返回 `MenuStateError::Write(Transaction::finish)`，两者都不改动模型；panic 被 contain；后续查询仍可用。
- `menu_state_registration_is_per_activation_and_readable`：同一 activation 的第二个回调和嵌套/迟到的命令处理器一样被拒绝；关闭后查询返回 `Stale`；重新显示且未登记时返回 `Ok(None)`，第三次 activation 重新登记后恢复；跨 Runtime 返回 `WrongRuntime`。

全部 140 个 `zenui-windows` 库测试、17 个 `menu_commands` 集成测试与 workspace 724 个测试在 2026-11-24 通过。

## 可见读数

| 缩放 | 初始完整窗口 | View 已勾选 | View 未勾选 | File 可用 | File 已禁用 | Clear 之后 |
| --- | --- | --- | --- | --- | --- | --- |
| 100% | [100-window.png](证据/host-menu/100-window.png) | [100-view-checked.png](证据/host-menu/100-view-checked.png) | [100-view-unchecked.png](证据/host-menu/100-view-unchecked.png) | [100-file-enabled.png](证据/host-menu/100-file-enabled.png) | [100-file-disabled.png](证据/host-menu/100-file-disabled.png) | [100-cleared.png](证据/host-menu/100-cleared.png) |
| 150% | [150-window.png](证据/host-menu/150-window.png) | [150-view-checked.png](证据/host-menu/150-view-checked.png) | [150-view-unchecked.png](证据/host-menu/150-view-unchecked.png) | [150-file-enabled.png](证据/host-menu/150-file-enabled.png) | [150-file-disabled.png](证据/host-menu/150-file-disabled.png) | [150-cleared.png](证据/host-menu/150-cleared.png) |

初始客户区显示 `Ready`；View > Show status 带勾选，File > Clear 可用。选择 Show status 后文本变为 `Status hidden`，再打开 View 时勾选消失。选择 Clear 后文本变为 `Cleared`；再打开 File 时 Clear 变灰，点击该禁用项不执行命令，文本保持 `Cleared`。完整读数见 [`states.txt`](证据/host-menu/states.txt)。

## 判据对照

| 要求 | 状态 | 证据 |
| --- | --- | --- |
| #193：隐藏 HWND 的 `GetMenu` 报告声明的标签与 id | 通过 | `hidden_window_reports_the_declared_menu_bar`；`states.txt` |
| #193：96/144 DPI 下 resize 后逻辑客户区精确 | 通过 | `menu_bar_keeps_the_logical_client_size_at_96_and_144_dpi`；上表 |
| #193：`WM_COMMAND`（`HIWORD=0`）恰好派发一次；`lParam!=0` 与未知 id 不派发 | 通过 | `menu_commands_dispatch_once_and_reject_non_menu_messages` |
| #193：过期、隐藏与待决关闭窗口不运行 | 通过 | `menu_commands_are_ignored_while_hidden_or_closing`；运行时的 stale 测试 |
| #193：`WM_ENTERMENULOOP` 取消 IME 组合与指针瞬态 | 通过 | `entering_the_menu_loop_cancels_composition_and_pointer_transients` |
| #193：`HMENU` 句柄随窗口释放；本工单未创建 `HACCEL`（#195 已接入） | 通过 | `menu_handle_dies_with_the_window`；`reopening_rebuilds_the_menu_and_reregisters_state` |
| #194：enabled/checked 跟随共享状态，并在两种 init 消息上重新查询 | 通过 | `menu_init_messages_reapply_committed_state`；六组可见图 |
| #194：状态在两次弹出菜单之间改变可被观察 | 通过 | 同上测试；View 勾选与 File 禁用图 |
| #194：回调写 Signal/Store 得到 typed error 且状态不变 | 通过 | `menu_state_reads_committed_state_and_rejects_writes`；`menu_state_errors_leave_the_menu_unchanged` |
| #194：禁用项不能经菜单派发；隐藏/过期窗口保持菜单项不变 | 通过 | `menu_init_messages_reapply_committed_state`；`hidden_windows_leave_menu_items_unchanged`；禁用点击后的 `100-cleared.png`；运行时 `Stale` 测试 |
| #194：关闭/重新显示丢弃旧状态回调并重新登记 | 通过 | `menu_state_registration_is_per_activation_and_readable`；`reopening_rebuilds_the_menu_and_reregisters_state` |
| #194：可见 `menu_host` 展示勾选切换与拒绝的禁用项 | 通过 | 六组可见图；`states.txt` |
| `cargo fmt --all -- --check`、workspace clippy、workspace tests | 通过 | PR 的 12 个必需检查与本地门禁 |

## 边界

- **采集：** 完整窗口图来自 `PrintWindow(PW_RENDERFULLCONTENT)`；它不绘制外框的缩放宽边，图片右/下的黑色条是采集产物。打开中的弹出菜单由图外的 DPI-aware 屏幕读取获得，图片左/下的黑色条是窗口不可见缩放宽边的采集产物；两者都不证明 DWM 合成时序或被其它窗口遮挡时的表现。
- **菜单状态时机：** 状态只在菜单即将打开时查询并写入原生项；菜单保持打开时不会轮询共享状态。Windows 菜单是模态循环，这一时机与系统行为一致，本页不声明打开中的实时刷新。
- **失败处理：** 状态回调的 Signal 写、Store 事务或 panic 在宿主中升级为 `HostError::MenuState`；返回错误时该次菜单初始化保持原状。运行时 API 本身只返回 typed error，调用方可以自行决定是否继续。
- **后续交付：** 快捷键表与 `HACCEL` 已由 #195 接入，`HIWORD=1` 的快捷键路径与快捷键验收见 [stage-three 快捷键验收](stage-three-快捷键-验收.md)；窗口级上下文菜单已由 #196 接入，见 [stage-three 上下文菜单验收](stage-three-上下文菜单-验收.md)；应用级命令（#197）与应用托盘（#198）已接入，托盘证据见 [stage-three 托盘验收](stage-three-托盘-验收.md)。
- **菜单结构：** 可见示例与隐藏测试覆盖一层子菜单；`NativeMenu` 的命令映射支持更深层，但多层弹出菜单的系统消息顺序没有可见证据。
- **菜单栏高度：** 尺寸计算使用 `AdjustWindowRectExForDpi` 的单行菜单栏高度；标签换行成多行菜单栏时系统实际高度可能更大，本页不覆盖该情形。
- **系统菜单：** `WM_INITMENUPOPUP` 的高字标记系统菜单时宿主跳过状态应用；应用声明中不存在系统菜单项。
- **单屏：** 只在单屏 2560×1600 上运行；100% 是临时切换，结束时恢复 150%。不声明多显示器或混合 DPI。

## 后续状态

本页关闭 #193 的原生菜单栏、命令路由与可见证据，以及 #194 的动态启用/勾选状态。#195 的快捷键见 [stage-three 快捷键验收](stage-three-快捷键-验收.md)，#196 的窗口级上下文菜单见 [stage-three 上下文菜单验收](stage-three-上下文菜单-验收.md)，#198 的应用托盘见 [stage-three 托盘验收](stage-three-托盘-验收.md)；#200 的契约汇总仍由后续工单交付。
