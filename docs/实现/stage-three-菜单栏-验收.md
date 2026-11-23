# Stage-three 原生菜单栏验收（实机）

本页记录 #193：目录声明的静态菜单在 Windows 宿主上构建为每窗口一个原生菜单栏，参与非客户区尺寸计算；菜单选择经 `WM_COMMAND`（`HIWORD=0`、`lParam=0`）回到运行时命令路径。菜单状态查询（#194）、快捷键（#195）、上下文菜单（#196）与托盘（#198）不在本页范围。

**状态：可见自动驱动完成（100% 与 150%）；隐藏 `GetMenu` 测试只是部分证据。**

验收日期：2026-11-24。功能基线为 `main` 的 `dc00606`。可见运行使用仓库内的 `crates/windows/examples/menu_host.rs`；菜单由真实 `SendInput` 打开和选择，截图与窗口查询脚本放在仓库外。

## 环境

| 项目 | 100% | 150% |
| --- | --- | --- |
| 显示缩放 | 96 DPI | 144 DPI |
| 窗口位置 | 屏幕 (300,150) | 同左 |
| 外框（`GetWindowRect`） | 576×379 | 862×566 |
| 客户区（`GetClientRect`） | 560×320 | 840×480 |
| 外框减客户区 | 16×59 | 22×86 |
| 菜单栏 | File / View | 同左 |

示例窗口的目录尺寸是 560×320 逻辑像素；两种缩放都保持这一逻辑客户区。外框包含原生标题栏与菜单栏，尺寸来自 `AdjustWindowRectExForDpi`。

## 方法

- `WindowCatalog::menu` 的声明由 `Runtime::window_menu` 交给宿主。宿主在 `CreateWindowExW` 前构建 `HMENU`，把句柄作为窗口菜单传入；创建成功后菜单归窗口所有并由 `DestroyWindow` 注销，创建失败时由 `NativeMenu` 注销。`NativeMenu` 保留命令 id 映射供 `WM_COMMAND` 使用。
- 创建、`Host::resize` 与 `WM_GETDPISCALEDSIZE` 三处非客户区计算都用菜单存在性作为 `bMenu`；`WM_GETDPISCALEDSIZE` 直接查询 `GetMenu`，不在 DPI 计算中借出宿主。
- 命令 id 按声明前序从 1 分配，分隔线与子菜单不占 id。`WM_COMMAND` 只接受 `HIWORD=0` 且 `lParam=0`；宿主把数字 id 映射回静态声明 id，再调用 `Runtime::dispatch_command(..., CommandSource::MenuBar)`。隐藏、待决关闭与已关闭窗口在宿主和运行时两层都被拒绝。
- 可见证据由 `SendInput` 注入 `Alt+V` 打开 View 菜单，再按 Enter 选择 `Show status`；窗口用 `PrintWindow(PW_RENDERFULLCONTENT)` 采集。打开中的弹出菜单是独立原生窗口，因此用 DPI-aware 的屏幕读取窗口可见区域，不采集窗口外的桌面内容。

## 隐藏测试

`zenui-windows` 的隐藏窗口测试覆盖：

- `hidden_window_reports_the_declared_menu_bar`：`GetMenu` 非空；三级标签、分隔线、命令 id 1/2/3、`MF_GRAYED` 与 `MF_CHECKED` 默认状态来自声明。
- `menu_commands_dispatch_once_and_reject_non_menu_messages`：`HIWORD=0` 的命令恰好运行一次；`lParam!=0`、`HIWORD=1`、未知 id 与分隔线 id 0 都不派发。
- `menu_commands_are_ignored_while_hidden_or_closing`：隐藏窗口与等待关闭确认的父窗口不运行命令。
- `entering_the_menu_loop_cancels_composition_and_pointer_transients`：`WM_ENTERMENULOOP` 取消 IME 组合，清除 pointer/hover/pressed/thumb 拖动与 leave 跟踪，并释放延迟的捕获。
- `menu_bar_keeps_the_logical_client_size_at_96_and_144_dpi`：`resize` 在运行机实际 DPI 下保持精确逻辑客户区；`WM_GETDPISCALEDSIZE` 在 96/144 下返回含菜单栏的外框，且高于无菜单外框。
- `menu_handle_dies_with_the_window`：窗口关闭后 `IsMenu` 为 false；已挂接菜单由 `DestroyWindow` 注销。
- `an_empty_menu_declaration_creates_no_native_menu_bar`：空声明不创建原生菜单栏，也不占非客户区。
- `menus::tests::an_unassigned_menu_is_destroyed_on_drop`：窗口没有接收菜单时由构建值注销。

根包的 `the_window_menu_is_readable_for_a_live_window_only` 覆盖声明读取：存活窗口返回菜单，关闭、无菜单与跨 Runtime 的上下文返回 `None`。全部 136 个 `zenui-windows` 库测试与 workspace 测试都通过。

## 可见读数

| 缩放 | Ready（完整窗口） | View 菜单 | 命令执行后 |
| --- | --- | --- | --- |
| 100% | [100-window.png](证据/host-menu/100-window.png) | [100-view-menu.png](证据/host-menu/100-view-menu.png) | [100-command.png](证据/host-menu/100-command.png) |
| 150% | [150-window.png](证据/host-menu/150-window.png) | [150-view-menu.png](证据/host-menu/150-view-menu.png) | [150-command.png](证据/host-menu/150-command.png) |

两个缩放都显示 `File` 与 `View` 菜单栏、`Ready` 初始文本；打开 View 后弹出 `Show status`；执行后客户区文本变为 `Command ran`。完整读数见 [`states.txt`](证据/host-menu/states.txt)。

## 判据对照

| #193 要求 | 状态 | 证据 |
| --- | --- | --- |
| 隐藏 HWND 的 `GetMenu` 报告声明的标签与 id | 通过 | `hidden_window_reports_the_declared_menu_bar`；`states.txt` |
| 96/144 DPI 下 resize 后逻辑客户区精确 | 通过 | `menu_bar_keeps_the_logical_client_size_at_96_and_144_dpi`；上表 |
| `WM_COMMAND`（`HIWORD=0`）恰好派发一次；`lParam!=0` 与未知 id 不派发 | 通过 | `menu_commands_dispatch_once_and_reject_non_menu_messages` |
| 过期、隐藏与待决关闭窗口不运行 | 通过 | `menu_commands_are_ignored_while_hidden_or_closing`；运行时的 stale 测试 |
| `WM_ENTERMENULOOP` 取消 IME 组合与指针瞬态 | 通过 | `entering_the_menu_loop_cancels_composition_and_pointer_transients` |
| `HMENU`/`HACCEL` 句柄随窗口释放 | 通过 | `menu_handle_dies_with_the_window`；`HMENU` 由 `DestroyWindow` 注销，本工单未创建 `HACCEL`（见边界） |
| 可见 `menu_host` 展示菜单栏与命令，完整窗口可采集 | 通过 | 六张截图；`SendInput` 菜单选择 |
| `cargo fmt --all -- --check`、workspace clippy、workspace tests | 通过 | PR 的 12 个必需检查与本地门禁 |

## 边界

- **采集：** 完整窗口图来自 `PrintWindow(PW_RENDERFULLCONTENT)`；它不绘制外框的缩放宽边，图片四周的黑色条是采集产物，不是应用绘制。打开中的弹出菜单由图外的 DPI-aware 屏幕读取获得，只裁剪窗口可见区域；它证明原生菜单文本与位置，不证明 DWM 合成时序或被其它窗口遮挡时的表现。
- **尚未交付：** 菜单状态在打开时仍是声明默认值；`WM_INITMENU`/`WM_INITMENUPOPUP` 的动态查询属于 #194。本工单没有创建 `HACCEL`；快捷键表、其注销与 `HIWORD=1` 的命令路径属于 #195，本页不声明快捷键可用。上下文菜单与托盘不在本工单。
- **菜单栏高度：** 尺寸计算使用 `AdjustWindowRectExForDpi` 的单行菜单栏高度；标签换行成多行菜单栏时系统实际高度可能更大，本页不覆盖该情形。
- **静态默认状态：** 可见示例只声明可用的普通命令；禁用项与勾选项由隐藏测试通过 `GetMenuState` 验证，不在可见图内。
- **窗口状态：** 隐藏与待决关闭在真实 HWND 上验证；"过期"由运行时命令路径的 stale 测试覆盖，已销毁 HWND 不再接收消息。
- **单屏：** 只在单屏 2560×1600 上运行；100% 是临时切换，结束时恢复 150%。不声明多显示器或混合 DPI。

## 后续状态

本页关闭 #193 的原生菜单栏、命令路由与可见证据；#194 的动态启用/勾选状态、#195 的快捷键、#196 的上下文菜单、#198 的托盘与 #200 的契约汇总仍由后续工单交付。
