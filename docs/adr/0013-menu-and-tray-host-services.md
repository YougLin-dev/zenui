# 菜单与托盘是运行时命令路径上的宿主服务

窗口菜单、快捷键、上下文菜单与应用托盘都需要原生句柄、系统消息和弹窗生命周期，而命令身份、处理器与 enabled/checked 状态属于 Runtime。两边如果各自保存一套命令表，就会出现重复派发、状态分叉或句柄泄漏。

## Decision

**声明与身份**

- 窗口目录通过 `WindowCatalog::menu` 声明菜单栏、通过 `WindowCatalog::context_menu` 声明窗口级上下文菜单；应用托盘通过 `Runtime::declare_tray(TraySpec)` 声明。三份声明都在首个窗口打开或根树挂载之前完成，并由运行时校验。
- `MenuCommand` 的稳定 id 是命令身份。原生数字 id 只在构建它的那份菜单内按声明前序分配，用于 `HMENU`/`HACCEL` 回查；它不是公开身份，也不跨菜单复用。
- 菜单栏与同一窗口的上下文菜单可以共享处理器，但命令 id 不能重复；上下文菜单不声明快捷键。托盘菜单的每个命令都必须有应用级处理器。
- 同一窗口菜单树拒绝重复命令 id、重复归一化快捷键、无效快捷键与超过 65535 项的声明；这些错误在目录或托盘声明期返回，失败不留下半份声明。

**命令路径**

- 菜单栏、快捷键、上下文菜单、辅助技术动作与托盘都进入同一运行时命令路径，分别标记 `CommandSource::{MenuBar, Accelerator, ContextMenu, Accessibility, Tray}`。
- 窗口目标的派发优先运行该窗口注册的处理器；窗口没有同一 id 时回退应用级处理器。`CommandSource::Tray` 从不运行窗口处理器，事件只通过 `CommandEvent::window` 暴露可选目标。
- 原生事件只翻译成一次逻辑命令：菜单栏接受 `WM_COMMAND` 的 `HIWORD=0` 且 `lParam=0`，快捷键接受 `HIWORD=1`；上下文菜单与托盘菜单使用 `TrackPopupMenuEx(TPM_RETURNCMD)`，系统不再发送第二次 `WM_COMMAND`。

**弹窗时状态**

- 窗口在 factory 中通过 `WindowCtx::on_menu_state` 为当前 activation 登记至多一个只读状态回调；关闭窗口时释放，重新打开必须重新登记。
- 宿主在 `WM_INITMENU`、`WM_INITMENUPOPUP` 与命令派发前调用 `Runtime::menu_state` 求值 enabled/checked，再写回原生菜单项或决定是否派发。没有回调时使用 `MenuCommand` 的声明默认值。
- 回调在只读作用域内运行：Signal 写与 Store 事务返回 typed error，panic 被 contain，失败时该次菜单初始化保持原状。
- 上下文菜单在弹出期间保持声明默认状态，不进入菜单栏的 `WM_INITMENU`/`WM_INITMENUPOPUP` 状态路径；两套数字 id 各自从 1 开始，不能按数字 id 互相改写。托盘菜单同样使用声明默认状态，不登记 `on_menu_state`。

**原生所有权**

- 声明了菜单的窗口在创建 HWND 前构建一个菜单栏 `HMENU`，在需要时构建一个 `HACCEL`；创建、非客户区计算与 DPI 换算都读取同一份菜单存在性。菜单与快捷键表随窗口条目释放，创建失败时由构建值立即注销。
- 上下文菜单每次打开时用 `CreatePopupMenu` 构建，菜单关闭后随本次打开释放；它不复用窗口菜单栏句柄。
- 托盘宿主拥有一个从不显示的顶层 `ZenUIHostWindow`，图标 id 固定为 1，`NOTIFYICONDATAW::hWnd` 指向它。presenter 执行 `NIM_ADD`、`NIM_SETVERSION(NOTIFYICON_VERSION_4)`、`NIM_MODIFY` 与 `NIM_DELETE`；图标句柄来自共享的 `IDI_APPLICATION`，宿主与 presenter 都不销毁它。
- `TaskbarCreated` 在创建图标前注册；Explorer 重建通知区域后，宿主只在托盘 HWND 收到广播时重新添加图标与 tooltip。
- presenter 的 add/update/remove/menu 调用都被 contain；panic 转为 `HostError`，不穿过窗口过程或 `Drop`。原生 presenter 可在 `&mut Host` 借用之外展示菜单，返回后重新借出并派发一次命令。

**托盘与会话**

- 默认 `QuitOnLastClose` 会话在最后一个窗口关闭后移除托盘图标；`StayResident` 会话保留图标，零窗口时 worker 唤醒投递到托盘 HWND，左键恢复 primary，直到显式退出。[ADR-0018](0018-application-session-and-quit.md) 固定会话策略与退化条件。
- 零窗口托盘独存要求 `SessionPolicy::StayResident` 且原生宿主声明托盘；没有托盘时 `Host::open` 返回 `HostError::Session(SessionError::TrayRequired)`。
- 原生最小化按钮继续产生普通任务栏最小化窗口，不会自动隐藏到托盘；托盘激活会先恢复最小化窗口再聚焦。进入托盘由应用的 hide/close 请求驱动。

**限制**

- 托盘 tooltip 按 UTF-16 单元限制为 127；不支持自定义图标资产、气球通知、双击语义、跳转列表或任务栏缩略图。
- 不支持节点级上下文菜单、全局热键或第二套托盘/菜单命令路径。
- 原生菜单的键盘选择、tooltip 与 Explorer 重建以可见运行为准；隐藏 presenter/fake 测试只是部分证据。验证只在单屏 100%/150% 进行，多显示器与混合 DPI 仍由 #63 跟踪。

## Considered Options

- **宿主拥有命令注册与处理器**：同一动作会因菜单、快捷键或托盘出现第二套注册表，窗口关闭时难以同步释放。拒绝；宿主只翻译原生事件。
- **把 `TrackPopupMenuEx` 的返回值再交给 `WM_COMMAND`**：会产生第二次派发，或者需要按数字 id 猜测来源。拒绝；`TPM_RETURNCMD` 返回的命令直接进入运行时路径一次。
- **每次打开菜单重建 `HMENU`/`HACCEL`**：句柄生命周期与 activation 脱节，DPI/非客户区计算和快捷键注销都更复杂。拒绝；按窗口构建并随窗口释放。
- **让上下文菜单复用菜单栏状态回调**：两套从 1 开始的数字 id 会互相命中，弹窗还会改变菜单栏状态。拒绝；上下文菜单保持声明默认值。
- **托盘菜单运行窗口处理器**：托盘是应用级对象，没有可靠的窗口目标。拒绝；托盘命令必须注册应用级处理器。
- **默认常驻或自动最小化到托盘**：没有托盘的应用会在零窗口后挂起，也会改变普通最小化语义。拒绝；常驻必须显式声明并在原生宿主要求托盘。
- **在本次交付节点级上下文菜单**：命令需要节点目标、节点生命周期与第二套菜单状态；超出窗口级声明。拒绝；保留为后续工单。

## Consequences

- 菜单、快捷键、上下文菜单与托盘共用一条命令路径和一份声明；新增来源只需提供 `CommandSource` 与目标身份。
- `HMENU`、`HACCEL`、`HICON` 与 `NOTIFYICONDATAW` 的生命周期分别绑定窗口、activation、进程共享图标与托盘宿主，`Host` 析构和会话退出都有确定的清理路径。
- 隐藏宿主不安装原生 presenter，测试通过 fake 记录 add/update/remove/menu；菜单栏、快捷键与上下文菜单记录了 100%/150% 可见行为，托盘记录单屏 150% 的点击、菜单与 Explorer 重建，Windows 11 溢出面板不承诺图标始终可见。
- 托盘独存与最小化恢复由应用会话策略驱动；ADR-0018 记录默认退出、显式退出与零窗口常驻的边界。
