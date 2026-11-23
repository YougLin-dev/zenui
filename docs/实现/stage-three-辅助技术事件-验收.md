# Stage-three 辅助技术事件验收（实机）

本页记录 #190 的 UIA 事件：相邻两次发布的可访问性树按 runtime ID 比较，聚焦、启用、ToggleState、Value 与子节点增删/重排各投递一次事件。事件在发布锁释放后由发布线程投递；后台窗口的焦点变化不产生聚焦事件，指针 hover/press 与滚动几何不改变树，因此不产生事件。

**状态：可见验收完成（100% 与 150%）；Narrator 逐字朗读为部分证据。** 事件流由进程外原生 UIA 客户端记录，Inspect 以 UI Automation 模式跟踪同一焦点变化；Narrator 的朗读文本受 UIAccess 与仅 zh-CN 语音限制，只记录音频活动，不作为逐字判据，详见下文。

验收日期：2026-11-24。功能基线为 `main` 的 `413331d`。可见运行使用仓库内的 `todo_host` 示例；事件客户端、驱动脚本与音频工具放在仓库外，提交前未进入工作树。

## 环境

| 项目 | 100% | 150% |
| --- | --- | --- |
| 显示缩放 | 96 DPI | 144 DPI |
| 物理客户区 | 904×681 | 1478×1044 |
| 动作驱动 | UIA pattern + 物理 `SendInput` | 同左 |
| 事件客户端 | 进程外 .NET `System.Windows.Automation` | 同左 |

单屏 2560×1600；本页不声明多显示器或混合 DPI。

## 事件规划

- 节点身份使用 provider 本地 runtime ID；根没有本地 runtime ID，以窗口默认 ID 表示。
- 结构事件按父节点的子序列比较：ChildAdded 发在新增子节点，ChildRemoved 在父节点携带被移除的 runtime ID，ChildrenReordered 在父节点。新子树只报告顶层新增，移除子树同理；客户端收到后重新读取内部结构。
- IsEnabled、ToggleState 与 Value 只比较两次发布都存在的节点。新节点的初始状态由结构事件表达，不给它补一条属性事件。`checked` 与 `aria:pressed` 共用 ToggleState。
- 焦点事件只在窗口发布时处于前台才投递；后台窗口的焦点变化不在之后激活时补发。焦点清空没有 UIA 事件，客户端以 `GetFocus`/`GetFocusedElement` 为空观察，不能拿根节点冒充新焦点。
- 根节点对应 HWND，原生 `IsEnabled` 由宿主窗口 provider 自己通知；规划器跳过根，避免同一目标收到两条事件。
- 一次提交内顺序固定为结构、属性、焦点。一次用户动作可以包含多次提交：例如 Invoke 先把焦点移到按钮，再提交模型变化，因此日志里可能先看到聚焦事件，再看到结构与 Value 事件。
- 单条投递失败不阻断后续事件；事件是通知，不影响场景提交或 Runtime 状态。

## 方法

- 两种缩放各启动一次可见 `todo_host`，把窗口放在屏幕内并置前。
- 进程外客户端订阅该 PID 的 focus changed、property changed（IsEnabled、ToggleState、Value、HasKeyboardFocus）与 structure changed，记录时间、名称、AutomationId、控制类型、runtime ID 与新值。完整日志见 [`100-events.txt`](证据/host-uia-events/100-events.txt)、[`150-events.txt`](证据/host-uia-events/150-events.txt)；动作时间见 [`100-actions.txt`](证据/host-uia-events/100-actions.txt)、[`150-actions.txt`](证据/host-uia-events/150-actions.txt)。
- 动作序列：点击草稿输入；Tab 到 Add；Shift+Tab 回输入；点击 Complete Read docs；ValuePattern.SetValue("Event row")；Invoke Add；Invoke Reverse；Invoke Delete Read docs；点击 Active 过滤；SetValue("pending draft")；Alt+F4 请求关闭；在确认窗 Invoke Continue editing。
- 物理点击使用屏幕坐标；UIA pattern 调用走 #189 的排队动作路径。两种缩放下序列相同。

## 可见结果

主要读数（两 scale 相同）：

| 动作 | FOCUS | 属性 | 结构 |
| --- | --- | --- | --- |
| 点击草稿输入 | "What needs to be done?" ×1 | — | — |
| Tab | "Add" ×1 | — | — |
| Shift+Tab | 草稿输入 ×1 | — | — |
| 点击 Complete Read docs | — | ToggleState 0→1 ×1 | — |
| SetValue("Event row") | — | Value ""→"Event row" ×1 | — |
| Invoke Add | "Add" ×1 | Value "Event row"→"" ×1 | ChildAdded ×1 |
| Invoke Reverse | "Reverse" ×1 | — | ChildrenReordered ×1 |
| Invoke Delete Read docs | "Delete Write tests" ×1 | — | ChildRemoved 携带 runtime ID ×1 |
| 点击 Active 过滤 | "Delete Event row" ×1 | ToggleState All 1→0、Active 0→1 | ChildRemoved ×1 |
| SetValue("pending draft") | — | Value ""→"pending draft" ×1 | — |
| Alt+F4 打开确认 | — | IsEnabled false：根 ×1，非根 26 个各 ×1 | — |
| Continue editing | — | IsEnabled true：根 ×1，非根 26 个各 ×1 | — |

合计：8 次聚焦、3 次 ToggleState、3 次 Value、1 次 ChildAdded、1 次 ChildrenReordered、2 次 ChildRemoved、54 次 IsEnabled（两个方向各 27 个元素，根一次、非根各一次）。两个 scale 的计数一致；归一化后只有相邻两次提交之间的到达顺序差一条，事件集合相同。

代表性日志（150%）：

```text
FOCUS name="Add" ... runtime=[42,65014790,4,5]
STRUCTURE ChildAdded runtime=[] sender=... type=ControlType.Group runtime=[42,65014790,4,33]
PROPERTY ValuePatternIdentifiers.ValueProperty old="Event row" new="" name="What needs to be done?" ...
STRUCTURE ChildrenReordered runtime=[] sender=... type=ControlType.Group runtime=[42,65014790,4,20]
STRUCTURE ChildRemoved runtime=[42,65014790,4,27] sender=... type=ControlType.Group runtime=[42,65014790,4,20]
PROPERTY TogglePatternIdentifiers.ToggleStateProperty old=1 new=0 name="All" ...
PROPERTY TogglePatternIdentifiers.ToggleStateProperty old=0 new=1 name="Active" ...
PROPERTY AutomationElementIdentifiers.IsEnabledProperty old=<null> new=False name="ZenUI Todo" ...
PROPERTY AutomationElementIdentifiers.IsEnabledProperty old=True new=False name="Add task" ...
```

根的两条 IsEnabled（`old=<null>`）来自宿主窗口 provider；非根事件来自本工单的规划器。两个方向都只出现一次，没有重复投递。

截图：[100% 初始](证据/host-uia-events/100-initial.png)、[100% 动作后](证据/host-uia-events/100-after-actions.png)、[150% 初始](证据/host-uia-events/150-initial.png)、[150% 动作后](证据/host-uia-events/150-after-actions.png)。动作后截图可见 Active 过滤、待决草稿与 Event row；关闭确认截图见 [150-confirm.png](证据/host-uia-events/150-confirm.png)。

## Inspect

Inspect 的 UI Automation 模式打开 Watch Focus 后，Tab 到 Add 时数据视图以 `How found: Focus` 更新为：

- `Name: "Add"`，`ControlType: UIA_ButtonControlTypeId`，`HasKeyboardFocus: true`
- `RuntimeId: [2A.3E00C06.4.5]`，`FrameworkId: "ZenUI"`
- `ProviderDescription: ... ZenUI accessibility provider`

截图见 [inspect-focus-150.png](证据/host-uia-events/inspect-focus-150.png)。Inspect 没有独立的 UIA 事件日志面板；“Watch Focus”就是它消费聚焦事件的方式，因此本页不能声称 Inspect 提供了结构/属性事件日志。结构、属性事件的一手记录是上面的进程外客户端。

## Narrator

Narrator 在可见运行中开启；它使用与进程外客户端相同的 UIA 事件流。直接读取朗读文本受两点限制：

- Narrator 以 UIAccess 运行，注入的 Narrator+Alt+X（Speech Recap）与 Narrator+Ctrl+X（复制上一句）被系统拒绝，无法用脚本打开或读取回顾窗口。
- 本机 OneCore 语音只有 zh-CN；Narrator 用中文语音读英文名称，离线识别只能得到近似文本。

因此改用 WASAPI loopback 记录默认扬声器：SetFocus 到复选框、Space 切换、SetFocus 到 All 三个动作后都检测到语音活动；同一动作后没有观察到重复朗读同一短语的音频。原始声音与识别文本不作为逐字判据，记录见 [`narrator-audio.txt`](证据/host-uia-events/narrator-audio.txt)。精确的“一次”由事件客户端的逐条日志与 Inspect Focus 跟踪证明。

## 隐藏测试

`cargo test -p zenui-windows --lib` 的规划器测试（11 个）覆盖：

- `unchanged_and_first_trees_emit_nothing`
- `focus_changes_are_announced_once_only_while_foreground`
- `a_background_focus_change_is_not_replayed_on_activation`
- `clearing_focus_is_not_a_focus_event`
- `enabled_toggle_and_value_changes_emit_one_property_event_each`
- `disabling_and_reenabling_a_window_reports_each_changed_property_once`
- `added_and_removed_children_emit_one_structure_event_each`
- `nested_children_report_the_parent_runtime_id`
- `keyed_reordering_emits_one_event_on_the_parent`
- `a_new_or_removed_subtree_reports_only_its_top_level_child`
- `a_removed_node_produces_no_property_or_focus_event_for_itself`

已移除节点的缓存 provider 在现有 `actions_report_unavailable_for_disabled_stale_and_hidden_targets` 返回 `UIA_E_ELEMENTNOTAVAILABLE`；规划器测试同时确认不为该节点产生属性或聚焦事件。隐藏窗口不显示原生 UI，规划器测试只是部分证据。

## 判据对照

| #190 要求 | 状态 | 证据 |
| --- | --- | --- |
| 纯 diff 测试覆盖焦点、启用、checked、pressed、Value、子节点增删与 keyed reorder；未变化不产生事件 | 通过 | 11 个规划器测试；`checked`/`aria:pressed` 都进入 ToggleState |
| 只有前台窗口投递聚焦事件；激活不补发后台焦点；关闭确认禁用/恢复各一次 | 通过 | 两个规划器测试；可见 IsEnabled 54 条，根与非根各方向一次 |
| 移除节点的 provider 返回不可用；过期 NodeId 不崩溃、不产生事件 | 通过 | 现有 stale provider 测试；规划器“removed node”测试 |
| Narrator 对可见 Todo 朗读焦点/状态/结构变化一次；Inspect 事件日志一致 | 部分通过 | 事件客户端与 Inspect Watch Focus 一致；Narrator 有语音活动，但逐字文本与结构朗读无法从 UIAccess/zh-CN 环境采集 |
| fmt / clippy / workspace tests | 通过 | 本工单的 workspace 门禁；Windows lib 106 个测试通过 |

## 证据等级与边界

- **可见原生 UIA（主证据）：** 进程外客户端在 100% 与 150% 下记录同一动作序列；事件集合、计数与目标一致。Inspect 的 Watch Focus 确认聚焦事件被标准客户端消费。
- **Narrator（部分证据）：** 只能记录语音活动，不能记录逐字内容；不声称未验证的结构朗读文本。
- **隐藏测试（部分证据）：** 只覆盖纯规划器与既有 provider 生命周期，不显示窗口，不替代可见 UIA。
- **未覆盖：** 本工单不投递滚动或几何属性事件；`TextPattern` 已由 [文本验收](stage-three-辅助技术文本-验收.md) 交付，`ScrollPattern` 已由 [滚动验收](stage-three-辅助技术滚动-验收.md) 交付，菜单/托盘事件仍属后续工单。混合 DPI 与多显示器不在本页声明。
- **脱敏：** 应用截图只含客户区；Inspect 截图裁掉标题栏与桌面树；不包含用户名或本机路径。临时客户端、驱动脚本、音频、Inspect 副本都放在仓库外。

## 后续状态

本页关闭 #190 的焦点、属性与结构事件；`TextPattern` 已由 [文本验收](stage-three-辅助技术文本-验收.md) 交付，`ScrollPattern` 已由 [滚动验收](stage-three-辅助技术滚动-验收.md) 交付。
