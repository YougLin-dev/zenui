# Stage-three 应用会话与显式退出验收（实机）

本页记录 #199：应用在运行时启动前声明最后一个窗口关闭后的会话策略，通过 `SessionCtx` 排队窗口恢复与显式退出。默认策略保持“最后窗口关闭即结束宿主会话”，`StayResident` 明确允许零窗口会话继续运行并保留托盘。产品决策见 [ADR-0015](../adr/0015-application-session-and-quit.md)。

**状态：可见自动驱动完成（Windows 11 单屏 150%）；隐藏会话测试只是部分证据，零窗口消息循环与托盘恢复以可见运行为准。**

验收日期：2026-11-24。功能基线为 `main` 的 `33dbb96`。可见运行使用仓库内的 `crates/windows/examples/tray_host.rs`；点击、菜单选择与截图脚本放在仓库外。

## 环境

| 项目 | 读数 |
| --- | --- |
| 显示 | 单屏 2560×1600，150%（144 DPI） |
| 示例窗口 | 标题 `ZenUI Tray`，外框 862×566 物理像素，客户区 840×480 |
| 会话策略 | `SessionPolicy::StayResident` |
| 托盘 | `ZenUI Tray`，菜单 Show window / Reset status / Hide window / Close window / Quit |
| 图标位置 | 收起 overflow 时返回溢出按钮矩形 (2031,1528)-(2079,1600)；展开后 (1906,1383)-(1966,1443) |

## 方法

- `Runtime::declare_session(SessionPolicy)` 在首个窗口打开或根树挂载前声明一次；未声明时使用 `QuitOnLastClose`。`StayResident` 允许零窗口会话，但原生宿主要求同时声明托盘：没有托盘时零窗口会话没有消息入口或可达退出动作，`Host::open` 返回 `HostError::Session(SessionError::TrayRequired)`。
- `Runtime::session()` 返回 Copy 的 `SessionCtx`，可在窗口或应用级命令处理器中捕获。`request_show(key)` 与 `request_quit()` 只接受 `Event` / `Delivery`，进入会话请求队列；未知目录键在排队时返回 `UnknownWindow`。
- 宿主在窗口请求之后调用 `Runtime::apply_session_requests`。`Show` 对已关闭窗口重新运行 factory，对隐藏窗口恢复同一 activation；`Quit` 关闭全部 activation、释放待决 close epoch 与确认窗口，不运行新的关闭处理器，丢弃同一 turn 尚未结算的窗口请求，并让之后的 `show` 返回 `WindowError::State("the application session has ended")`。
- 宿主循环条件从“是否还有窗口”改为 `session_running`：默认策略在最后一个窗口关闭后结束；`StayResident` 即使零窗口也继续。显式退出、或默认策略下最后一个窗口关闭，都通过同一条状态路径结束循环。Runtime 模型仍保留，嵌入方可重新打开窗口并再次运行宿主，除非已经显式退出。
- `StayResident` 零窗口时，worker 唤醒目标回退到托盘 HWND；托盘图标不随最后一个窗口关闭移除。左键或键盘激活优先恢复可见焦点窗口、primary 或任一存活窗口；零窗口时常驻会话恢复 primary。托盘菜单的 Show window 通过 `SessionCtx::request_show` 恢复已关闭窗口，Quit 通过 `SessionCtx::request_quit` 结束会话。
- 可见运行发现 Windows shell 可能用 `SendMessage` 投递托盘回调，外层消息循环因此收不到一条可结算的消息。宿主在每次托盘回调后向托盘 HWND 投递 `WM_NULL`，让恢复/退出后的窗口同步与首次呈现发生；没有该消息时 runtime 已打开窗口，但宿主仍停留在旧窗口集合。
- 窗口目录的 primary 不变量不变：常驻会话可以没有存活窗口，但目录始终声明 primary；它是托盘恢复和宿主首次打开的入口。关闭确认 epoch 仍绑定父 activation，显式退出清理它们但不把 quit 当成 Allow。

## 隐藏测试

`zenui-windows` 的 173 个库测试包含 4 个会话测试：

- `a_resident_session_keeps_the_tray_and_click_reopens_the_last_window`：关闭唯一窗口后进程与图标保留；左键恢复 primary 并产生新 activation；图标不重复添加；析构移除一次。
- `a_resident_session_show_command_reopens_zero_windows`：零窗口时托盘菜单的 app.show 通过 `SessionCtx::request_show` 重新打开 primary，图标保持。
- `a_tray_quit_ends_the_resident_session_and_removes_the_icon_once`：菜单 Quit 关闭全部窗口、结束会话、移除图标一次；后续泵返回会话结束，析构不重复删除。
- `a_resident_session_requires_a_declared_tray`：没有托盘的常驻宿主在 `Host::open` 返回 `TrayRequired`。

根包 6 个 `session` 集成测试覆盖：默认策略与一次性声明、命令处理器显式退出并关闭多窗口、会话 show 重开最后窗口、隐藏窗口 show 保留 activation、未知 key 在排队时拒绝、退出释放待决关闭确认且关闭处理器只运行一次。

2026-11-24 的 workspace 测试为 793 个通过，其中 `zenui-windows` 库测试 173 个；既有 close confirmation、多窗口、托盘和 worker 测试全部保持不变并通过。

## 可见读数

| 状态 | 证据 |
| --- | --- |
| 初始窗口（Window visible） | [150-initial.png](证据/host-session/150-initial.png) |
| 隐藏后左键恢复（Hidden to the tray） | [150-hidden-restored.png](证据/host-session/150-hidden-restored.png) |
| 关闭最后窗口后会话与图标仍存活 | [150-resident-closed.png](证据/host-session/150-resident-closed.png) |
| 零窗口左键恢复（Closed to the tray） | [150-click-reopened.png](证据/host-session/150-click-reopened.png) |
| 常驻托盘菜单 | [150-menu.png](证据/host-session/150-menu.png) |
| 菜单 Show window 恢复（Restored from the tray） | [150-menu-show.png](证据/host-session/150-menu-show.png) |
| 菜单 Quit 后图标移除 | [150-quit.png](证据/host-session/150-quit.png) |

完整坐标、状态变化与 shell 回调读数见 [`states.txt`](证据/host-session/states.txt)。

## 判据对照

| #199 要求 | 状态 | 证据 |
| --- | --- | --- |
| 退出显式且确定；最后窗口关闭遵循文档化策略而不是隐式循环退出 | 通过 | `SessionPolicy` 默认 `QuitOnLastClose`；`Runtime::quit` / `SessionCtx::request_quit`；ADR-0015；根包与宿主测试 |
| 隐藏/最小化到托盘保留会话与图标，托盘恢复隐藏窗口 | 通过 | 可见隐藏→左键恢复；`a_resident_session_keeps_the_tray_and_click_reopens_the_last_window` |
| 既有 close confirmation 与多窗口验收不变 | 通过 | 全部 793 个 workspace 测试；退出测试覆盖待决确认清理 |
| 隐藏与可见证据遵循 ADR-0005；primary 不变量保留 | 通过 | 隐藏测试为部分证据，150% 可见运行；无托盘常驻宿主被拒绝 |
| `cargo fmt --all -- --check`、workspace clippy、workspace tests | 通过 | PR 的 12 个必需检查与本地门禁 |

## 边界

- **平台 session end：** 系统注销/关机、不可撤销的终止与有界 flush 不在本切片；这里只定义应用主动 quit。
- **退出确认：** 显式退出不运行逐窗口 close handler；需要询问未保存内容的应用先完成自己的确认再请求 quit。
- **恢复目标：** 托盘左键恢复目录 primary；应用自定义的窗口选择由托盘菜单调用 `SessionCtx::request_show(key)`。
- **无托盘常驻：** 原生宿主拒绝；headless Runtime 可以声明策略并由调用方驱动循环与退出。
- **缩放与环境：** 只在单屏 150% 运行；未声明 100%、混合 DPI、多显示器或远程桌面结果。
- **采集：** 窗口图来自 `PrintWindow(PW_RENDERFULLCONTENT)`；托盘/菜单图来自 DPI-aware 屏幕裁剪。菜单截图前最小化其它窗口，背景只有桌面壁纸。

## 后续状态

本页关闭 #199 的会话策略、显式退出、零窗口常驻、托盘恢复与图标生命周期。菜单/托盘的契约汇总与 ADR-0013 由 #200 交付；平台 session end 与启动/安装策略仍由后续工单处理。
