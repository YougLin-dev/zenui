# Stage-three 辅助技术动作验收（实机）

本页记录 #189 的 UIA 写动作：在 #188 的只读树之上增加 Invoke、Toggle、Value 与 SetFocus pattern。provider 线程只检查当前发布并排入类型化动作；UI 线程按入队顺序取走动作，用最新 presentation snapshot 重验后走与指针、键盘相同的 Runtime 命令路径。

**状态：可见验收完成（100% 与 150%）。** 证据来自进程外原生 `IUIAutomation` 客户端与 Inspect 的 UI Automation 模式；窗口是真实可见 HWND，动作由 UIA pattern 调用触发，场景、动作与树都走生产代码。

验收日期：2026-11-23。功能基线为 `main` 的 `ede7513`。可见运行使用临时示例探针（未提交），它在本仓库的 Todo demo 上启动可见宿主；原生客户端与 Inspect 截图脚本也在提交前删除，仓库只保留本页与证据。

## 环境

| 项目 | 100% | 150% |
| --- | --- | --- |
| 显示缩放 | 96 DPI | 144 DPI |
| 初始物理客户区 | 900×700 | 1350×1050 |
| 逻辑视口 | 900×700 | 900×700 |
| UIA 客户端 | 进程外 `IUIAutomation`，Raw View | 同左 |
| Inspect | UI Automation 模式，以下详情面板文本为准 | 同左 |

单屏 2560×1600；本页不声明多显示器或混合 DPI。

## Pattern 语义

| 发布树证据 | UIA pattern | 动作 |
| --- | --- | --- |
| `SnapshotAction::Click` | `InvokePattern` | 恰好一次 `on:click` |
| `SnapshotAction::Change` 且节点有 `checked` / `aria:pressed` 状态 | `TogglePattern` | 读取当前状态并翻转，恰好一次 `on:change` |
| `text-input` 的已提交 `Value` | `ValuePattern` | `Value` 返回已提交文本；`SetValue` 整体替换，恰好一次 `on:input`，caret 到末尾，一条撤销记录 |
| `SnapshotSemantics::focusable` 且已启用 | `IRawElementProviderFragment::SetFocus` | 仅当窗口 HWND 前台时移动运行时焦点 |

Invoke 与 Toggle 是否存在来自 #187 的呈现语义，而不是标签猜测：`text` 节点没有 `InvokePattern`，没有 `on:change` 的按钮没有 `TogglePattern`。`ValuePattern` 是文本输入类型的读模型，Value 来自已提交文本；是否可写由 `Input` 语义、启用状态与组合状态决定。`SetValue` 的新值等于当前值时是静默无操作：不派发 `on:input`，也不产生撤销记录。

SetFocus 是 fragment provider 方法，不是 UIA pattern。

## 排队与重验

provider 可能在任意 COM 线程被调用，因此写方法不直接改 Runtime：

1. provider 解析窗口当前发布，检查 Node ID、启用状态与组合状态；不可用目标返回 `UIA_E_ELEMENTNOTAVAILABLE`，目标存在但不支持该 pattern 时返回 `UIA_E_NOTSUPPORTED`。
2. 通过检查的动作进入窗口自己的 `VecDeque`，随后向窗口 `PostMessageW(WM_APP + 3)` 唤醒 UI 线程。
3. UI 线程按入队顺序取出动作。每条动作都重新读取该窗口最新的 `PresentationSnapshot`，丢弃已卸载、已禁用或已隐藏的目标；组合中的 `SetValue` 也在这一步被拒绝。
4. 通过重验的动作调用既有的 `click_node`、`change_node`、`replace_text_node`、`focus_node`。Runtime 再检查 Node ID、窗口可见性与关闭挂起状态；被正常竞态拒绝的动作不产生宿主错误。单个动作的宿主级失败会记录并在本批结束后报告，不会丢弃已经取出的后续动作。

队列锁只在入队与取走时持有，执行动作期间不持锁；provider 调用与 `Host::pump` 不共享第二把锁，不会形成互相等待。单窗口队列最多保留 1024 个待执行动作；队列溢出或唤醒消息投递失败时，provider 报告 `UIA_E_ELEMENTNOTAVAILABLE`，不静默接受。关闭或隐藏窗口时，发布树与队列一起清空。

## 可见动作结果

同一序列在两种缩放下执行，完整 Raw View 前后 dump 见 [`100-actions.txt`](证据/host-uia-actions/100-actions.txt) 与 [`150-actions.txt`](证据/host-uia-actions/150-actions.txt)；摘要见 [`states.txt`](证据/host-uia-actions/states.txt)。

| 步骤 | 100% | 150% |
| --- | --- | --- |
| `SetFocus` 草稿输入 | `HasKeyboardFocus=true` | `HasKeyboardFocus=true` |
| `SetValue("UIA action")` | `Value.IsReadOnly=false`；值变为 `UIA action` | 同左 |
| `Invoke` Add | 新行 `Complete UIA action` 恰好一个；`2 remaining` | 同左 |
| `Toggle` Complete Read docs | `Toggle.ToggleState` 0 → 1；`1 remaining` | 同左 |
| `Invoke` Reverse | 行序 `[Write tests, Read docs, UIA action]` → `[UIA action, Read docs, Write tests]` | 同左 |

截图：[100% 动作前](证据/host-uia-actions/100-initial.png)、[100% 动作后](证据/host-uia-actions/100-after-actions.png)、[150% 动作前](证据/host-uia-actions/150-initial.png)、[150% 动作后](证据/host-uia-actions/150-after-actions.png)。动作后的截图可见新增行、已完成复选框与反序后的三行。

## Inspect

Inspect 的 UI Automation 模式在两种缩放下读取同一批元素：

- `Add` 按钮：`IsInvokePatternAvailable=true`，Toggle 与 Value 为 false。截图：[100%](证据/host-uia-actions/inspect-invoke-100.png)、[150%](证据/host-uia-actions/inspect-invoke-150.png)。
- 草稿输入：`IsValuePatternAvailable=true`，`Value.IsReadOnly=false`，Invoke/Toggle 为 false。150% 的详情面板在滚动后截图为 [inspect-value-150.png](证据/host-uia-actions/inspect-value-150.png)；两种缩放的完整详情文本见 [inspect-value-100.txt](证据/host-uia-actions/inspect-value-100.txt)、[inspect-value-150.txt](证据/host-uia-actions/inspect-value-150.txt)。
- `Complete Read docs`：`IsTogglePatternAvailable=true`，`Toggle.ToggleState` 与已提交状态一致。两 scale 的详情文本见 [inspect-toggle-100.txt](证据/host-uia-actions/inspect-toggle-100.txt)、[inspect-toggle-150.txt](证据/host-uia-actions/inspect-toggle-150.txt)。

Inspect 截图使用 Kits 里同一份 `inspect.exe` 的临时副本，只把 manifest 的 `uiAccess` 置为 false，让它以当前用户完整性级别运行并接受自动化。按原 manifest 运行的实例是 UIAccess，Windows 会拦截非提升进程对它的合成输入，并让它的启动警告对话框无法关闭；该副本在提交前删除。详情面板在 150% 下比窗口高，Value/Toggle 的 pattern 行位于可视区之外，因此这两个元素以上面的 Inspect 文本 dump 为准，不用裁剪截图冒充完整读数。

## 隐藏测试

`cargo test -p zenui-windows --lib` 与根包测试覆盖：

- `invoke_and_toggle_patterns_enqueue_one_command_each_in_order`：provider 调用只入队；`Host::pump` 后各自恰好一次，顺序为 `click, change`；无动作的节点没有 pattern。
- `queued_actions_are_revalidated_against_the_latest_snapshot`：排在前面的动作禁用目标后，后续已入队的 Invoke 被 UI 线程丢弃。
- `actions_report_unavailable_for_disabled_stale_and_hidden_targets`：禁用、已卸载、隐藏目标返回 `UIA_E_ELEMENTNOTAVAILABLE` 且不运行 handler。
- `value_pattern_replaces_the_draft_once_and_records_one_undo`：一次 `on:input`、caret 在末尾、一次撤销；组合中的动作被重验拒绝；组合中 `IsReadOnly=true`。
- `set_focus_only_queues_a_focusable_target_and_never_steals_foreground`：后台窗口不移动运行时焦点；不可聚焦元素返回 `UIA_E_NOTSUPPORTED`。
- `suspended_parent_targets_report_unavailable_and_keep_their_value`：关闭确认挂起父窗口的 SetValue/SetFocus 返回不可用，值保持不变。
- `an_action_enqueued_from_another_thread_runs_only_after_pump`：非 UI 线程只改变队列，唤醒消息之后才运行。
- 根包 `replacing_the_value_dispatches_one_input_and_records_one_undo_entry` 与 `replace_ignores_the_selection_and_moves_the_caret_to_the_end`：整体替换的编辑与历史语义。

这些测试只提供部分证据：窗口在隐藏模式下创建，不显示原生 UI，不替代上面的可见 UIA 验收。

## 判据对照

| #189 要求 | 状态 | 证据 |
| --- | --- | --- |
| Invoke 恰好一次 `on:click` | 通过 | 可见 Add 只新增一行；host 测试 |
| Toggle 恰好一次 `on:change` | 通过 | 可见状态 0→1 且剩余数只减一；host 测试 |
| 禁用/过期/隐藏/挂起目标报告不可用且不运行 handler | 通过 | 禁用/过期/隐藏与关闭挂起父窗口均由 host 测试覆盖；可见动作序列不包含禁用目标 |
| SetValue 整体替换、一次 `on:input`、caret 末尾、一次撤销 | 通过 | 可见值替换；`value_pattern_*` 与运行时测试 |
| 组合中或禁用输入为只读 | 通过 | Inspect `Value.IsReadOnly`；host 测试 |
| SetFocus 只在前台窗口移动运行时焦点 | 通过 | 可见 `HasKeyboardFocus=true`；后台 host 测试 |
| provider 非 UI 线程只入队，`pump` 按序执行且无死锁 | 通过 | 队列与线程测试；`Host::pump` 可见运行 |
| pattern 存在性来自呈现语义 | 通过 | Inspect 与 host pattern 矩阵 |
| 隐藏测试与 Inspect 可见 Todo | 通过 | 本页两节 |
| fmt / clippy / workspace tests | 通过 | 本工单的 workspace 门禁 |

## 证据等级与边界

- **可见原生 UIA（主证据）：** 进程外 `IUIAutomation` 客户端在 100% 与 150% 下调用真实 pattern，读取动作后的树；Inspect 以同一模式确认 pattern 集合与状态。
- **隐藏测试（部分证据）：** 覆盖入队、顺序、重验、禁用/过期/隐藏/挂起、只读、撤销与线程边界；不显示窗口，不替代可见验收。
- **未覆盖：** UIA 事件属于 #190，`TextPattern` 属于 #191，`ScrollPattern` 属于 #218；本页不声称读屏软件的朗读顺序、焦点事件订阅或文本范围接口。
- **单屏声明：** 只在单屏 2560×1600 上运行；100% 与 150% 是同一显示器的临时设置，结束时已恢复 150%；不声称混合 DPI 或多显示器。
- **脱敏：** 应用截图只含客户区；Inspect 截图裁掉桌面与标题栏，只保留工具窗口。临时探针、UIA 客户端与 Inspect 副本在提交前删除。

## 后续状态

本页关闭 #189 的写 pattern 与可见证据；#190 的 UIA 事件、#191 的文本范围与 #192 的汇总契约仍由后续工单交付。
