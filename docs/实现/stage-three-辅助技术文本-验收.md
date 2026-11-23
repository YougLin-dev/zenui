# Stage-three 辅助技术文本验收（实机）

本页记录 #191 的 UIA 文本能力：`text-input` 的 Edit 节点暴露最小 `ITextProvider`/`ITextRangeProvider`，读取已提交文本、selection 与场景矩形，并通过 `Select` 把 selection 交给 UI 线程。文档不包含 preedit；查找、移动、扩展、比较、多选与滚动等 range 方法明确返回 `UIA_E_NOTSUPPORTED`。

**状态：可见验收完成（100% 与 150%）；Narrator 朗读为部分证据。** 文本、selection 与矩形由进程外 .NET UIA 客户端读取；Inspect 以 UI Automation 模式确认 Edit 节点暴露 TextPattern；Narrator 在聚焦草稿与 Details note 时产生语音，但本环境无法逐字读取朗读内容，详见下文。

验收日期：2026-11-24。功能基线为 `main` 的 `714ee67`。可见运行使用仓库内的 `todo_host` 示例；UIA 文本客户端、驱动脚本、Inspect 副本与音频工具放在仓库外，提交前未进入工作树。

## 环境

| 项目 | 100% | 150% |
| --- | --- | --- |
| 显示缩放 | 96 DPI | 144 DPI |
| 主窗口物理客户区 | 884×661 | 1328×994 |
| Details 物理客户区 | 520×420 | 780×630 |
| 动作驱动 | UIA TextPattern/ValuePattern + 物理 `SendInput` | 同左 |
| 读取客户端 | 进程外 .NET `System.Windows.Automation` | 同左 |

单屏 2560×1600；本页不声明多显示器、混合 DPI 或富文本能力。

## provider 行为

- 只有 `text-input` 映射出的 Edit 节点暴露 TextPattern；节点必须仍有已提交值。`TextPattern` 与 `ValuePattern` 可以同时存在。
- `DocumentRange` 的文本是已提交值，preedit 不进入文档；UIA 偏移是 UTF-16 code unit，provider 在调用线程换算为运行时字符偏移，并把越界偏移夹到值的末尾、把代理对内部的偏移夹到该字符起点。组合期间运行时 caret 是 preedit 末尾的显示偏移，发布树改记为被替换区间的终点并清空 selection，因此 GetSelection 不会把 display 偏移当成文档偏移。
- `GetText(-1)` 返回范围内的完整文本；非负 `maxlength` 按 UTF-16 code unit 截断，不拆开代理对。`GetSelection` 返回一个范围：存在 normalized selection 时使用它（折叠的 `(n, n)` 也按折叠范围处理），没有 selection 时折叠在当前 caret。
- `GetVisibleRanges` 对可见文本输入返回整份文档范围，完全离开视口时返回 `(0, 0)` 退化范围（UIA 不允许返回 NULL）；`GetEnclosingElement` 返回同一 Edit 节点；`Clone` 复制范围偏移。`GetVisibleRanges` 不声称部分可见文本的精确裁剪。
- `GetBoundingRectangles` 使用发布树的场景几何：完全离开视口的节点、没有 caret 几何的折叠范围与没有可见 selection 装饰的范围返回空 SAFEARRAY（不是 NULL）；折叠范围在当前位置有 caret 装饰时返回该矩形；非折叠的 selection 范围等于当前 selection 时返回逐行 selection 装饰，即使它覆盖全文；Document 范围（包括 visible ranges）始终返回元素文本区域，客户端持有的过期 selection 也退回元素文本区域。所有矩形按客户端原点和窗口 scale 翻译为物理坐标。
- `Select` 只把 `(start, end)` 排入窗口动作队列；UI 线程读取最新快照，重验节点启用、`on:input` 存在且未在组合中，再调用 `Runtime::set_selection_node`。它不派发 `on:input`，caret 落在范围终点，selection 折叠时 caret 与 anchor 相等。组合中、禁用、卸载或只读输入返回 `UIA_E_ELEMENTNOTAVAILABLE`；窗口隐藏或关闭后旧 provider 同样返回不可用。
- `RangeFromPoint`、`RangeFromChild`、`Move`、`ExpandToEnclosingUnit`、`Compare`、`CompareEndpoints`、`FindAttribute`、`FindText`、`GetAttributeValue`、`MoveEndpointByUnit`、`MoveEndpointByRange`、`AddToSelection`、`RemoveFromSelection`、`ScrollIntoView` 与 `GetChildren` 都返回 `UIA_E_NOTSUPPORTED`。本切片不声明按词/行移动、属性查询、富文本或多个不连续 selection，也不投递 `TextSelectionChanged` 事件；selection 通过 `GetSelection` 读取。

## 方法

- 两种缩放各重启一次可见 `todo_host`，打开主窗口与 Details 窗口，把窗口放在屏幕内并置前。
- 草稿通过 ValuePattern 写为 `a中🙂e\u{0301}z`（ASCII、CJK、非 BMP、组合字符），Details note 写为在输入框宽度内换成两行显示的长句。随后用 TextPattern 读取文档、selection 与矩形，并用键盘 Home/Right/End 移动 caret；`Select` 由 TextPattern 的 `DocumentRange.Select()` 触发。
- 进程外客户端记录元素物理边界、文档、selection 文本、逐行矩形与 caret 矩形。完整日志见 [`text-100.log`](证据/host-uia-text/text-100.log) 与 [`text-150.log`](证据/host-uia-text/text-150.log)。
- 矩形同时与截图中的绘制结果对照；截图为物理客户区，客户端原点由 `ClientToScreen` 独立读取。

## 可见结果

两种缩放的文本与 selection 读数一致：

| 读数 | 100% | 150% |
| --- | --- | --- |
| 草稿文档 | `a中🙂e\u{0301}z` | 同左 |
| 草稿 Home caret 物理矩形 | (71,120,1,19) | (195,268,1,28) |
| 草稿 Right caret 物理矩形 | (78,120,1,19) | (207,268,1,28) |
| 草稿 Select 全选物理矩形 | (71,120,55,19) | (195,268,83,28) |
| Details Home caret 物理矩形 | (187,267,1,19) | (387,507,1,28) |
| Details End caret 物理矩形 | (366,286,1,19) | (657,535,1,28) |
| Details Select 全选物理矩形 | (187,267,454,19)、(187,286,179,19) | (387,507,681,28)、(387,535,269,28) |

DocumentRange 的矩形等于输入元素的物理边界：100% 草稿为 (64,115,782,29)，150% 为 (185,261,1174,43)；Details 分别为 (180,262,488,226) 与 (377,499,732,339)。

把物理矩形减去客户端原点后，两 scale 的局部位置一致并按 1.5 倍缩放：

| 局部读数 | 100% | 150% |
| --- | --- | --- |
| 草稿客户端原点 | (48,71) | (161,195) |
| 草稿全选局部矩形 | (23,49,55,19) | (34,73,83,28) |
| Details 客户端原点 | (164,187) | (353,387) |
| Details 第一行 selection | (23,80,454,19) | (34,120,681,28) |
| Details 第二行 selection | (23,99,179,19) | (34,148,269,28) |

截图与数值一致：

- 草稿 Select 后 caret 在文本末尾，selection 覆盖整个值：[100%](证据/host-uia-text/text-100-main-selection.png)、[150%](证据/host-uia-text/text-150-main-selection.png)。
- 草稿 Home 后 caret 在文本起点：[100%](证据/host-uia-text/text-100-main-caret.png)、[150%](证据/host-uia-text/text-150-main-caret.png)。
- Details 两行 selection 与第二行末尾 caret：[100%](证据/host-uia-text/text-100-details-selection.png)、[150%](证据/host-uia-text/text-150-details-selection.png)；第二行 End caret 见 [100%](证据/host-uia-text/text-100-details-caret.png) 与 [150%](证据/host-uia-text/text-150-details-caret.png)。

`Select` 之后进程外客户端的 `SELECTION_TEXT` 立即变成完整值，草稿 caret 落在选择终点；`on:input` 静默由隐藏测试断言。Home/Right 只改变 caret，selection 文本保持空，矩形随字符宽度移动。

## Inspect

Inspect 以 UI Automation 模式展开 `ZenUI Details`，选中树中的 `Details note`。数据视图显示：

- `How found: Focus`，`Name: "Details note"`，`ControlType: UIA_EditControlTypeId`，`ClassName: "text-input"`，`FrameworkId: "ZenUI"`
- `Value.Value` 为完整 note，`Value.IsReadOnly: false`
- `IsTextPatternAvailable: true`，`IsValuePatternAvailable: true`；`IsTextEditPatternAvailable`、`IsTextChildPatternAvailable`、`IsTextPattern2Available` 与其它未实现 pattern 为 false
- `Next` 为 `"Details rows"` 列表，`Previous` 为 `"Details filter"` 文本，祖先为 `ZenUI Details` 窗口

截图：[节点数据](证据/host-uia-text/inspect-text-150.png) 与 [TextPattern 可用性](证据/host-uia-text/inspect-textpattern-150.png)。TextPattern 行需要滚动数据视图，因此两张截图分别保留节点身份与 pattern 可用性。

## Narrator

Narrator 在最小化主页后保持运行。进程外脚本用真实点击把主窗口与 Details 置于前台，再用 UIA `SetFocus` 聚焦草稿和 Details note；两者都已通过 TextPattern 选中全文。WASAPI loopback 记录到：

- 聚焦草稿前后出现一组语音（录制起点后 0.0–3.4 s），caret 移动后仍有语音（4.8–8.2 s）
- 切到 Details note 后出现一组语音（8.7–10.8 s）

Narrator 以 UIAccess 运行，注入的 Narrator+Alt+X / Narrator+Ctrl+X 被拒绝，Speech Recap 无法由脚本打开；本机 OneCore 语音只有 zh-CN。System.Speech 识别得到近似但不可用的文本，whisper base.en 没有可用输出，因此本页只记录语音活动，不声称逐字内容、朗读次数或 Narrator 主动完成了子范围选择。精确的文本、selection 与 Select 结果由上面的 UIA 客户端日志、Inspect 与隐藏测试证明。原始记录见 [`narrator-text.txt`](证据/host-uia-text/narrator-text.txt)。

## 隐藏测试

`cargo test -p zenui-windows --lib` 新增 8 个测试：

- `accessibility::text::utf16_offsets_round_trip_ascii_cjk_non_bmp_and_combining`：字符与 UTF-16 偏移在 ASCII、CJK、非 BMP 与组合字符上往返，超界与代理对内部偏移夹取。
- `accessibility::text::slices_stay_on_character_boundaries_and_clamp`：范围切片停在字符边界，反向范围归一化，末尾夹取。
- `accessibility::text::truncation_counts_utf16_units_without_splitting_surrogates`：`GetText` 的长度截断按 UTF-16 unit 计，不拆开代理对。
- `host::tests::text_pattern_reads_the_document_and_selection_in_utf16_offsets`：进程内 provider 读取文档与 selection；四种字符范围的 GetSelection 文本与 TextEditState 一致；selection 矩形逐项等于场景 selection 装饰的物理翻译；折叠 selection 的 caret 矩形等于场景 caret。
- `host::tests::text_pattern_reports_no_visible_text_when_the_input_is_scrolled_out`：滚动到长列表底部使草稿完全离开视口；GetVisibleRanges 返回空文本退化范围，折叠范围与文档范围的矩形都是空 SAFEARRAY。
- `host::tests::text_pattern_rejects_every_unsupported_range_operation`：逐个调用未实现的 range/provider 方法并断言 `UIA_E_NOTSUPPORTED`；`Clone` 与 `GetEnclosingElement` 可用。
- `host::tests::text_pattern_select_moves_the_selection_without_dispatching_input`：保存的范围在 caret 移动后仍按 UTF-16 偏移 Select；selection、caret 与 anchor 更新，`on:input` 保持静默；文档范围全选同样不派发输入。
- `host::tests::text_pattern_keeps_composing_disabled_and_static_inputs_read_only`：组合中、禁用与没有 `on:input` 的输入可读文档与 selection，但 Select 返回 `UIA_E_ELEMENTNOTAVAILABLE` 且不运行 handler；组合中的 preedit 不进入 DocumentRange，发布树把显示 caret 映射到被替换区间的文档终点。

隐藏测试只验证 provider 与场景/运行时之间的接口；它们不替代可见 UIA 客户端、Inspect 或 Narrator 结果。

## 判据对照

| #191 要求 | 状态 | 证据 |
| --- | --- | --- |
| 字符偏移与 UIA UTF-16 在 ASCII、CJK、emoji/非 BMP 与组合字符上往返；GetSelection 匹配 TextEditState；GetText 匹配已提交值 | 通过 | 3 个 helper 测试与 provider 读取测试；100%/150% 客户端文档为 `a中🙂e\u{0301}z`，selection 文本与编辑状态一致 |
| caret 与非折叠 selection 矩形在 100% 与 150% 匹配翻译后的 caret/selection 装饰 | 通过 | provider 测试逐项比较场景装饰；可见客户端矩形与截图一致，局部位置按 1.5 倍缩放 |
| Select 调用 set_selection_node 且不派发 on:input；caret 移动一致；组合/禁用输入只读 | 通过 | Select 测试与只读测试；可见客户端 Select 后 selection 文本与 caret 一致，`on:input` 静默由隐藏测试断言 |
| 只实现文档/selection/文本/矩形/Select；其它 range surface 显式拒绝 | 通过 | 未实现方法逐个断言 `UIA_E_NOTSUPPORTED` |
| 隐藏 smoke 与 Inspect 覆盖 provider；Narrator 读取并选择草稿/Details note；fmt、clippy、workspace tests 通过 | 部分通过 | 8 个隐藏测试、Inspect TextPattern 可用；Narrator 有语音活动，但逐字内容与主动选择无法从 UIAccess/zh-CN 环境采集；`cargo fmt --all -- --check`、workspace clippy `-D warnings` 与 workspace tests 通过 |
