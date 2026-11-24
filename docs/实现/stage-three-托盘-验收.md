# Stage-three 应用托盘验收（实机）

本页记录 #198：应用声明一个托盘图标与右键菜单，宿主用独立的隐藏托盘 HWND 接收通知区域回调与 Explorer 的 `TaskbarCreated` 广播。左键或键盘激活恢复窗口，右键菜单在宿主借用之外展示，选中命令以 `CommandSource::Tray` 派发一次。节点级上下文菜单、托盘独存与最小化到托盘不在本页范围。

**状态：可见自动驱动完成（Windows 11 单屏 150%）；隐藏 fake presenter 测试只是部分证据，系统 tooltip、原生菜单绘制与 Explorer 重建以可见运行为准。**

验收日期：2026-11-24。功能基线为 `main` 的 `3290780`。可见运行使用仓库内的 `crates/windows/examples/tray_host.rs`；点击、菜单选择、tooltip 与截图脚本放在仓库外，未修改系统的“始终显示”设置。

## 环境

| 项目 | 读数 |
| --- | --- |
| 显示 | 单屏 2560×1600，150%（144 DPI） |
| 宿主 DPI awareness | per-monitor v2 |
| 示例窗口 | 标题 `ZenUI Tray`，外框 862×566 物理像素，位于 (48,48) |
| 客户区 | 840×480 物理像素 |
| 托盘 HWND | 隐藏顶层 `ZenUIHostWindow`，标题同为 `ZenUI Tray` |
| 通知区域回调 | 私有 `WM_APP + 4`（worker 唤醒是 `WM_APP + 1`） |
| tooltip | 系统绘制，显示 `ZenUI Tray` |
| 右键菜单窗口 | 类名 `#32768`，(1935,1291)-(2182,1413) 物理像素 |

## 方法

- `Runtime::declare_tray(TraySpec)` 在首个窗口打开或根树挂载前声明 tooltip 与右键菜单；每个 Runtime 一次。`TraySpec::validate` 按 UTF-16 单元限制 tooltip 为 127，菜单拒绝重复命令 id、快捷键、超过 65535 项；失败不保留声明。声明后仍可注册应用级处理器，宿主在 `Host::open` 时用 `Runtime::missing_tray_handler` 校验每个菜单命令都有应用级处理器，缺失时返回错误并指出命令 id。
- 宿主创建自己从不显示的顶层托盘 HWND，与可见窗口共用 `ZenUIHostWindow` 类；`HostPointer` 在 `run`/`pump` 期间也给它安装 `GWLP_USERDATA`，菜单展示、窗口销毁与 `without_routing` 的重入抑制覆盖这个 HWND。图标 id 固定为 1，`NOTIFYICONDATAW::hWnd` 指向托盘 HWND。
- 左键与键盘激活从目录存活窗口中选择目标：可见焦点窗口优先，其次 primary，最后任一存活窗口；已经关闭的窗口不会重新打开。目标通过 `Runtime::show` 重新显示并聚焦，已经关闭的窗口不会因托盘点击重新打开。原生显示留给随后的窗口同步阶段，避免在窗口过程里同步派发 `WM_SHOWWINDOW` 的重入。
- 右键回调在托盘 HWND 的窗口过程里处理。宿主先把 presenter 与菜单移出 `Host`，在**不持有宿主借用**的情况下展示；菜单关闭后重新借出，归还 presenter 并以 `CommandSource::Tray` 派发一次应用级命令。菜单期间目标窗口关闭是正常竞态，丢弃 stale 命令；presenter 返回未声明的 id 作为宿主错误上报。presenter 的 add/update/remove/menu 调用都由 `catch_unwind` 包住，panic 转为 `HostError::TrayPresenter`，不穿过窗口过程或 `Drop`。
- 原生 presenter 用 `NIM_ADD` 添加图标并 `NIM_SETVERSION(NOTIFYICON_VERSION_4)`，随后 `NIM_MODIFY` 写入标准 tooltip；`NIM_DELETE` 移除。右键菜单用 `CreatePopupMenu` 构建，`SetForegroundWindow` 后调用 `TrackPopupMenuEx(TPM_RETURNCMD | TPM_RIGHTBUTTON)`，关闭后投递 `WM_NULL` 完成前台清理。
- `TaskbarCreated` 在创建图标前注册；Explorer 重建通知区域时向顶层窗口广播。宿主只在托盘 HWND 收到该消息时重新添加图标与 tooltip，避免每个窗口各添加一次。
- 图标随最后一个窗口关闭移除；窗口之后重新出现时再次添加。`Host` 析构先移除图标再释放窗口；`added` 标志让关闭路径与析构路径都幂等。`Host::set_tray_presenter` 在替换 presenter 时先尽力移除旧图标，再让新 presenter 添加一次。
- 可见宿主在创建时安装原生 presenter；隐藏宿主默认没有 presenter，CI 不触碰真实通知区域。测试与嵌入方通过 `Host::set_tray_presenter` 注入 fake，记录 add/update/remove 与菜单选择。
- 图标句柄来自 `LoadIconW(None, IDI_APPLICATION)` 的共享系统图标，宿主与 presenter 都不调用 `DestroyIcon`。第一切片不提供自定义图标资产、气球通知或托盘独存。

## Windows 11 溢出面板

新图标默认进入通知区域溢出面板，本页没有修改系统的“始终显示”设置。面板收起时 `Shell_NotifyIconGetRect` 返回溢出按钮矩形 (2031,1528)-(2079,1600)，不能据此认为图标在任务栏可见区；展开后同一查询返回图标矩形 (1906,1383)-(1966,1443)，UIA `NotifyItemIcon` 的名称为 ` ZenUI Tray`。tooltip、左键与右键都在溢出面板中由系统转发。Explorer 重建后图标重新进入溢出面板。

## 隐藏测试

`zenui-windows` 的 169 个库测试包含 10 个托盘测试：

- `the_fake_tray_presenter_records_one_add_update_menu_and_delete`：隐藏宿主安装 fake 后恰好一次 add（回调消息是 `WM_APP + 4`，图标非空）与一次 tooltip update；右键回调到达一次菜单请求，坐标原样保留，选中命令以 `CommandSource::Tray` 派发一次并暴露 primary 目标；`Host` 析构移除图标一次。
- `a_hidden_tray_without_a_presenter_does_not_add_an_icon`：隐藏宿主声明托盘但没有 presenter 时不产生 add，也不把图标状态标为已添加。
- `the_tray_callback_routes_only_through_the_tray_hwnd`：把同一回调消息发给主窗口不打开菜单、不派发命令；发给托盘 HWND 才触发。
- `clicking_the_tray_restores_the_primary_window`：隐藏唯一窗口后 `NIN_SELECT` 重新显示并聚焦 primary，不派发命令。
- `taskbar_created_re_adds_and_the_last_close_removes_once`：注册消息非零；模拟广播后再次 add 与 update；最后一个窗口关闭移除一次，`Host` 析构不重复删除。
- `reopening_after_the_last_close_adds_the_icon_again`：关闭最后一个窗口移除图标；重新显示窗口后再次 add 与 update，最终析构再移除一次。
- `the_tray_uses_the_shared_system_icon`：fake 记录的 HICON 在宿主析构后仍是系统共享图标，证明没有 `DestroyIcon`。
- `a_panicking_tray_presenter_is_contained_and_restored`：菜单 panic 转为 `HostError::TrayPresenter`，presenter 归还，下一次打开成功派发。
- `a_tray_menu_cannot_return_an_undeclared_command`：未声明 id 作为 `HostError::Command(MissingHandler)` 上报，不运行处理器。
- `a_tray_declaration_requires_application_handlers_before_the_host_opens`：缺少应用级处理器时 `Host::open` 失败并指出命令 id。
- 根包 5 个 `tray` 集成测试覆盖：tooltip 的 UTF-16 上限、重复命令、快捷键、65535 项上限与 `AlreadyDeclared`；缺失处理器查询；首个窗口打开后 `RegistrationClosed`；零窗口 `Tray` 来源派发。

2026-11-24 的 workspace 测试为 783 个通过，其中 `zenui-windows` 库测试 169 个。

## 可见读数

| 状态 | 证据 |
| --- | --- |
| 初始窗口（Window visible） | [150-initial.png](证据/host-tray/150-initial.png) |
| 悬停 tooltip（ZenUI Tray） | [150-tooltip.png](证据/host-tray/150-tooltip.png) |
| 右键菜单（Reset status / Hide window / Close） | [150-menu.png](证据/host-tray/150-menu.png) |
| 选择 Reset status 后 | [150-reset.png](证据/host-tray/150-reset.png) |
| 选择 Hide window 后图标仍在溢出面板 | [150-hidden.png](证据/host-tray/150-hidden.png) |
| 左键点击图标恢复窗口 | [150-restored.png](证据/host-tray/150-restored.png) |
| Explorer 重启后重新添加并显示 tooltip | [150-after-explorer-restart.png](证据/host-tray/150-after-explorer-restart.png) |
| 选择 Close 后图标移除 | [150-closed.png](证据/host-tray/150-closed.png) |

完整坐标、回调字与操作序列见 [`states.txt`](证据/host-tray/states.txt)。

## 判据对照

| #198 要求 | 状态 | 证据 |
| --- | --- | --- |
| fake presenter 恰好记录一次 add、tooltip update、菜单派发与 delete；无 presenter 不调用 shell | 通过 | `the_fake_tray_presenter_records_one_add_update_menu_and_delete`、`a_hidden_tray_without_a_presenter_does_not_add_an_icon` |
| 托盘回调消息与 worker 唤醒不同，经自己的 HWND 路由；fake 回调恰好运行一次应用命令 | 通过 | `WM_APP + 4`；`the_tray_callback_routes_only_through_the_tray_hwnd` |
| `TaskbarCreated` 后重新添加；`Host` 析构与最后一个窗口关闭移除图标 | 通过 | 模拟广播测试；可见 Explorer 重启；`taskbar_created_re_adds_and_the_last_close_removes_once` |
| HICON 无泄漏或双重销毁，使用系统图标 | 通过 | 共享 `IDI_APPLICATION`、无 `DestroyIcon`；`the_tray_uses_the_shared_system_icon` |
| 可见 tray_host 显示图标、tooltip、点击恢复与右键菜单，诚实记录 Windows 11 溢出 | 通过 | 八张可见图；`states.txt` 的溢出与 #32768 读数 |
| `cargo fmt --all -- --check`、workspace clippy、workspace tests | 通过 | PR 的 12 个必需检查与本地门禁 |

## 边界

- **托盘独存与最小化到托盘：** 本页记录默认 `QuitOnLastClose` 行为：最后一个窗口关闭会移除图标并结束消息循环。零窗口的托盘独存与隐藏主窗口后继续运行由 #199 的应用会话策略交付，原生最小化窗口可由托盘激活恢复，但最小化按钮自动隐藏不在本切片；见 [stage-three 会话验收](stage-three-会话-验收.md)。
- **图标资产：** 第一切片只用共享系统图标；自定义 `.ico`、主题适配与高 DPI 多尺寸资产不在本工单。
- **菜单状态：** 托盘菜单使用 `MenuCommand` 的声明默认 enabled/checked，不登记 `on_menu_state`，也不进入窗口菜单的 `WM_INITMENU` 状态路径。
- **键盘：** 原生菜单的键盘选择没有作为可见证据。Explorer 溢出面板持有前台时，合成右键可以打开菜单，但合成键盘事件不保证进入应用菜单的模态循环；菜单选中/取消/panic/未声明 id 的命令路径由 fake presenter 覆盖。宿主本身不拦截菜单键盘消息。
- **通知与双击：** 没有 balloon/toast 通知、双击语义、图标动画或上下文菜单外的左键行为。
- **溢出：** Windows 11 决定图标是否位于溢出面板；宿主不查询、不修改“始终显示”设置，也不承诺图标始终可见。
- **缩放：** 本页只在单屏 150% 运行；没有声明 100%、混合 DPI 或多显示器结果。shell 提供的菜单矩形与坐标是物理像素，宿主不自行缩放。
- **采集：** 窗口图来自 `PrintWindow(PW_RENDERFULLCONTENT)`；tooltip 与菜单图来自 DPI-aware 屏幕裁剪，周围桌面与环境图标不是应用绘制。

## 后续状态

本页关闭 #198 的托盘声明、presenter seam、隐藏托盘 HWND、回调路由、点击恢复、右键应用命令、`TaskbarCreated` 恢复与 HICON 生存期。#199 的会话退出与托盘独存策略已由 [stage-three 会话验收](stage-three-会话-验收.md) 交付；节点级上下文菜单与 #200 的契约汇总仍由后续工单处理。
