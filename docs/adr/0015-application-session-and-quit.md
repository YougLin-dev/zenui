# 应用会话按显式策略结束，不依赖最后一个窗口的隐式退出

窗口 activation、托盘图标和共享模型有不同的生存期。关闭最后一个窗口目前直接结束原生消息循环，这既不是明确的产品策略，也让“隐藏到托盘”和“关闭到托盘”只能靠调用方猜测。应用需要一个显式的会话策略与退出动作，同时保持 Runtime 的模型、窗口目录和关闭确认协议不变。

## Decision

一个 Runtime 拥有一个应用会话。会话策略必须在首个窗口打开或根树挂载之前通过 `Runtime::declare_session` 声明一次；没有声明时使用 `SessionPolicy::QuitOnLastClose`。

- `QuitOnLastClose` 是默认值：最后一个窗口关闭后，原生宿主结束消息循环。Runtime 不被销毁，嵌入方仍可重新打开目录窗口并再次运行宿主。
- `StayResident` 明确允许会话在零窗口时继续。原生宿主要求同时声明托盘；没有托盘时零窗口会话没有消息入口，也没有可达的退出动作，`Host::open` 返回 typed error，而不是挂起。
- 应用通过 `Runtime::session()` 取得 Copy 的 `SessionCtx`，在命令处理器中捕获并排队 `request_show(key)` 或 `request_quit()`。会话动作只接受 `Event` / `Delivery` 阶段，在窗口请求之后结算；`request_show` 只打开目录中已声明的窗口，关闭后重新运行 factory，隐藏时恢复同一 activation。
- `Runtime::quit` 是显式、不可撤销的会话动作：关闭全部 activation（包括 stage-one `open_window` 创建的窗口），释放待决 close epoch 与确认窗口，不调用新的关闭处理器；直接调用时同一 turn 尚未结算的窗口请求被丢弃；之后的 `show` 返回 `WindowError::State`。排队结算的 `Quit` 在 `Show` 之后按批次顺序执行，排在它之后的动作随会话结束丢弃。应用需要询问未保存内容时，先完成自己的确认再请求退出。
- 原生宿主在 `StayResident` 零窗口时保留托盘图标，把 worker 唤醒投递到托盘 HWND，并在左键点击时重新打开 primary；托盘菜单可以通过 `SessionCtx::request_show` 恢复已关闭的窗口，通过 `SessionCtx::request_quit` 结束会话。会话结束后宿主同步移除托盘图标并返回消息循环。
- headless Runtime 只维护策略与请求队列，不规定驱动循环。调用方在同一 turn 先结算窗口请求、再结算会话请求；不声明 `StayResident` 也可以显式退出。

窗口目录的 primary 不变量保持不变：常驻会话可以没有存活窗口，但目录始终声明的 primary 是托盘恢复和宿主首次打开的入口。关闭确认 epoch 仍绑定父 activation；显式退出清理它们，但不把 quit 当作一次 Allow 回复。

## Considered Options

- **保留最后一个窗口关闭即退出的隐式行为**：调用方无法区分“应用策略”和宿主实现细节，也无法在选择常驻时关闭最后一个窗口。拒绝。
- **默认常驻，直到显式退出**：没有托盘的普通应用会在零窗口后永久挂起，必须额外提供隐藏消息窗口或超时。拒绝；常驻必须是显式产品决定。
- **常驻不要求托盘**：原生宿主在零窗口时没有 HWND 可接收 worker 唤醒或用户退出动作，只能制造一个隐式托盘或空转。拒绝；headless 驱动不受此限制。
- **退出复用逐窗口关闭确认**：一次 quit 会变成多次异步 Allow/Deny，且窗口可能在确认期间重新出现；与“显式、确定结束”冲突。拒绝；应用先完成自己的确认。
- **把平台 session end 与应用 quit 合并**：系统终止不可撤销且不能等待无限期 worker，应用 quit 仍可在模型层收尾；两者需要不同路径。本切片只定义应用 quit，平台终止仍由后续会话工单处理。
- **关闭最后一个窗口自动改为隐藏**：close 释放 activation 与本地编辑状态，hide 保留它们；替换会改变已有窗口语义和 stale 规则。拒绝。

## Consequences

- 宿主循环条件从“是否还有窗口”改成“会话是否继续运行”；默认策略仍产生与之前相同的退出时机，但现在由声明的策略驱动。
- `StayResident` 宿主的唤醒目标在零窗口时回退到托盘 HWND；托盘图标、`TaskbarCreated` 恢复和点击恢复沿用 #198 的路径。
- 会话请求是新的应用级队列；窗口请求与 close 回复的结算顺序保持不变，会话动作在其后执行。
- 隐藏与 WARP 测试覆盖策略、显式退出、常驻恢复和图标生命周期；可见 `tray_host` 记录关闭到托盘、点击/菜单恢复与 Quit 移除图标。跨显示器与平台 session end 不在本 ADR 的证据范围。
