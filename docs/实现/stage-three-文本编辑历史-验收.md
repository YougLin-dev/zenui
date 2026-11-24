# Stage-three 文本编辑历史验收（实机）

本页记录文本输入撤销/重做的完整切片：#204 把 Windows 宿主上的 Ctrl+Z/Ctrl+Y 接到运行时历史，#205 把连续单字符输入合并为有界撤销步，#206 汇总契约、SDK 与可见验收。运行时的历史语义见 [ADR-0014](../adr/0014-text-edit-history.md)，宿主按键路径见 [SDK 窗口与任务](../SDK/窗口与任务.md)；文本输入的公开编辑与组合契约见 [SDK 样式与输入](../SDK/样式与输入.md)。

**状态：可见自动驱动完成（Windows 11 单屏 150%）；隐藏宿主测试与运行时历史测试只是部分证据，没有单独记录人工逐项按键或视觉复核。**

验收日期：2026-11-24。功能基线为 `main` 的 `ae7cd74`。可见运行使用仓库内的 `crates/windows/examples/todo_host.rs`；点击、按键、UIA 文本查询、截图与窗口读数脚本放在仓库外，未进入工作树。

## 环境

| 项目 | 读数 |
| --- | --- |
| 显示 | 单屏 2560×1600，150%（144 DPI） |
| 主窗口 | 标题 `ZenUI Todo`，外框 1372×1106 物理像素，位于 (48,48)，客户区 1350×1050 |
| 详情窗口 | 标题 `ZenUI Details`，外框 802×686 物理像素，位于 (48,48)，客户区 780×630 |
| 输入 | 主窗口草稿 `What needs to be done?` 与详情窗口 `Details note` |
| 输入法 | 微软拼音；英文采集前用真实 Shift 切到英文模式 |

## 方法

- `shortcuts::editor_shortcut` 是宿主与编辑器之间的唯一策略 seam：只有聚焦的启用可编辑输入、按下 Ctrl 且没有进行中的 IME 组合时返回动作。Ctrl+A 全选，Ctrl+Z/Y 进入运行时历史；Ctrl+C/V 已由剪贴板工单接线，Ctrl+X 仍由宿主消费并保留给剪切工单。
- `Host::key_down_with` 在快照中解析聚焦输入；Ctrl+Z 调用 `Runtime::undo_node`，Ctrl+Y 调用 `Runtime::redo_node`，随后请求显露与重绘。一次成功的历史应用恰好派发一次 `on:input`，应用模型仍是值的权威。
- 连续单字符插入合并为一个撤销步：条目保留第一次插入的 `before` 与最近一次插入的 `after`，每个键入字符仍各派发一次 `on:input`，撤销/重做则整步写回。run 最多 64 个字符；空白并入前一个 run（行首空白自成一个 run），下一个非空白字符开启新步。
- caret/selection 移动、焦点变化、Backspace/Delete、多字符插入、整体替换、组合开始/提交、撤销/重做与外部替换都会结束当前 run。空 run 的撤销/重做是静默无操作；新编辑清空 redo。
- 撤销恢复一个 run 的起始值、caret、anchor 与归一化 selection；重做恢复该 run 提交后的状态。精确恢复只对原样提交值的处理器成立；handler 改写值后以已提交值为准。
- 运行时错误通过现有的 `ignore_expected_dispatch` 收敛：禁用、组合中、隐藏、关闭挂起与过期节点不写模型；没有焦点输入时宿主在进入快捷键策略前返回，不派发也不移动焦点。

## 可见场景

所有截图都来自同一台 150% 机器；按键由 `keybd_event` 注入真实键盘消息队列，操作前用 `SetForegroundWindow` 置前并确认前台 HWND。文本值用仓库外的 UIA `TextPattern` 读取 `DocumentRange.GetText(-1)`，因此不可见的尾随空格与中文码元也有读数。

### 连续输入、撤销与重做

| 步骤 | 输入值 | 证据 |
| --- | --- | --- |
| 初始，草稿输入聚焦且为空 | `""` | [150-initial.png](证据/host-history/150-initial.png) |
| 键入 `ab cd` | `ab cd` | [150-typed.png](证据/host-history/150-typed.png) |
| Ctrl+Z（撤销第二个词） | `ab `（码元 `97,98,32`） | [150-undo.png](证据/host-history/150-undo.png) |
| Ctrl+Z（撤销第一个 run） | `""` | [150-undo-run.png](证据/host-history/150-undo-run.png) |
| Ctrl+Y（重做第一个 run） | `ab ` | [150-redo.png](证据/host-history/150-redo.png) |
| 键入 `x`（清空 redo） | `ab x`（码元 `97,98,32,120`） | [150-redo-cleared-before.png](证据/host-history/150-redo-cleared-before.png) |
| Ctrl+Y（redo 已清空） | `ab x` 不变 | [150-redo-cleared-after.png](证据/host-history/150-redo-cleared-after.png) |

空格在截图中不可见，UIA 读数证明第一次撤销后尾随空格仍在；新编辑清空 redo 后两次读数都是 `ab x`。

### 选择替换

键入 `abcd`，Ctrl+A 全选，再键入 `y`：一次单字符编辑替换整个选择。Ctrl+Z 恢复 `abcd`、caret 在末尾，并恢复撤销前的选择。

| 状态 | 输入值 / 选择 | 证据 |
| --- | --- | --- |
| 选择替换后 | `y` | [150-selection-replaced.png](证据/host-history/150-selection-replaced.png) |
| Ctrl+Z 后 | `abcd`，UIA `GetSelection` 为 `abcd` | [150-selection-restored.png](证据/host-history/150-selection-restored.png) |

### Backspace 与 Delete

在 `abcd` 末尾按 End 后 Backspace，删除 `d`；Ctrl+Z 恢复 `abcd`。移到开头后 Delete 删除 `a`；Ctrl+Z 再次恢复 `abcd`。

| 操作 | 输入值 | 证据 |
| --- | --- | --- |
| Backspace | `abc` | [150-backspace.png](证据/host-history/150-backspace.png) |
| Ctrl+Z | `abcd` | [150-backspace-undo.png](证据/host-history/150-backspace-undo.png) |
| Home + Delete | `bcd` | [150-delete.png](证据/host-history/150-delete.png) |
| Ctrl+Z | `abcd` | [150-delete-undo.png](证据/host-history/150-delete-undo.png) |

### 中文组合提交

微软拼音下键入 `nihao` 形成 preedit `nǐhao`（组合下划线）；Space 选中候选后已提交值为 `你好`（码元 `20320,22909`），Ctrl+Z 撤销整次组合提交。提交后系统 IME 可能继续在应用上绘制组合下划线；UIA 已提交值与随后 Ctrl+Z 的行为表明它不是运行时 composition，下划线何时消失由 IME 决定，截图保留所见。

| 状态 | 输入值 | 证据 |
| --- | --- | --- |
| 组合中 preedit | `nǐhao`，UIA 已提交值为空 | [150-ime-preedit.png](证据/host-history/150-ime-preedit.png) |
| 候选提交 | `你好`（UIA；截图中系统 IME 下划线仍在） | [150-ime-committed.png](证据/host-history/150-ime-committed.png) |
| Ctrl+Z | `""` | [150-ime-undo.png](证据/host-history/150-ime-undo.png) |

### 两个输入 / 两个窗口

主窗口草稿输入保留 `main`；打开 `ZenUI Details` 后在 `Details note` 键入 `note`，Ctrl+Z 只撤销详情窗口的输入，主窗口草稿保持不变。

| 状态 | 读数 | 证据 |
| --- | --- | --- |
| 详情窗口键入 `note` | `Details note` = `note` | [150-details-typed.png](证据/host-history/150-details-typed.png) |
| 详情窗口 Ctrl+Z | `Details note` = `""` | [150-details-undo.png](证据/host-history/150-details-undo.png) |
| 主窗口 | `What needs to be done?` 仍为 `main`（码元 `109,97,105,110`） | [150-two-windows-main.png](证据/host-history/150-two-windows-main.png) |

两个输入的历史互不共享；撤销详情窗口不写主窗口的模型。

### 空栈

新建的可见运行中，草稿输入聚焦且为空时按 Ctrl+Z 与 Ctrl+Y：值保持 `""`，caret 与焦点不变，没有 `on:input`。截图与 UIA 读数见 [150-empty-undo.png](证据/host-history/150-empty-undo.png) 和 [150-empty-redo.png](证据/host-history/150-empty-redo.png)。

## 隐藏测试

`zenui-windows` 的 178 个库测试包含 5 个宿主映射测试：

- `ctrl_z_undoes_and_ctrl_y_redoes_value_caret_and_selection_once`：替换选区后 Ctrl+Z 恢复 `old`、anchor 1、caret 3 与 selection `(1, 3)`，Ctrl+Y 恢复 `oX`、caret 2 与折叠 selection；每次恰好一次 `on:input`。
- `typing_after_undo_clears_the_redo_stack`：撤销后键入 `Y`，再 Ctrl+Y 不派发，值保持新编辑结果。
- `undo_and_redo_without_a_focused_input_are_consumed_no_ops`：没有焦点输入时不派发、不移动焦点、不改值。
- `undo_and_redo_during_composition_are_consumed_no_ops`：组合与 preedit 保持，值不变，不派发。
- `undo_and_redo_after_the_focused_input_is_disabled_are_consumed_no_ops`：输入被禁用后按键不派发、不移动焦点、不改值。

根包 `tests/text_history.rs` 的 29 个测试覆盖词与空白、行首空白、64 字符上界、caret/selection 移动、焦点变化、Backspace/Delete、多字符插入、整体替换、组合提交、CJK、emoji、撤销/重做、外部替换、共享 Signal、隐藏窗口、条目/字节容量与卸载。workspace 串行 810 个测试通过。

## 判据对照

| 要求 | 状态 | 证据 |
| --- | --- | --- |
| #204：Ctrl+Z 撤销、Ctrl+Y 重做聚焦启用输入；值、caret、anchor/selection 与每次一次 `on:input` 跟随运行时契约 | 通过 | 宿主映射测试；可见 `ab cd`→`ab `→`""`；选择替换恢复 |
| #204：撤销后键入清空 redo | 通过 | `typing_after_undo_clears_the_redo_stack`；可见 `ab x` 后 Ctrl+Y 不变 |
| #204：无焦点、禁用输入或组合中为消费的无操作，不派发、不移动焦点 | 通过 | 三个 no-op 宿主测试；组合 preedit 保持 |
| #204：Ctrl+A、方向键、Home/End、Backspace/Delete、Enter、Tab 与 IME 路径不变 | 通过 | 既有导航、回车、Tab 与 IME 测试；可见 Ctrl+A、End、Home、Backspace、Delete |
| #205：连续输入按词撤销，空白后的下一个词独立 | 通过 | 可见 `ab cd` 第一次 Ctrl+Z 得 `ab `；`a_typed_word_undoes_as_one_step_and_the_next_word_is_separate` |
| #205：移动、焦点、Backspace/Delete、多字符插入、组合提交、撤销/重做、外部替换开启新 run | 通过 | 同名边界测试；`movement_starts_a_new_run`、`backspace_and_multi_scalar_inserts_start_new_runs`、`composition_commit_and_focus_changes_start_new_runs`、`typing_after_undo_does_not_merge_into_the_older_run`、`external_value_changes_clear_both_stacks` |
| #205：合并 run 的撤销/重做恢复整段 before/after，重做仍是一步 | 通过 | 词测试的 caret/selection 与输入计数；可见 `""`→`ab ` |
| #205：确定性、无 timer、有文档化的 run 上界；覆盖空白、CJK、emoji | 通过 | [ADR-0014](../adr/0014-text-edit-history.md) 记录 64 字符上界；`a_run_stops_at_the_documented_scalar_bound`、`cjk_and_emoji_typing_coalesce_by_scalar`、`leading_whitespace_is_its_own_run` |
| #206：ADR、GLOSSARY、SDK 与验证重点记录已交付历史语义 | 通过 | [ADR-0014](../adr/0014-text-edit-history.md)；GLOSSARY 的 Text edit history / Application undo；[SDK 样式与输入](../SDK/样式与输入.md)；[验证重点](验证重点.md) |
| #206：可见页覆盖输入、Ctrl+Z/Y、选择替换、Backspace/Delete、中文组合提交、redo 失效、两个输入/窗口与空栈 | 通过（单屏 150%） | 本页六个场景与 21 张图；UIA 读数见 [`states.txt`](证据/host-history/states.txt) |
| #206：隐藏/CI 标注部分证据；临时探针移出工作树；截图不含个人数据 | 通过 | 本页证据等级；脚本均在仓库外；截图只含示例 Todo 与操作读数 |
| `cargo fmt --all -- --check`、workspace clippy、workspace tests | 通过 | PR 的 12 个必需检查与本地门禁 |

## 边界

- **粒度：** 连续单字符输入至多合并 64 个字符；空白并入前一个 run，行首空白自成一 run。多字符插入、整体替换、删除与组合提交各自成步。
- **IME 下划线：** 候选提交后微软拼音可能继续在应用上绘制组合下划线；UIA 已提交值与 Ctrl+Z 行为表明它不是运行时 composition，下划线何时消失由 IME 决定。不把该残留归为 ZenUI 的状态，也不据此声称 IME 组合已结束。
- **缩放：** 只在单屏 150% 运行；键位、合并策略与历史语义不依赖 DPI，没有重复 100%、混合 DPI 或多显示器。
- **可见范围：** 两个输入/窗口的可见隔离只覆盖主窗口草稿与详情窗口 note；更多窗口、隐藏后历史与容量在隐藏测试覆盖。可见主窗口值使用英文 `main`，避免系统 IME 下划线混入两窗口读数。
- **采集：** 窗口图来自 `PrintWindow(PW_RENDERFULLCONTENT)`；右/下的黑色条是采集产物。按键由 `keybd_event` 注入真实键盘消息队列；UIA 客户端与截图脚本放在仓库外，提交前未进入工作树。

## 后续状态

本页关闭 #204、#205 与 #206 的文本编辑历史切片。当前交付是每个输入自己的有界 undo/redo、Windows Ctrl+Z/Y 映射与确定性 run 合并；带 origin/revision 的编辑器协议、按原因恢复 selection 与跨输入/应用级撤销仍是目标设计，不与本页的已交付语义混用。契约见 [ADR-0014](../adr/0014-text-edit-history.md)。
