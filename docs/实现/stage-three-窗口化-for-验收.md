# Stage-three 窗口化 For 验收（实机记录）

本页记录 #184 的可见 Windows 验收：10,000 行列表只挂载可见行窗口，上下 spacer 保持完整行范围；滚动、编辑、重排与过滤都在真实 HWND 上通过 OS 级输入完成。契约见 [ADR-0011](../adr/0011-windowed-for-mounting.md)。滚轮精度累积、hide/show、close/reopen 与模拟设备丢失的证据边界汇总在 [stage-three 滚动总验收](stage-three-滚动总验收.md)。

**状态：可见自动驱动验收完成（100% 与 150%）。** 列表始终只提交 101–143 个元素（含界面 chrome 与上下 spacer），行带总行数保持 10,002；滚动推进挂载窗口，编辑只作用于已挂载行，重排与过滤后窗口仍正确。

验收日期：2026-11-23。验收对象为 `feat/184-windowed-for` 上 #184 的实现（提交见 PR）。

## 环境

| 项 | 值 |
| --- | --- |
| 操作系统 | Windows 11 专业版 10.0.26300，64 位，交互式控制台会话 |
| 显示缩放 | 100%：`GetDpiForWindow=96`，客户区 900×700；150%：`GetDpiForWindow=144`，客户区 1350×1050（逻辑尺寸始终 900×700） |
| 切换方式 | Settings > 系统 > 屏幕 > 缩放（UI Automation 选中 `100%` / `150%`）；验收结束后恢复 150% |
| 显示 | 单屏 2560×1600，验收期间不使用多显示器 |
| 滚轮设置 | `WheelScrollLines=3`（一个 notch = 72 逻辑像素） |
| 列表 | 演示应用导入 10,000 行 `Imported task NNNNN`，另有 2 行种子数据；`row-height=37`，行带 `total=10002` |
| 输入方式 | OS 级注入：绝对指针移动、左键按下/松开、`MOUSEEVENTF_WHEEL` notch、End/Home/PageDown 与 Unicode 键盘输入；窗口放到屏幕左上角并激活 |

## 方法

- 临时探针（未提交）在 `Host::open` 之前通过演示的导入请求种入 10,000 行，以 `HostConfig::visible()` 运行宿主，每 10 ms `pump()`；滚动偏移、行带窗口、元素数、焦点、编辑值与可见按钮中心变化时输出一行状态。
- 外部脚本按状态行里的逻辑坐标点击（`Edit Imported task …`、`All`、`Done`、`Reverse`），按逻辑坐标滚动；截图读取客户区像素。截图只证明可见窗口的像素与状态，不证明用户注视下的 DWM 时序。
- 每个 DPI 重复同一组步骤：顶部、滚轮 30 notch、End、Home、滚轮 15 notch 后编辑一个行内输入、反向滚轮回顶后点击 Reverse、点击 Done 过滤、点击 All 恢复、聚焦行内输入后 PageDown 触发窗口外丢焦。
- 完整状态见 [`证据/host-windowed-for/100-states.txt`](证据/host-windowed-for/100-states.txt) 与 [`150-states.txt`](证据/host-windowed-for/150-states.txt)。

## 可见结果

### 100%（DPI 96，客户区 900×700）

| # | 步骤 | 预期 | 结果 | 截图 |
| --- | --- | --- | --- | --- |
| 1 | 启动 | 行带从第 0 行开始，只提交窗口；`total=10002` | 通过；`elements=101`，`band_first=0` | [01-top](证据/host-windowed-for/100-01-top.png) |
| 2 | 列表内滚轮 30 notch | 偏移 2160，挂载窗口推进到第 52 行，元素数保持有界 | 通过；`band_first=52`，`elements=143` | [02-scrolled](证据/host-windowed-for/100-02-scrolled.png) |
| 3 | End | 偏移夹到 `max=369613`，窗口推进到末尾 | 通过；`band_first=9983`，`elements=137` | [03-end](证据/host-windowed-for/100-03-end.png) |
| 4 | Home | 偏移归零，窗口回到第 0 行 | 通过；`band_first=0`，`elements=101` | [04-home](证据/host-windowed-for/100-04-home.png) |
| 5 | 滚轮 15 notch 后点击第 28 行输入并输入 `edited` | 编辑命中已挂载行；焦点、caret 与值正确 | 通过；`band_first=23`，`focus=Edit Imported task 00026`，`value=edited` | [05-edit](证据/host-windowed-for/100-05-edit.png) |
| 6 | 回顶后点击 Reverse | 完整 keyed 顺序反转；窗口从新的第 0 行重新挂载 | 通过；顶部显示 `Imported task 09999`，原第 0 行不再可见 | [06-reverse](证据/host-windowed-for/100-06-reverse.png) |
| 7 | 点击 Done 过滤 | 行集收窄为已完成的 1 行；行带与元素数随之收缩，滚动条消失 | 通过；`total=1`，`elements=29`，`max=0` | [07-filter-done](证据/host-windowed-for/100-07-filter-done.png) |
| 8 | 点击 All 恢复 | 完整行集与 10,000 行内容范围恢复 | 通过；`total=10002`，`elements=101`，`max=369613` | [08-filter-all](证据/host-windowed-for/100-08-filter-all.png) |
| 9 | 点击可见行输入后 PageDown | 聚焦行离开窗口超过一行 grace，宿主 blur；窗口随翻页推进 | 通过；`focus=-`，`offset=676`，`band_first=12` | [09-focus-dropped](证据/host-windowed-for/100-09-focus-dropped.png) |

### 150%（DPI 144，客户区 1350×1050）

| # | 步骤 | 预期 | 结果 | 截图 |
| --- | --- | --- | --- | --- |
| 1 | 启动 | 逻辑几何与 100% 相同，物理像素按 1.5 缩放 | 通过；`elements=101`，`band_first=0` | [01-top](证据/host-windowed-for/150-01-top.png) |
| 2 | 滚轮 30 notch | 偏移与挂载窗口按逻辑像素推进 | 通过；`offset=2160`，`band_first=52`，`elements=143` | [02-scrolled](证据/host-windowed-for/150-02-scrolled.png) |
| 3 | End | 夹到 `max=369613`，窗口到末尾 | 通过；`band_first=9983` | [03-end](证据/host-windowed-for/150-03-end.png) |
| 4 | Home | 回到顶部窗口 | 通过；`band_first=0` | [04-home](证据/host-windowed-for/150-04-home.png) |
| 5 | 滚轮 15 notch 后编辑行内输入 | 与 100% 相同的逻辑命中与编辑 | 通过；`focus=Edit Imported task 00026`，`value=edited` | [05-edit](证据/host-windowed-for/150-05-edit.png) |
| 6 | 回顶后 Reverse | 反转后的第一窗口 | 通过；顶部显示 `Imported task 09999` | [06-reverse](证据/host-windowed-for/150-06-reverse.png) |
| 7 | Done 过滤 | 收窄为 1 行 | 通过；`total=1`，`elements=29` | [07-filter-done](证据/host-windowed-for/150-07-filter-done.png) |
| 8 | All 恢复 | 完整行集恢复 | 通过；`total=10002`，`elements=101` | [08-filter-all](证据/host-windowed-for/150-08-filter-all.png) |
| 9 | 聚焦后 PageDown | 丢焦并推进窗口 | 通过；`focus=-`，`offset=676`，`band_first=12` | [09-focus-dropped](证据/host-windowed-for/150-09-focus-dropped.png) |

关键状态（完整行见 `states.txt`）：

| 状态 | 100% | 150% |
| --- | --- | --- |
| 启动 | `offset=0 max=369613 band_first=0 total=10002 elements=101` | 同左 |
| 滚轮 30 notch | `offset=2160 band_first=52 elements=143` | 同左 |
| End | `offset=369613 max band_first=9983 elements=137` | 同左 |
| Home | `offset=0 band_first=0 elements=101` | 同左 |
| 编辑 | `offset=1080 band_first=23 focus=Edit Imported task 00026 value=edited` | 同左 |
| Done 过滤 | `total=1 elements=29 max=0` | 同左 |
| All 恢复 | `total=10002 elements=101 max=369613` | 同左 |
| PageDown 丢焦 | `offset=676 band_first=12 focus=-` | 同左 |

## 证据等级与边界

- **可见运行（主证据）：** 真实 HWND、真实滚轮/键盘/指针输入、DX12 呈现；100% 与 150% 各一组截图与状态行。列表在 10,002 行下始终只提交 101–143 个元素，`band_first` 随滚动从 0 推进到 9983，重排与过滤后窗口与行带更新正确，窗口外行在 Frame 后不再可编辑。
- **隐藏 smoke（部分证据）：** `scroll_only_input_moves_the_row_window_without_touching_the_model` 覆盖 End/Home 推进窗口、窗口外行卸载且旧 Node ID 失效、重新进入获得新 Node ID；`tab_reveals_an_offscreen_focus_target_and_restores_the_top_window` 覆盖显露把窗口带回顶部且焦点 chrome 保留；宿主单元测试 `scrolling_a_focused_composing_row_out_of_the_grace_window_blurs_it` 覆盖远距离滚动 blur 组合编辑器并取消组合。滚动条、reveal 与 IME 锚点的既有 smoke 继续通过。
- **单元与渲染测试：** `windowed_for` 测试覆盖窗口挂载、焦点 grace/丢焦、Tab 边界与重排；渲染测试 `a_windowed_list_keeps_its_declared_extent_and_band_geometry` 覆盖 spacer 高度、内容范围不变与行带几何。
- **未覆盖：** 变量行高与按距离 pin 聚焦行（ADR-0011 明确拒绝）；真实 IME 候选窗在跨窗口丢焦瞬间的可见行为（宿主单元测试覆盖取消路径，可见 IME 组合验收见滚动条验收）；混合 DPI 与多显示器；长会话中反复跨窗口的堆槽复用（`elements` 槽位不复用，属后续优化）。
