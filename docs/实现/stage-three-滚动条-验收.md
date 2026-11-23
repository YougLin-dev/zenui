# Stage-three 滚动条验收（实机记录）

本页记录 #182 的可见 Windows 验收：覆盖层滚动条获得指针命中优先权，thumb 拖动与轨道点击、滚轮、PageUp/PageDown、Home/End 共享同一个夹取的滚动偏移，拖动与点击都不移动焦点、不取消 IME 组合。契约见 [ADR-0010](../adr/0010-scroll-is-presentation-state.md)；滚动条的渲染几何与测试见 #181，滚动条仍是覆盖层，不新增 Node ID。

**后续状态（2026-11）：** 窗口化 `For` 已由 #184 交付（[窗口化 For 验收](stage-three-窗口化-for-验收.md)），滚动总验收补充滚轮精度累积、hide/show 与 close/reopen 的汇总证据，见 [stage-three 滚动总验收](stage-three-滚动总验收.md)。

**状态：可见自动驱动验收完成（100% 与 150%）。** 运行使用真实可见 HWND、真实 `WM_MOUSE*`/`WM_MOUSEWHEEL` 路径、宿主默认 DX12 presenter 与真实微软拼音输入法；拖动由 OS 级 `SendInput` 完成（左键按下、8 段真实鼠标移动、松开），点击、滚轮与键盘同样来自 `SendInput`。截图证明可见窗口的像素与场景状态，不证明用户注视下的 DWM 时序。

验收日期：2026-11-23。验收对象为 `feat/182-scrollbar-interaction` 上 #182 的实现（提交见 PR）；评审修正后按同一组步骤重跑并补充聚焦拖动与组合期间按键的步骤，运行后、提交前唯一的变化是删除临时示例探针。

## 环境

| 项 | 值 |
| --- | --- |
| 操作系统 | Windows 11 专业版 10.0.26300，64 位，交互式控制台会话 |
| 显示缩放 | 100%：`GetDpiForWindow=96`，客户区 900×700；150%：`GetDpiForWindow=144`，客户区 1350×1050 |
| 切换方式 | Settings > 显示 > 缩放（UI Automation）+ `rundll32 user32.dll,UpdatePerUserSystemParameters`；结束后恢复 150%（`GetDpiForSystem=144`） |
| 图形 | NVIDIA GeForce RTX 4060 Laptop GPU，驱动 32.0.15.7283；可见宿主使用默认 DX12 路径 |
| 显示 | 单屏 2560×1600 @ 240 Hz |
| 滚轮设置 | `WheelScrollLines=3`（一个 notch = 72 逻辑像素） |
| 滚动条几何 | 轨道 8 逻辑像素宽、贴右缘内缩 4；500 行场景的 thumb 约 25.97 逻辑像素高；轨道 `(888, 4, 8, 692)`，`max=17954` |
| 输入方式 | OS 级 `SendInput`：真实光标移动、左键按下/松开、`MOUSEEVENTF_WHEEL` notch、PageDown/PageUp/Home/End；窗口放到屏幕左上角并激活 |

## 方法

- 临时探针（未提交）用公开的 `Runtime::input` / `submit` 在 `Host::open` 之前种入 500 行，用 `HostConfig::visible()` 运行宿主，并以 10ms 周期 `pump()`；滚动偏移、滚动条几何、焦点、组合与 preedit 变化时打印一行状态。探针不注入额外交互。
- 拖动由外部脚本用 `SendInput` 完成：光标移到 thumb、左键按下、按 8 段移动、松开；轨道点击、滚轮与键盘同样由 `SendInput` 注入。应用截图用 `PrintWindow(PW_CLIENTONLY|PW_RENDERFULLCONTENT)` 读取客户区。
- 每个 DPI 重复同一组步骤：起点截图、拖动到轨道中部、拖动到底部、点击 thumb 上方的轨道、Home/End 与滚轮/PageDown 复核，最后在拖动结束后点击一行 checkbox，确认宿主仍可用且没有残留按压/悬停。
- 评审修正后追加聚焦与组合步骤：点击草稿输入后拖动 thumb 到中部，确认焦点仍是草稿输入；拖动回顶部后点击 Task 0 行内输入，用微软拼音输入 `nihao`，组合中按 Home、再点击轨道翻页与回翻，最后 Space 提交，确认按键与滚动条都不编辑、不显露、不取消组合。
- 完整状态见 [`证据/host-scrollbar/states.txt`](证据/host-scrollbar/states.txt)。

## 可见结果

### 100%（DPI 96，客户区 900×700）

| # | 步骤 | 预期 | 结果 | 截图 |
| --- | --- | --- | --- | --- |
| 1 | 启动 | `offset=0`，thumb 贴轨道顶部（`thumb=(888, 4, 8, 25.97)`） | 通过；轨道与 thumb 只在内容溢出时出现 | [01-top](证据/host-scrollbar/100-01-top.png) |
| 2 | 从 thumb 顶部拖到轨道中部 | 偏移与 thumb 位置同步，约在最大偏移的一半；两端都不越界 | 通过；`offset=9004`（max 17954），thumb y = 338.02 | [02-drag-middle](证据/host-scrollbar/100-02-drag-middle.png) |
| 3 | 继续拖到轨道底部之外再松开 | 偏移夹到 `max`，thumb 贴轨道底部 | 通过；`offset=17954`，thumb 底边 696.00 = 轨道底边 | [03-drag-bottom](证据/host-scrollbar/100-03-drag-bottom.png) |
| 4 | 点击 thumb 上方的轨道 | 向点击一侧翻一页（`floor(700 − 24) = 676`），焦点不变 | 通过；`offset=17278 = max − 676` | [04-track-page-up](证据/host-scrollbar/100-04-track-page-up.png) |
| 5 | Home → 滚轮 → PageDown → End → Home | 键鼠共享同一偏移：0 → 72 → 748 → 17954 → 0 | 通过 | — |
| 6 | 拖动结束后点击 Task 0 的 checkbox | 没有残留按压/悬停阻挡内容点击；checkbox 勾选、剩余数 499 | 通过；焦点与滚动偏移动画外不变 | [05-click-after-drag](证据/host-scrollbar/100-05-click-after-drag.png) |
| 7 | 点击草稿输入后拖动 thumb 到轨道中部 | 拖动不移动焦点；焦点保持草稿输入 | 通过；`offset=9004`，`focus=Some(NodeId(1)) "What needs to be done?"`，thumb y = 338.02 | [07-focused-drag](证据/host-scrollbar/100-07-focused-drag.png) |
| 8 | 拖动 thumb 回顶部，点击 Task 0 行内输入 | 偏移回到 0；焦点落到行内输入 | 通过；`offset=0`，`focus=Some(NodeId(22)) "Edit Task 0"` | — |
| 9 | 输入 `nihao` | 组合在行内开始，preedit 带下划线，模型值不变 | 通过；`composition=Some((0, 0))`，`preedit=Some("ni'hao")`，`value=""` | [08-composing-preedit](证据/host-scrollbar/100-08-composing-preedit.png) |
| 10 | 组合中按 Home | 不编辑、不显露、不滚动；组合与 preedit 保持 | 通过；无新状态行，焦点、`offset=0`、组合与 preedit 全部不变；截图与 08 的 sha256 一致 | [09-composing-home](证据/host-scrollbar/100-09-composing-home.png) |
| 11 | 组合中点击 thumb 下方轨道翻页，再点击上方轨道回翻后 Space | 轨道点击只滚动；组合存活、焦点不变；Space 提交 | 通过；翻页后 `offset=676` 且组合仍在；回翻后 `offset=0`；提交为 `value="你好"`、`composition=None` | [10-composing-track-page](证据/host-scrollbar/100-10-composing-track-page.png) · [11-composing-committed](证据/host-scrollbar/100-11-composing-committed.png) |

### 150%（DPI 144，客户区 1350×1050）

| # | 步骤 | 预期 | 结果 | 截图 |
| --- | --- | --- | --- | --- |
| 1 | 启动 | `offset=0`，逻辑几何与 100% 相同，物理坐标按 1.5 缩放 | 通过；轨道 `(888, 4, 8, 692)`，thumb `(888, 4, 8, 25.97)` | [01-top](证据/host-scrollbar/150-01-top.png) |
| 2 | 拖动到轨道中部 | 偏移约在最大偏移的一半 | 通过；`offset=8968`，thumb y = 336.68 | [02-drag-middle](证据/host-scrollbar/150-02-drag-middle.png) |
| 3 | 拖到轨道底部之外再松开 | 夹到 `max`，thumb 贴底 | 通过；`offset=17954`，thumb 底边 696.00 | [03-drag-bottom](证据/host-scrollbar/150-03-drag-bottom.png) |
| 4 | 点击 thumb 上方的轨道 | 翻一页，`offset = max − 676` | 通过；`offset=17278` | [04-track-page-up](证据/host-scrollbar/150-04-track-page-up.png) |
| 5 | 滚轮向上一档 → PageDown → End → Home | 17206 → 17882 → 17954 → 0 | 通过 | [05-home](证据/host-scrollbar/150-05-home.png) |
| 6 | 拖动结束后点击 Task 0 的 checkbox | 内容点击正常生效，无残留瞬态 | 通过；Task 0 勾选，剩余 499 | [06-click-after-drag](证据/host-scrollbar/150-06-click-after-drag.png) |
| 7 | 点击草稿输入后拖动 thumb 到轨道中部（物理坐标 ×1.5） | 拖动不移动焦点；逻辑几何与 100% 相同 | 通过；`offset=9004`，`focus=Some(NodeId(1)) "What needs to be done?"`，thumb y = 338.02 | [07-focused-drag](证据/host-scrollbar/150-07-focused-drag.png) |
| 8 | 拖动 thumb 回顶部，点击 Task 0 行内输入 | 偏移回到 0；焦点落到行内输入 | 通过；`offset=0`，`focus=Some(NodeId(22)) "Edit Task 0"` | — |
| 9 | 输入 `nihao` | 组合在行内开始，preedit 带下划线，模型值不变 | 通过；`composition=Some((0, 0))`，`preedit=Some("ni'hao")`，`value=""` | [08-composing-preedit](证据/host-scrollbar/150-08-composing-preedit.png) |
| 10 | 组合中按 Home | 不编辑、不显露、不滚动；组合与 preedit 保持 | 通过；无新状态行，截图与 08 的 sha256 一致 | [09-composing-home](证据/host-scrollbar/150-09-composing-home.png) |
| 11 | 组合中点击轨道翻页与回翻后 Space | 轨道点击只滚动；组合存活、焦点不变；Space 提交 | 通过；翻页后 `offset=676` 且组合仍在；回翻后 `offset=0`；提交为 `value="你好"`、`composition=None` | [10-composing-track-page](证据/host-scrollbar/150-10-composing-track-page.png) · [11-composing-committed](证据/host-scrollbar/150-11-composing-committed.png) |

关键状态（完整行见 `states.txt`）：

| 状态 | 100% | 150% |
| --- | --- | --- |
| 启动 | `offset=0`，thumb y=4，高 25.97 | 同左，scale 1.5 |
| 拖到中部 | `offset=9004`，thumb y=338.02 | `offset=8968`，thumb y=336.68（第一轮）；第二轮复跑为 9004，物理坐标 ×1.5 |
| 拖到底部 | `offset=17954=max`，thumb 底边=696 | 同左 |
| 轨道点击 | `17278 = max − 676` | 同左 |
| 滚轮/PageDown/End | 72 / 748 / 17954（随后 Home 归零） | 17206 / 17882 / 17954（随后 Home 归零） |
| 聚焦草稿后拖到中部 | `offset=9004`，焦点仍 `NodeId(1)` | 同左 |
| 组合开始（行内） | `offset=0`，`composition=Some((0, 0))`，preedit `ni'hao` | 同左 |
| 组合中 Home | 状态不变（无新行） | 状态不变（无新行） |
| 组合中轨道翻页 | `offset=676`，组合与焦点保持 | 同左 |
| Space 提交 | `value="你好"`，`composition=None` | 同左 |

## 证据等级与边界

- **可见运行（主证据）：** 真实 HWND、真实鼠标拖动/点击、真实滚轮与键盘消息、真实微软拼音、DX12 呈现；100% 与 150% 各一组截图和状态行，包含聚焦拖动保持焦点、组合中 Home 与轨道点击/翻页不编辑也不取消组合、Space 正常提交。
- **隐藏 smoke（部分证据）：** `dragging_the_scrollbar_thumb_scrolls_the_shared_offset_and_clamps`、`a_scrollbar_track_click_pages_towards_the_clicked_side`、`the_scrollbar_wins_hit_priority_over_the_content_under_it`、`wheel_drag_and_keys_share_one_clamped_offset_without_stale_transients` 覆盖夹取、轨道方向、命中优先与拖动后的 hover/press 清理。它们运行在隐藏窗口与 CI 的 WARP/软件适配器上，不能替代可见验收。
- **单元测试：** `pressing_the_scrollbar_pages_without_cancelling_a_composition` 覆盖滚动条命中不取消 IME 组合；`a_composing_editor_keeps_its_scroll_under_editing_keys` 覆盖组合中的 Home/End、方向键与 Backspace/Delete 不编辑、不显露、不滚动；`scrollbar_drag_offsets_follow_the_pointer_and_clamp` 覆盖拖动公式的夹取与无行程时保持偏移；渲染测试 `the_scrollbar_thumb_has_a_documented_minimum` 覆盖轨道容不下最小 thumb 或 thumb 会填满轨道时不绘制滚动条。
- **未覆盖：** 触摸拖动与触控板惯性；滚动条 hover/按下配色（当前覆盖层没有专用状态色）；混合 DPI 多显示器与真实硬件设备丢失仍属 #63。列表虚拟化已由 #184 交付，见[窗口化 For 验收](stage-three-窗口化-for-验收.md)。
