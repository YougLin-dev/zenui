# Stage-three 焦点显露验收（实机记录）

本页记录 #180 的可见 Windows 验收：宿主在焦点、caret 与 IME 组合变化后把目标显露到视口内，显式滚动不改变焦点/caret 也不触发显露，组合期间的候选窗继续以翻译后的 caret 定位。契约见 [ADR-0010](../adr/0010-scroll-is-presentation-state.md)；本页只覆盖 #180 当时交付的范围。

**后续状态（2026-11）：** 滚动条已由 #181/#182 交付（[滚动条验收](stage-three-滚动条-验收.md)），窗口化 `For` 已由 #184 交付（[窗口化 For 验收](stage-three-窗口化-for-验收.md)）。滚轮精度累积、hide/show、close/reopen 与模拟设备丢失的证据边界汇总在 [stage-three 滚动总验收](stage-three-滚动总验收.md)。

**状态：可见自动驱动验收完成（100% 与 150%）。** 运行使用真实可见 HWND、真实 `WM_MOUSEWHEEL` 路径、宿主默认 DX12 presenter 与真实微软拼音输入法；输入由 OS 级 `SendInput` 注入（鼠标移动、左键按下/松开、真实滚轮 notch、键盘按键），不是人工鼠标键盘。截图证明可见窗口的像素与场景状态，不证明用户注视下的 DWM 时序。

验收日期：2026-11-23。验收对象为 `feat/180-focus-reveal` 上 #180 的实现（提交见 PR）；运行后、提交前唯一的变化是删除临时示例探针。

## 环境

| 项 | 值 |
| --- | --- |
| 操作系统 | Windows 11 专业版 10.0.26300，64 位，交互式控制台会话 |
| 显示缩放 | 100%：`GetDpiForWindow=96`，客户区 900×700；150%：`GetDpiForWindow=144`，客户区 1350×1050 |
| 切换方式 | Settings > 显示 > 缩放（UI Automation）+ `rundll32 user32.dll,UpdatePerUserSystemParameters`；结束后恢复 150%（`GetDpiForSystem=144`） |
| 图形 | NVIDIA GeForce RTX 4060 Laptop GPU，驱动 32.0.15.7283；可见宿主使用默认 DX12 路径 |
| 显示 | 单屏 2560×1600 @ 240 Hz |
| 滚轮设置 | `WheelScrollLines=3`（一个 notch = 72 逻辑像素） |
| 输入法 | 微软拼音（`HKL=0x08040804`，TSF 候选窗） |
| 输入方式 | OS 级 `SendInput`：真实光标移动、左键按下/松开、`MOUSEEVENTF_WHEEL` notch、字母键、Tab、End、Space；窗口用标题栏点击激活并放到屏幕左上角 |

## 方法

- 临时探针（未提交）用公开的 `Runtime::input` / `submit` 在 `Host::open` 之前种入 500 行，用 `HostConfig::visible()` 运行宿主，并以 10ms 周期 `pump()`；场景/焦点/组合状态变化时打印一行 `state`：`offset`、`max`、`viewport`、`scale`、`elements`、`focus`、`label`、聚焦元素的翻译后 `rect`、`caret`、`composition`、`value` 与前三个完全可见的 `Edit Task` 输入中心。探针不绘制滚动条，也不注入额外 UI。
- 滚轮与键盘由外部 PowerShell 脚本通过 `SendInput` 注入；IME 由脚本按键驱动，使用系统中已启用的微软拼音。应用截图用 `PrintWindow(PW_CLIENTONLY|PW_RENDERFULLCONTENT)` 读取客户区，先写 BMP 再转 PNG；候选窗截图用桌面 `CopyFromScreen` 读取包含标题栏的窗口区域，因此 100% 下部分候选窗 PNG 为 905×739（客户区 900×700 加标题栏和少量桌面边缘），它们只用于证明候选窗位置，不参与客户区像素对比。
- 每个 DPI 下重复同一组步骤：End 到底、Tab 显露视口外草稿输入、滚轮把焦点滚出视口、点击行内输入、输入拼音、组合期间滚动、组合中 preedit 更新、把组合行滚出视口、滚回并 Space 提交、滚轮把焦点滚出视口后 Tab 到行内按钮。
- 两端都是 2519 个场景元素；内容 900×18654 逻辑像素；`max=17954`；一个 notch = 72 逻辑像素；Tab 显露的两次目标分别落在 `offset=44`（草稿输入）与 `offset=417`（行内 Delete）。完整状态见 [`证据/host-reveal/states.txt`](证据/host-reveal/states.txt)。

## 可见结果

### 100%（DPI 96，客户区 900×700）

| # | 步骤 | 预期 | 结果 | 截图 |
| --- | --- | --- | --- | --- |
| 1 | End | 底部 `offset=17954=max`，无焦点 | 通过 | [01-bottom-end](证据/host-reveal/100-01-bottom-end.png) |
| 2 | Tab | 焦点到视口外的草稿输入并按当前偏移显露；`offset=44`，`rect.y=0`，caret 在 (23, 5) | 通过；一次重建从 17954 落到 44（无第三段回落） | [02-tab-header-reveal](证据/host-reveal/100-02-tab-header-reveal.png) |
| 3 | 滚轮向下一档 ×5 | `offset=404`；聚焦草稿输入完整滚出视口（`rect.y=-360`，caret 缺失），焦点与 caret 不动 | 通过；不触发显露 | — |
| 4 | 点击行内输入 | 焦点 NodeId(52) "Edit Task 6"；caret 贴在可见行内 | 通过 | — |
| 5 | 输入 `nihao` | preedit `nǐhao` 带下划线显示在行内；`composition=Some((0, 0))`；模型 `value=""` | 通过；候选窗锚在翻译后的 preedit 起点 | [03-ime-preedit](证据/host-reveal/100-03-ime-preedit.png) · [04-ime-candidate](证据/host-reveal/100-04-ime-candidate.png) |
| 6 | 组合中滚轮向上一档 | `offset=332`；焦点、caret、`composition` 与 `value` 全部保持，不提交不取消 | 通过；行随内容下移 72 | [05-ime-scroll-during-composition](证据/host-reveal/100-05-ime-scroll-during-composition.png) |
| 7 | 组合中再输入一个字母 | preedit 增长；候选窗按滚动后的 caret 重新锚定（见「候选窗边界」） | 通过 | [06-ime-candidate-reanchor](证据/host-reveal/100-06-ime-candidate-reanchor.png) |
| 8 | 组合中滚轮向下一档 ×2 | `offset=476`；组合行完整离开视口（`rect.y=-59`，caret 缺失）；`composition` 仍存在，`value=""` | 通过；宿主发出组合/候选隐藏锚点 | [07-ime-row-offscreen](证据/host-reveal/100-07-ime-row-offscreen.png) · [08-ime-offscreen-candidate](证据/host-reveal/100-08-ime-offscreen-candidate.png) |
| 9 | 滚回一档后 Space | `offset=332`；组合提交为 `你好啊`，`composition=None`，模型只写入一次 | 通过 | [09-ime-committed](证据/host-reveal/100-09-ime-committed.png) |
| 10 | 滚轮向下一档 ×10 | `offset=1052`；焦点仍是 NodeId(52)，`rect.y=-635`，caret 缺失 | 通过；滚动不移焦 | — |
| 11 | Tab | 焦点移到行内 NodeId(53) "Delete Task 6"，显露到 `offset=417`，按钮 `rect.y=0`；元素数仍 2519 | 通过；行未重挂载 | [10-tab-row-reveal](证据/host-reveal/100-10-tab-row-reveal.png) |

### 150%（DPI 144，客户区 1350×1050）

| # | 步骤 | 预期 | 结果 | 截图 |
| --- | --- | --- | --- | --- |
| 1 | End | 底部 `offset=17954=max`，无焦点 | 通过 | [01-bottom-end](证据/host-reveal/150-01-bottom-end.png) |
| 2 | Tab | 草稿输入显露到 `offset=44`；逻辑几何与 100% 相同，物理坐标按 1.5 缩放 | 通过；caret 在 (23, 5) | [02-tab-header-reveal](证据/host-reveal/150-02-tab-header-reveal.png) |
| 3 | 滚轮向下一档 ×5 | `offset=404`；草稿输入 `rect.y=-360`，caret 缺失，焦点保持 | 通过 | — |
| 4 | 点击行内输入并输入 `nihao` | 焦点 NodeId(52) "Edit Task 6"；preedit 带下划线，候选窗锚在翻译后的 caret | 通过 | [03-ime-preedit](证据/host-reveal/150-03-ime-preedit.png) · [04-ime-candidate](证据/host-reveal/150-04-ime-candidate.png) |
| 5 | 组合中滚轮向上一档 | `offset=332`；组合、焦点、caret 保持 | 通过 | [05-ime-scroll-during-composition](证据/host-reveal/150-05-ime-scroll-during-composition.png) |
| 6 | 组合中再输入一个字母 | 候选窗按滚动后的 caret 重新锚定 | 通过 | [06-ime-candidate-reanchor](证据/host-reveal/150-06-ime-candidate-reanchor.png) |
| 7 | 组合中滚轮向下一档 ×2 | `offset=476`；组合行完整离开视口，`composition` 仍存在，`value=""` | 通过 | [07-ime-row-offscreen](证据/host-reveal/150-07-ime-row-offscreen.png) · [08-ime-offscreen-candidate](证据/host-reveal/150-08-ime-offscreen-candidate.png) |
| 8 | 滚回一档后 Space | 组合提交为 `你好啊`，`composition=None` | 通过 | [09-ime-committed](证据/host-reveal/150-09-ime-committed.png) |
| 9 | 滚轮向下一档 ×10 后 Tab | 焦点从 NodeId(52) 移到 NodeId(53)，显露到 `offset=417`，按钮 `rect.y=0` | 通过；元素数仍 2519 | [10-tab-row-reveal](证据/host-reveal/150-10-tab-row-reveal.png) |

关键状态（完整行见 `states.txt`）：

| 状态 | 100% | 150% |
| --- | --- | --- |
| End | `offset=17954=max`，focus None | 同左，scale 1.5 |
| Tab 显露草稿输入 | `offset=44`，`rect.y=0`，caret (23, 5) | 同左 |
| 滚轮 −5 后草稿输入 | `offset=404`，`rect.y=-360`，caret 缺失，焦点保持 | 同左 |
| 组合开始（行内） | `offset=404`，`composition=Some((0, 0))`，`value=""` | 同左 |
| 组合中滚轮 | `offset=332`，组合与 caret 保持 | 同左 |
| 组合行完整离屏 | `offset=476`，`rect.y=-59`，caret 缺失，组合仍在 | 同左 |
| Space 提交 | `value="你好啊"`，`composition=None` | 同左 |
| 焦点滚出后 Tab | `offset=417`，焦点 NodeId(53)，`rect.y=0` | 同左 |

## 候选窗边界

宿主在每次场景重建后用 preedit 起点 caret 的物理客户区坐标调用 `ImmSetCompositionWindow` / `ImmSetCandidateWindow`；caret 完全离开视口时把两个锚点移到工作区外。可见验收中确认了以下 IME 侧行为：

- 组合开始时候选窗锚在翻译后的 preedit 起点，滚动偏移与 DPI 都参与换算（截图 04）。
- 微软拼音（TSF 候选窗）在**同一次组合内缓存候选窗的屏幕位置**：只滚动、不改 preedit 时，候选窗停留在旧锚点（截图 05/08），宿主虽然已经发出新锚点（隐藏时是工作区外的锚点），IME 要到下一次 preedit 更新才重排；下一次 preedit 更新会立即把候选窗移到滚动后的 caret（截图 06）。
- 为了让 IME 重读锚点，验收中尝试过 `NI_CONTEXTUPDATED`、`NI_CLOSECANDIDATE`/`NI_OPENCANDIDATE`、向默认 IME 窗口发送 `WM_IME_CONTROL`（`IMC_SETCOMPOSITIONWINDOW` / `IMC_SETCANDIDATEPOS`）以及隐藏默认 IME 窗口；这些调用都不改变 TSF 候选窗在本次组合内的缓存位置。宿主保留 IMM32 的定位/隐藏锚点（单元测试覆盖锚点与隐藏决策），不取消组合、不切换 IME 状态。TSF 文本存储属于未来原生适配，不在本工单。
- 组合行完整离屏时应用侧 preedit 与 caret 装饰按视口裁剪消失（截图 07），组合状态与模型值保持，滚回后继续同一次组合。

## 证据等级与边界

- **可见运行（主证据）：** 真实 HWND、真实滚轮/键盘消息、真实微软拼音与 DX12 呈现；100% 与 150% 各一组截图和状态行。
- **隐藏 smoke（部分证据）：** `tab_reveals_an_offscreen_focus_target_and_restores_the_top_window`（#184 后改名，覆盖随窗口化列表显露时把窗口带回顶部）与 `an_edit_after_wheel_scrolling_reveals_the_focused_input` 覆盖焦点/caret 显露与“滚动后下一次编辑再显露”，并在 CI 的 WARP/隐藏窗口里运行；它们不能替代可见验收。
- **单元测试：** `reveal_offsets_are_minimal_vertical_moves` 覆盖上/下越界、完全可见、超高元素顶部优先、两端夹取与分数取整；`ime_anchors_follow_the_translated_caret_and_clamp_into_the_client_area` 与 `a_caret_that_leaves_the_viewport_hides_the_candidate` 覆盖候选锚点的 DPI 换算、客户区夹取与隐藏决策。
- **未覆盖：** Shift+Tab（与 Tab 共用焦点遍历与显露路径，由隐藏 smoke 覆盖）；多行 preedit 的锚点与工作区夹取（demo 只有单行输入）；候选窗在 TSF IME 下的即时隐藏（见「候选窗边界」）；滚动条 #181/#182 与列表虚拟化已由后续工单交付，分别见对应验收页；混合 DPI 多显示器与真实硬件设备丢失仍属 #63。
