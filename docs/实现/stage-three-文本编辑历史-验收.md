# Stage-three 文本编辑历史验收（实机）

本页记录 #204：Windows 宿主把聚焦启用文本输入上的 Ctrl+Z/Ctrl+Y 接到运行时的 `undo_node`/`redo_node`。历史本身由 headless 运行时按输入保存，契约见 [ADR-0014](../adr/0014-text-edit-history.md)；本页只覆盖宿主映射、可见按键与当前已交付的边界。连续单字符输入合并为词级撤销由 #205 处理，完整的产品验收与文档汇总由 #206 补充。

**状态：可见自动驱动完成（Windows 11 单屏 150%）；隐藏宿主测试与运行时历史测试只是部分证据，没有单独记录人工逐项按键或视觉复核。**

验收日期：2026-11-24。功能基线为 `main` 的 `92241e3`。可见运行使用仓库内的 `crates/windows/examples/todo_host.rs`；点击、按键、截图与窗口读数脚本放在仓库外，未进入工作树。

## 环境

| 项目 | 读数 |
| --- | --- |
| 显示 | 单屏 2560×1600，150%（144 DPI） |
| 示例窗口 | 标题 `ZenUI Todo`，外框 1372×1106 物理像素，位于 (48,48) |
| 客户区 | 1350×1050 物理像素 |
| 输入 | 草稿输入 `What needs to be done?`，真实键盘消息注入 |
| 输入法 | 微软拼音；采集前用真实 Shift 按键切到英文模式，字母作为普通编辑提交 |

## 方法

- `shortcuts::editor_shortcut` 仍是宿主与编辑器之间的唯一策略 seam：只有聚焦的启用可编辑输入、按下 Ctrl 且没有进行中的 IME 组合时返回动作。Ctrl+A 全选，Ctrl+Z/Y 进入运行时历史，Ctrl+C/X/V 继续由宿主消费并保留给剪贴板工单。
- `Host::key_down_with` 在快照中解析聚焦输入；Ctrl+Z 调用 `Runtime::undo_node`，Ctrl+Y 调用 `Runtime::redo_node`，随后请求显露与重绘。一次成功的历史应用恰好派发一次 `on:input`，应用模型仍是值的权威。
- 运行时错误通过现有的 `ignore_expected_dispatch` 收敛：禁用、组合中、隐藏、关闭挂起与过期节点不写模型；没有焦点输入时宿主在进入快捷键策略前返回，不派发也不移动焦点。
- 撤销恢复编辑前的值、caret、anchor 与归一化 selection；重做恢复该次编辑提交后的状态。空栈是静默无操作；新编辑清空 redo 栈。
- 本切片每个单字符插入是一条历史，因此可见运行中一次 Ctrl+Z 只撤销最后键入的字符。词级合并、Backspace/Delete 的可见撤销、中文组合提交与两个输入的历史隔离不在本工单。

## 隐藏测试

`zenui-windows` 的 178 个库测试包含 5 个新的宿主测试：

- `ctrl_z_undoes_and_ctrl_y_redoes_value_caret_and_selection_once`：替换选区后 Ctrl+Z 恢复 `old`、anchor 1、caret 3 与 selection `(1, 3)`，Ctrl+Y 恢复 `oX`、caret 2 与折叠 selection；每次恰好一次 `on:input`。
- `typing_after_undo_clears_the_redo_stack`：撤销后键入 `Y`，再 Ctrl+Y 不派发，值保持新编辑结果。
- `undo_and_redo_without_a_focused_input_are_consumed_no_ops`：没有焦点输入时不派发、不移动焦点、不改值。
- `undo_and_redo_during_composition_are_consumed_no_ops`：组合与 preedit 保持，值不变，不派发。
- `undo_and_redo_after_the_focused_input_is_disabled_are_consumed_no_ops`：输入被禁用后按键不派发、不移动焦点、不改值。

这些测试只断言 presentation snapshot 与 `on:input` 计数；运行时的值/caret/anchor/selection 恢复、空栈、容量与外部替换仍由根包 `tests/text_history.rs` 覆盖。workspace 串行 801 个测试通过。

## 可见读数

| 步骤 | 输入值 | 证据 |
| --- | --- | --- |
| 初始，草稿输入聚焦且为空 | `""` | [150-initial.png](证据/host-history/150-initial.png) |
| 键入 `abc` | `abc`，caret 在末尾 | [150-typed.png](证据/host-history/150-typed.png) |
| Ctrl+Z | `ab`，caret 在末尾 | [150-undo.png](证据/host-history/150-undo.png) |
| Ctrl+Y | `abc`，caret 在末尾 | [150-redo.png](证据/host-history/150-redo.png) |
| 再次 Ctrl+Z 后键入 `d` | `abd`，caret 在末尾 | [150-redo-cleared-before.png](证据/host-history/150-redo-cleared-before.png) |
| Ctrl+Y（redo 已被新编辑清空） | `abd` 不变 | [150-redo-cleared-after.png](证据/host-history/150-redo-cleared-after.png) |

最后两张图字节相同：新编辑清空 redo 栈后，Ctrl+Y 不派发 `on:input`，值、caret 与焦点都不变。完整坐标、`fg`/HWND 读数与输入法处理见 [`states.txt`](证据/host-history/states.txt)。

## 判据对照

| #204 要求 | 状态 | 证据 |
| --- | --- | --- |
| Ctrl+Z 撤销、Ctrl+Y 重做聚焦启用输入；值、caret、anchor/selection 与每次一次 `on:input` 跟随运行时契约 | 通过 | 选中替换与撤销/重做测试；可见 `abc`→`ab`→`abc` |
| 撤销后键入清空 redo | 通过 | `typing_after_undo_clears_the_redo_stack`；可见 `abd` 后 Ctrl+Y 不变 |
| 无焦点、禁用输入或组合中为消费的无操作，不派发、不移动焦点 | 通过 | 三个 no-op 宿主测试；组合 preedit 保持 |
| Ctrl+A、方向键、Home/End、Backspace/Delete、Enter、Tab 与 IME 路径不变 | 通过 | 既有 `ctrl_a_*`、导航、回车、Tab 与 IME 测试；本次只替换 Undo/Redo 分支 |
| 隐藏宿主测试断言 snapshot；可见 Windows 验收显示真实按键；隐藏证据是部分的 | 通过（可见单屏 150%） | 五个宿主测试；六张可见图；`states.txt` |
| `cargo fmt --all -- --check`、workspace clippy、workspace tests | 通过 | PR 的 12 个必需检查与本地门禁 |

## 边界

- **粒度：** 当前每个单字符插入是一条历史；连续输入合并为词级撤销不在本切片，见 #205。
- **可见范围：** 没有在可见运行中覆盖两个输入/窗口的历史隔离、Backspace/Delete 撤销、中文组合提交或空栈按键；这些由隐藏测试与后续验收补充。
- **组合：** 采集时微软拼音处于中文模式，键入字母形成 preedit（下划线），此时 Ctrl+Z 不改变 preedit 或值，组合优先由 IME/宿主组合路径消费；正式采集前用真实 Shift 切到英文模式，截图中的字母都是已提交编辑。
- **缩放：** 只在单屏 150% 运行；这次键位映射不依赖 DPI，没有重复 100%、混合 DPI 或多显示器。
- **采集：** 窗口图来自 `PrintWindow(PW_RENDERFULLCONTENT)`；右/下的黑色条是采集产物。按键由 `keybd_event` 注入真实键盘消息队列，每次操作前确认前台 HWND 与目标一致。

## 后续状态

本页记录 #204 的宿主按键映射与可见读数。运行时的历史条目、容量、组合与外部替换语义见 [ADR-0014](../adr/0014-text-edit-history.md)；#205 负责词级合并，#206 负责 ADR/GLOSSARY/SDK/验证重点与更完整的文本历史验收页。
