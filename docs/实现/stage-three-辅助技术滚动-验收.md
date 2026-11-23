# Stage-three 辅助技术滚动验收（实机）

本页记录 #218 的 UIA 滚动能力：可滚动窗口的根暴露最小垂直 `IScrollProvider`，读取已发布树的场景几何，并把 `Scroll` 与 `SetScrollPercent` 交给宿主拥有的整页偏移。水平滚动、`ScrollItemPattern`、嵌套滚动容器与滚动条 Node ID 不在本页。

**状态：可见验收完成（100% 与 150%）。** 百分比、view size、参考项边界与 `IsOffscreen` 由进程外 .NET UIA 客户端读取；Inspect 以 UI Automation 模式显示窗口根的 ScrollPattern，并在 100% 下通过 `Scroll.SetScrollPercent` 驱动到 50%。

验收日期：2026-11-24。功能基线为 `main` 的 `0f19c42`。可见运行使用仓库内的 `todo_host` 示例；进程外 UIA 客户端、缩放与截图脚本、Inspect 副本放在仓库外，提交前未进入工作树。

## 环境

| 项目 | 100% | 150% |
| --- | --- | --- |
| 显示缩放 | 96 DPI | 144 DPI |
| 主窗口物理客户区 | 452×176 | 678×264 |
| 截图范围 | 客户区 | 客户区 |
| 读取客户端 | 进程外 .NET `System.Windows.Automation` | 同左 |
| 驱动 | 同左；Inspect 驱动一次 `SetScrollPercent` | 同左 |

单屏 2560×1600。两种缩放下窗口的逻辑客户区都是 452×176；内容高度为 286 逻辑像素，`max_scroll_offset` 为 110。本页不声明多显示器或混合 DPI。

## provider 行为

- 只有仍发布树的窗口根返回 `ScrollPattern`。内容不足一屏（max=0）时 pattern 仍存在，`VerticallyScrollable` 为 false，写入返回 `UIA_E_INVALIDOPERATION`；节点不暴露 `ScrollPattern` 或 `ScrollItemPattern`。
- `VerticallyScrollable` 等于 `max_scroll_offset > 0`。`VerticalViewSize` 为 `viewport_height / content_height × 100`，夹在 0..=100；content 高度为 0 时为 100。`VerticalScrollPercent` 为已应用偏移除以最大偏移的百分比，没有可滚动行程时返回 `UIA_ScrollPatternNoScroll`（-1）。读取使用该窗口已发布的 `AccessibilityTree`，不保留第二份偏移。
- 水平侧固定：`HorizontallyScrollable` 为 false，`HorizontalScrollPercent` 与 `HorizontalViewSize` 为 `NoScroll`。`Scroll` 的水平参数只接受 `NoAmount`，`SetScrollPercent` 的水平参数只接受 `NoScroll`；其它值返回 `UIA_E_INVALIDOPERATION`。
- `Scroll` 的 `NoAmount` 为空操作；`LargeIncrement`/`LargeDecrement` 使用与 PageDown/PageUp 相同的 `max(1, floor(viewport_height − 24))` 逻辑像素；`SmallIncrement`/`SmallDecrement` 使用当前滚轮设置的 `lines_per_notch × 24`，page-scroll 哨兵复用页步长，设置为 0 时步长为 0。未知 amount 返回 `UIA_E_INVALIDOPERATION`。
- `SetScrollPercent` 的垂直值必须是 `NoScroll` 或有限 0..=100，`NoScroll` 为空操作。0..100 映射为 `round(max × percent / 100)`，即四舍五入到最近整数逻辑像素（半值远离零），随后夹到 `[0, max]`；回报百分比由实际偏移算出，可能与请求值相差不到一个像素。
- 写侧只入队 `AccessibilityAction::Scroll` 并唤醒 UI 线程。UI 线程按当前场景重验关闭中、窗口启用、运行时接受输入与 `max > 0`，再夹取落点并调用显式滚动。它不读取运行时快照，不触发 reveal，也不刷新 runtime 计数。隐藏、禁用或过期的发布树返回 `UIA_E_ELEMENTNOTAVAILABLE`；零最大偏移返回 `UIA_E_INVALIDOPERATION`；排到 UI 线程后才进入关闭流程的动作被静默丢弃。
- 滚动只改变宿主呈现偏移：Node ID、runtime ID、焦点、caret、selection、draft、组合、快照相等性与运行时计数保持不变；UIA 边界与 `IsOffscreen` 跟随平移后的场景。没有新增滚动条 Node ID。

## 方法

- 两种缩放各启动一次可见 `todo_host`，把主窗口缩到逻辑客户区 452×176，使内容超过一屏并置前。
- 进程外客户端通过窗口根依次执行：读取初始状态；`SetScrollPercent(NoScroll, 50)`；`SetScrollPercent(NoScroll, 100)`；回到 0；`Scroll(NoAmount, LargeIncrement)`、`LargeDecrement`、`SmallIncrement`、`SmallDecrement`。每次调用后 700 ms 读取垂直百分比、view size、参考项 `Add task` 的物理边界与 `IsOffscreen`。
- 100% 运行还用 `Scroll(LargeIncrement, NoAmount)` 验证水平写入被拒绝。
- 截图为物理客户区；完整日志见 [`scroll-100.log`](证据/host-uia-scroll/scroll-100.log) 与 [`scroll-150.log`](证据/host-uia-scroll/scroll-150.log)。

## 可见结果

| 读数 | 100% | 150% |
| --- | --- | --- |
| 垂直 view size | 61.538% | 61.538% |
| 初始垂直百分比 | 0 | 0 |
| `SetScrollPercent` 50 后 | 50 | 50 |
| `SetScrollPercent` 100 后 | 100 | 100 |
| 参考项边界 0 / 50 / 100 | (124,171,420,37) / (124,116,420,37) / (124,61,420,37) | (135,205,630,55) / (135,122,630,55) / (135,40,630,55) |
| 参考项 `IsOffscreen` 在 100% | true | true |
| `Scroll` LargeIncrement | 0→100 | 0→100 |
| `Scroll` LargeDecrement | 100→0 | 100→0 |
| `Scroll` SmallIncrement | 0→65.455 | 0→65.455 |
| `Scroll` SmallDecrement | 65.455→0 | 65.455→0 |
| 水平 `Scroll` | `UIA_E_INVALIDOPERATION` | 由隐藏测试覆盖 |

50% 对应 55 逻辑像素，100% 对应 110。参考项在 100% 下每次上移 55 物理像素；在 150% 下两次位移为 83 与 82 物理像素，即 55 逻辑像素按窗口 scale 换算后的取整。小步长为 72 逻辑像素，占最大偏移的 65.455%，两种缩放相同；150% 的参考项位移为 108 物理像素。大页步长为 `max(1, 176 − 24) = 152`，超过最大偏移后夹在 100%。

截图与日志来自同一组运行：

- 初始：[100%](证据/host-uia-scroll/scroll-100-initial.png)、[150%](证据/host-uia-scroll/scroll-150-initial.png)。
- 50%：[100%](证据/host-uia-scroll/scroll-100-half.png)、[150%](证据/host-uia-scroll/scroll-150-half.png)。
- 100%：[100%](证据/host-uia-scroll/scroll-100-end.png)、[150%](证据/host-uia-scroll/scroll-150-end.png)。

两组的参考项在 100% 后都完全离开视口，100% 与 150% 的截图都只含客户区。

## Inspect

Inspect 以 UI Automation 模式选中 `ZenUI Todo` 窗口根。数据视图在 150% 下显示：

- `How found: Selected from tree`，`Name: "ZenUI Todo"`，`ControlType: UIA_WindowControlTypeId`，`ClassName: "ZenUIHostWindow"`，`FrameworkId: "ZenUI"`
- `Scroll.HorizontallyScrollable: false`，`Scroll.HorizontalScrollPercent: -1.000000`，`Scroll.HorizontalViewSize: -1.000000`
- `Scroll.VerticallyScrollable: true`，`Scroll.VerticalScrollPercent: 0.000000`，`Scroll.VerticalViewSize: 61.538462`
- `IsScrollItemPatternAvailable: false`

截图：[150% 属性](证据/host-uia-scroll/inspect-scroll-150.png)。

100% 下从 Action 菜单打开 `Scroll.SetScrollPercent...`，水平填 -1、垂直填 50 后执行；对话框返回 `SUCCESS - S_OK`，数据视图随后显示 `Scroll.VerticalScrollPercent: 50.000000` 与 `IsScrollPatternAvailable: true`，进程外客户端读到 50 且参考项从 y=171 移到 y=116。截图：[执行结果](证据/host-uia-scroll/inspect-drive-100.png)、[50% 属性](证据/host-uia-scroll/inspect-scroll-100.png)。

## 隐藏测试

`cargo test -p zenui-windows --lib` 中与本能力直接相关的 14 个测试：

- `accessibility::scroll::tests::uia_amounts_map_to_vertical_page_and_line_commands`：UIA 大小增量映射到分页与行命令，`NoAmount` 为空操作。
- `accessibility::scroll::tests::unknown_scroll_amounts_are_invalid`：未知 amount 返回 `UIA_E_INVALIDOPERATION`。
- `accessibility::tests::scroll_geometry_reports_view_size_and_percent`：view size、可滚动状态与百分比从同一份几何派生。
- `accessibility::tests::scroll_geometry_has_no_percent_without_scrollable_content`：零最大偏移没有百分比，view size 封顶为 100。
- `accessibility::tests::scroll_geometry_clamps_percentages_to_the_applied_range`：越界偏移与零高度视口被夹取。
- `host::tests::uia_scroll_percent_rounds_to_the_nearest_pixel_and_clamps`：50%/100% 在奇数最大偏移下四舍五入并夹取。
- `host::tests::uia_scroll_amounts_use_page_and_line_steps_and_clamp`：分页与行步长、两端夹取与水平拒绝。
- `host::tests::scroll_pattern_reports_vertical_geometry_and_horizontal_no_scroll`：provider 读取垂直几何，水平固定为 `NoScroll`。
- `host::tests::scroll_pattern_set_percent_rounds_and_clamps`：provider 入队的百分比在 UI 线程落到正确偏移。
- `host::tests::scroll_pattern_revalidates_a_queued_percent_against_the_current_scene`：排队期间 resize 后按新场景重新映射百分比。
- `host::tests::scroll_pattern_amounts_match_page_and_line_steps_and_clamp`：四个 amount 与 `NoAmount`、未知值、水平值与末端越界。
- `host::tests::scroll_pattern_changes_only_the_presentation_offset`：快照、revision、runtime 计数、Node ID、runtime ID、焦点、selection 与组合不变，reveal 未触发，边界跟随偏移。
- `host::tests::scroll_pattern_rejects_zero_maximum_hidden_and_disabled_targets`：零最大偏移、隐藏、禁用与关闭中目标被拒绝且不刷新 runtime。
- `host::tests::scroll_pattern_is_only_exposed_on_the_window_root`：窗口根暴露 pattern，节点与 `ScrollItemPattern` 都不暴露。

隐藏测试只覆盖 provider 与场景/运行时之间的接口；它们不替代可见 UIA 客户端与 Inspect 结果。

## 判据对照

| #218 要求 | 状态 | 证据 |
| --- | --- | --- |
| 根报告 `VerticallyScrollable = max > 0`、viewport/content 的垂直 view size、由已应用偏移算出的百分比；水平为 `NoScroll` 且不暴露 | 通过 | provider 读取测试；100%/150% 的 view size 61.538% 与百分比 0/50/100；水平字段 false 与 -1 |
| `SetScrollPercent` 把 0..100 映射到夹取的 `[0, max]` 并记录取整；`Scroll` 复用分页/行步长、两端夹取且不越界 | 通过 | 百分比取整与 amount 测试；100%/150% 的 50%、65.455% 与末端 100% |
| provider 只入队，UI 线程按当前场景重验；隐藏、过期、零最大、关闭中与禁用窗口拒绝且不刷新 runtime | 通过 | 队列重验测试与拒绝测试；关闭中动作的计数断言 |
| UIA 滚动只改宿主呈现偏移；Node ID、焦点、caret、selection、draft、组合、快照相等性与 runtime 计数不变；不触发 reveal | 通过 | `scroll_pattern_changes_only_the_presentation_offset` |
| 偏移变化后 UIA 边界与 `IsOffscreen` 跟随平移场景；Inspect 显示并可驱动；100%/150% 可见验收记录；隐藏测试标注为部分证据 | 通过 | 客户端边界与 `IsOffscreen`；Inspect 属性与 50% 驱动；本页日志、截图与证据等级 |
| 不新增 `ScrollItemPattern`、水平滚动、嵌套容器或滚动条 Node ID；fmt、clippy 与 workspace tests 通过 | 通过 | 根/节点 pattern 测试；`cargo fmt --all -- --check`、workspace clippy `-D warnings` 与 workspace tests |

## 证据等级与边界

- **可见运行（主证据）：** 真实 HWND、真实 UIA 跨进程调用与 DX12 呈现；100% 与 150% 各一组日志和截图。不是人工操作。
- **隐藏测试（部分证据）：** 上述 14 个测试覆盖 provider、几何换算、排队重验与状态不变量；它们不显示窗口，不替代可见验收。
- **Inspect 驱动：** 100% 下由 Inspect 的 `Scroll.SetScrollPercent` 对话框执行一次；150% 只读取属性，没有重复驱动。
- **取整边界：** 百分比按最近整数逻辑像素落地，回报值来自实际偏移；奇数最大偏移与超过 100% 的请求由测试覆盖。
- **单屏声明：** 只在单屏 2560×1600 上运行；100% 是临时切换，结束时恢复 150%。本页不声称多显示器、混合 DPI 或跨屏拖动。
- **不覆盖：** `ScrollItemPattern`、水平滚动、嵌套滚动容器、滚动条 Node ID、读屏软件对 ScrollPattern 的朗读顺序，以及菜单/托盘滚动。

## 后续状态

本页关闭 #218 的文档与可见证据缺口；ADR-0010/0011 的契约、滚动条与窗口化 `For` 的专题页范围不变。
