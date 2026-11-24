# Stage-three 文本编辑历史验收（实机）

本页记录 #204 与 #205：Windows 宿主把聚焦启用文本输入上的 Ctrl+Z/Ctrl+Y 接到运行时的 `undo_node`/`redo_node`，运行时把连续单字符输入合并为有界撤销步。历史的完整契约见 [ADR-0014](../adr/0014-text-edit-history.md)；本页覆盖宿主映射、当前粒度与可见按键。更完整的产品验收与文档汇总由 #206 补充。

**状态：可见自动驱动完成（Windows 11 单屏 150%）；隐藏宿主测试与运行时历史测试只是部分证据，没有单独记录人工逐项按键或视觉复核。**

验收日期：2026-11-24。功能基线为 `main` 的 `620d60b`。可见运行使用仓库内的 `crates/windows/examples/todo_host.rs`；点击、按键、UIA 文本查询、截图与窗口读数脚本放在仓库外，未进入工作树。

## 环境

| 项目 | 读数 |
| --- | --- |
| 显示 | 单屏 2560×1600，150%（144 DPI） |
| 示例窗口 | 标题 `ZenUI Todo`，外框 1372×1106 物理像素，位于 (48,48) |
| 客户区 | 1350×1050 物理像素 |
| 输入 | 草稿输入 `What needs to be done?`，真实键盘消息注入 |
| 输入法 | 微软拼音；采集前用真实 Shift 按键切到英文模式，字母与空格作为普通编辑提交 |

## 方法

- `shortcuts::editor_shortcut` 仍是宿主与编辑器之间的唯一策略 seam：只有聚焦的启用可编辑输入、按下 Ctrl 且没有进行中的 IME 组合时返回动作。Ctrl+A 全选，Ctrl+Z/Y 进入运行时历史，Ctrl+C/X/V 继续由宿主消费并保留给剪贴板工单。
- `Host::key_down_with` 在快照中解析聚焦输入；Ctrl+Z 调用 `Runtime::undo_node`，Ctrl+Y 调用 `Runtime::redo_node`，随后请求显露与重绘。一次成功的历史应用恰好派发一次 `on:input`，应用模型仍是值的权威。
- 运行时把连续的单字符插入合并为一个撤销步：条目保留第一次插入的 `before` 与最近一次插入的 `after`，每个键入字符仍各派发一次 `on:input`，撤销/重做则整步写回。连续输入 run 最多 64 个字符；空白并入前一个 run（行首空白自成一个 run），下一个非空白字符开启新步。
- caret/selection 移动、焦点变化、Backspace/Delete、多字符插入、整体替换、组合开始/提交、撤销/重做与外部替换都会结束当前 run。空 run 的撤销/重做仍是静默无操作；新编辑清空 redo。
- 运行时错误通过现有的 `ignore_expected_dispatch` 收敛：禁用、组合中、隐藏、关闭挂起与过期节点不写模型；没有焦点输入时宿主在进入快捷键策略前返回，不派发也不移动焦点。
- 撤销恢复一个 run 的起始值、caret、anchor 与归一化 selection；重做恢复该 run 提交后的状态。精确恢复只对原样提交值的处理器成立。

## 隐藏测试

`zenui-windows` 的 178 个库测试包含 5 个宿主映射测试：

- `ctrl_z_undoes_and_ctrl_y_redoes_value_caret_and_selection_once`：替换选区后 Ctrl+Z 恢复 `old`、anchor 1、caret 3 与 selection `(1, 3)`，Ctrl+Y 恢复 `oX`、caret 2 与折叠 selection；每次恰好一次 `on:input`。
- `typing_after_undo_clears_the_redo_stack`：撤销后键入 `Y`，再 Ctrl+Y 不派发，值保持新编辑结果。
- `undo_and_redo_without_a_focused_input_are_consumed_no_ops`：没有焦点输入时不派发、不移动焦点、不改值。
- `undo_and_redo_during_composition_are_consumed_no_ops`：组合与 preedit 保持，值不变，不派发。
- `undo_and_redo_after_the_focused_input_is_disabled_are_consumed_no_ops`：输入被禁用后按键不派发、不移动焦点、不改值。

根包 `tests/text_history.rs` 的 29 个测试覆盖词与空白、行首空白、64 字符上界、caret/selection 移动、焦点变化、Backspace/Delete、粘贴形状的多字符插入、组合提交、CJK 与 emoji、撤销/重做、外部替换、共享 Signal、隐藏窗口、容量与卸载。`zenui-windows` 测试只断言 presentation snapshot 与 `on:input` 计数；workspace 串行 810 个测试通过。

## 可见读数

| 步骤 | 输入值 | 证据 |
| --- | --- | --- |
| 初始，草稿输入聚焦且为空 | `""` | [150-initial.png](证据/host-history/150-initial.png) |
| 键入 `ab cd` | `ab cd`，caret 在末尾 | [150-typed.png](证据/host-history/150-typed.png) |
| Ctrl+Z（撤销第二个词） | `ab `，caret 在末尾 | [150-undo.png](证据/host-history/150-undo.png) |
| Ctrl+Z（撤销第一个 run） | `""`，caret 在开头 | [150-undo-run.png](证据/host-history/150-undo-run.png) |
| Ctrl+Y（重做第一个 run） | `ab ` | [150-redo.png](证据/host-history/150-redo.png) |
| 键入 `x`（清空 redo） | `ab x` | [150-redo-cleared-before.png](证据/host-history/150-redo-cleared-before.png) |
| Ctrl+Y（redo 已清空） | `ab x` 不变 | [150-redo-cleared-after.png](证据/host-history/150-redo-cleared-after.png) |

空格在截图中不可见，UIA `TextPattern` 读数确认撤销后的值是 `ab `（UTF-16 码元 `97,98,32`，3 个字符），而不是 `ab`；清空 redo 后的两次读数都是 `ab x`（`97,98,32,120`）。完整坐标、`fg`/HWND 读数、UIA 读数与输入法处理见 [`states.txt`](证据/host-history/states.txt)。

## 判据对照

| 要求 | 状态 | 证据 |
| --- | --- | --- |
| #204：Ctrl+Z 撤销、Ctrl+Y 重做聚焦启用输入；值、caret、anchor/selection 与每次一次 `on:input` 跟随运行时契约 | 通过 | 选中替换与撤销/重做测试；可见 `ab cd`→`ab `→`""` |
| #204：撤销后键入清空 redo | 通过 | `typing_after_undo_clears_the_redo_stack`；可见 `ab x` 后 Ctrl+Y 不变 |
| #204：无焦点、禁用输入或组合中为消费的无操作，不派发、不移动焦点 | 通过 | 三个 no-op 宿主测试；组合 preedit 保持 |
| #204：Ctrl+A、方向键、Home/End、Backspace/Delete、Enter、Tab 与 IME 路径不变 | 通过 | 既有导航、回车、Tab 与 IME 测试；本次只替换 Undo/Redo 分支 |
| #205：连续输入按词撤销，空白后的下一个词独立 | 通过 | 可见 `ab cd` 第一次 Ctrl+Z 得 `ab `；`a_typed_word_undoes_as_one_step_and_the_next_word_is_separate` |
| #205：移动、焦点、Backspace/Delete、多字符插入、组合提交、撤销/重做、外部替换开启新 run | 通过 | 同名边界测试；`movement_starts_a_new_run`、`backspace_and_multi_scalar_inserts_start_new_runs`、`composition_commit_and_focus_changes_start_new_runs`、`typing_after_undo_does_not_merge_into_the_older_run`、`external_value_changes_clear_both_stacks` |
| #205：合并 run 的撤销/重做恢复整段 before/after，重做仍是一步 | 通过 | 词测试的 caret/selection 与输入计数；可见 `""`→`ab ` |
| #205：确定性、无 timer、有文档化的 run 上界；覆盖空白、CJK、emoji | 通过 | ADR-0014 记录 64 字符上界；`a_run_stops_at_the_documented_scalar_bound`、`cjk_and_emoji_typing_coalesce_by_scalar`、`leading_whitespace_is_its_own_run` |
| 隐藏测试断言 snapshot；可见 Windows 验收显示真实按键；隐藏证据是部分的 | 通过（可见单屏 150%） | 五个宿主测试、29 个历史测试；七张可见图与 UIA 读数 |
| `cargo fmt --all -- --check`、workspace clippy、workspace tests | 通过 | PR 的 12 个必需检查与本地门禁 |

## 边界

- **粒度：** 连续单字符输入至多合并 64 个字符；空白并入前一个 run，行首空白自成一 run。多字符插入、整体替换、删除与组合提交各自成步。
- **可见范围：** 没有在可见运行中覆盖两个输入/窗口的历史隔离、Backspace/Delete 撤销、中文组合提交或 64 字符上界；这些由隐藏测试与后续验收补充。
- **组合：** 采集时微软拼音处于中文模式，键入字母形成 preedit（下划线），此时 Ctrl+Z 不改变 preedit 或值，组合优先由 IME/宿主组合路径消费；正式采集前用真实 Shift 切到英文模式，截图中的字母与空格都是已提交编辑。
- **缩放：** 只在单屏 150% 运行；这次键位与合并策略不依赖 DPI，没有重复 100%、混合 DPI 或多显示器。
- **采集：** 窗口图来自 `PrintWindow(PW_RENDERFULLCONTENT)`；右/下的黑色条是采集产物。按键由 `keybd_event` 注入真实键盘消息队列，每次操作前确认前台 HWND 与目标一致；UIA 客户端在仓库外读取文本与码元。

## 后续状态

本页记录 #204 的宿主按键映射与 #205 的有界连续输入合并。运行时的历史条目、容量、组合与外部替换语义见 [ADR-0014](../adr/0014-text-edit-history.md)；#206 负责 ADR/GLOSSARY/SDK/验证重点的最终汇总与更完整的文本历史验收。
