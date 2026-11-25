# Stage-three 外部替换与组合验收（实机）

本页记录文本编辑 origin/revision 血统的收尾切片：#280 交付结算与 stale settlement 的分类，#281 交付外部替换的 selection 策略与组合取消，#282 记录契约、SDK 文本与可见证据。运行时契约见 [ADR-0019](../adr/0019-text-edit-origin-and-revision.md)，历史规则仍见 [ADR-0014](../adr/0014-text-edit-history.md)；文本输入的编辑语义见 [SDK 样式与输入](../SDK/样式与输入.md)。

**状态：可见自动驱动完成（Windows 11 单屏 150%）；隐藏宿主测试、运行时血统测试与 CI 只是部分证据，没有单独记录人工逐项按键或视觉复核。**

验收日期：2026-11-25。功能基线为 `main` 的 `f4c493f`（#281 的合并提交）；可见运行使用包含本工单宿主 IME 同步修改的当前工作树。探针 `zenui-probe-282` 位于仓库外：一个窗口的 `text-input` 绑定应用 `Signal`，app 所有权的 worker 任务等待仓库外的触发文件，然后在 `Phase::Event` 的 continuation 里写入 `external`。探针、按键驱动与截图脚本都在工作树之外，提交前没有进入仓库；仓库只保留截图与读数。

## 环境

| 项目 | 读数 |
| --- | --- |
| 显示 | 单屏 2560×1600，150%（144 DPI） |
| 探针窗口 | 标题 `ZenUI Probe 282`，外框 982×356 物理像素，客户区 960×300 |
| 输入 | `aria:label="Probe editor"`，绑定应用 `Signal<String>` |
| 输入法 | 微软拼音；候选确认用 Space，组合模式由可观察的已提交值推断 |

## 方法

- 探针把输入绑定到窗口创建前 `Runtime::setup` 建立的应用 Signal；`on:input` 回写同一 Signal，任务 continuation 在 `Phase::Event` 写入 `external`。这是一条普通模型写入，不是 `Runtime::input` 的显式整体替换。
- 任务在 worker 线程只做文件轮询与返回字符串，不接触 UI；结果为 `external`，宿主唤醒后经 `drive_tasks` 交付。窗口中的 state 文本从 `waiting` 变为 `reset applied`，证明交付确实执行。
- 按键通过 `keybd_event` 注入真实键盘消息队列；文本值、选区与选区矩形通过 UIA `TextPattern` 的 `DocumentRange.GetText(-1)`、`GetSelection()` 与 `GetBoundingRectangles()` 读取；截图来自 `PrintWindow(PW_RENDERFULLCONTENT)`。
- 组合期间 UIA 已提交值保持不变，preedit 只在编辑器内，因此“已提交值是否变化”把组合与普通插入区分开，不依赖跨线程的 `ImmGetConversionStatus`。
- 宿主在外部替换取消运行时组合后的下一帧变化呈现前调用 `ImmNotifyIME(CPS_CANCEL)` 丢弃 OS IME 组合串；`crates/windows` 的隐藏测试注入 OS 组合状态验证该决策。本页的 Enter 步骤验证取消后候选不会再次提交。

## 可见场景

同一台 150% 机器、同一次运行；读取顺序与时间戳见 [`states.txt`](证据/host-external-reset/states.txt)。

| 步骤 | 读数 | 证据 |
| --- | --- | --- |
| 初始空输入聚焦 | committed `""`，selection `""` | states `[23:21:30.633]` |
| 第一次组合键入 `ni` | committed 仍 `""`，preedit 显示 `ni` | [150-first-preedit.png](证据/host-external-reset/150-first-preedit.png) |
| Space 确认候选 | committed `你`，产生一条撤销历史 | [150-committed.png](证据/host-external-reset/150-committed.png) |
| Ctrl+A 全选 | selection `你` | [150-selected.png](证据/host-external-reset/150-selected.png) |
| 第二次组合键入 `nihao` | committed 仍 `你`，preedit 为 `ni'hao` 并替换选区 | [150-preedit.png](证据/host-external-reset/150-preedit.png) |
| 任务写入 `external` | committed `external`，selection 折叠；preedit 取消 | [150-reset.png](证据/host-external-reset/150-reset.png) |
| Enter（候选确认尝试） | 仍为 `external`，没有二次提交 | [150-enter.png](证据/host-external-reset/150-enter.png) |
| Ctrl+Z | 仍为 `external`，外部替换已清空两个历史栈 | [150-undo.png](证据/host-external-reset/150-undo.png) |
| 切到英文键入 `x` | `externalx`，selection 折叠且矩形在文本末尾 | [150-typed.png](证据/host-external-reset/150-typed.png) |

外部替换发生时窗口标题下的 state 文本从 `waiting` 变为 `reset applied`；`external` 替换了组合前的值 `你`，选区与 preedit 都不再存在。Enter 没有把被取消的候选内容写进新值，Ctrl+Z 也没有恢复 `你`；随后的 `x` 落在 `external` 末尾，证明 caret 被放在新值末尾且编辑器仍可继续输入。

## 隐藏测试

- 根包内部的 `lineage_tests` 用运行时公开入口驱动并检查内部 trace：迟到结算被忽略、规范化 handler 以已提交值结算、旧值写入结算、同 key 重新挂载得到新 origin、已结算编辑的旧迟到值保持 stale、deferred Memo 写入不回滚、Memo 值绑定结算改变后的值、外部替换取消组合时记录 `CompositionCancelledByReset`。
- `tests/external_reset.rs` 的 7 个测试覆盖范围内 selection、越界 selection 与 caret 收拢、空值归零、组合取消（不派发 `on:input`）、CJK/emoji 字符边界、显式 `Runtime::input` 路径与隐藏/显示期间的共享 Signal 变更。
- `tests/ime.rs` 的 `an_external_value_change_cancels_the_composition` 断言重置后的值、caret、被拒绝的旧候选提交，以及新组合可以正常提交。
- `crates/windows` 新增 `an_external_reset_during_composition_cancels_the_os_composition` 与 `a_normal_composition_survives_an_unrelated_task_delivery`：前者在任务交付外部值后确认运行时 preedit 消失、caret 到末尾且宿主恰好通知一次 OS IME 取消；后者确认无关的任务交付重绘窗口时不取消仍在进行的组合。

## 判据对照

| 要求 | 状态 | 证据 |
| --- | --- | --- |
| #282：新 ADR 记录契约并指回 ADR-0014；ADR-0014 与 SDK 页不再把已交付协议写成未来工作 | 通过 | [ADR-0019](../adr/0019-text-edit-origin-and-revision.md)；[ADR-0014](../adr/0014-text-edit-history.md#revisionorigin-protocol) 改为指向 ADR-0019；[SDK 样式与输入](../SDK/样式与输入.md) 的编辑、组合与选择策略段落 |
| #282：验证重点的三个 target-design 项标记为已交付并给出测试 | 通过 | [验证重点](验证重点.md) 的“Windows 真实闭环”三条编辑血统/连续编辑/外部 replace 条目与宿主 IME 同步条目 |
| #282：可见运行记录 preedit 活动、外部重置、组合取消、无二次提交、caret/selection、历史清空与重置后输入 | 通过（单屏 150%） | 本页九个步骤、8 张图与 `states.txt` |
| #282：临时探针移除；截图与状态脱敏并提交 | 通过 | 探针与脚本都在 `E:\code\zenui-probe-282`，未进入工作树；截图只含探针窗口与示例读数 |
| `cargo fmt --all -- --check`、文档链接检查与 PR 必需检查 | 通过 | PR 的必需检查与本地门禁 |

## 边界

- **探针：** 可见场景使用仓库外的临时探针，而不是在两个可见窗口之间共享草稿；探针与驱动脚本不提交。仓库内的 Todo 示例没有为这轮验收增加重置按钮或临时属性。
- **宿主同步时机：** OS IME 的取消发生在外部替换后的下一帧变化呈现之前；可见运行在值已经变为 `external` 后才按 Enter，因此验证的是“取消已生效后候选不会再次提交”，不覆盖 IME 在同一消息内先收到候选确认的竞态。
- **输入法差异：** 候选选择与组合串格式由微软拼音决定；第一次组合用 Space 确认首选候选，记录的是该 IME 实际提交的 `你`。不同 IME 的候选顺序不改变运行时契约，但读数会不同。
- **缩放与显示器：** 只在单屏 150% 运行；selection 夹取与字符边界不依赖 DPI，没有重复 100%、混合 DPI 或多显示器，后者仍属 #63 的独立缺口。
- **已知限制：** 普通 `String` 绑定下，与仍在途的派发摘要相同的外部值按 stale settlement 处理；经 Memo 把值规范化回编辑前值的 handler 无法与延迟交付区分。两者记录在 [ADR-0019](../adr/0019-text-edit-origin-and-revision.md) 的 Known limitations，并由 #278 spec 的 Out of Scope 约定。
- **采集：** 截图来自 `PrintWindow(PW_RENDERFULLCONTENT)`；右/下黑边是采集产物。UIA 客户端、按键与截图脚本放在仓库外；提交前未进入工作树。
