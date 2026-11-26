# Stage-two Windows 验收（实机记录）

本页记录阶段二 Windows 原生切片的实机运行结果，并对照 #60、#63 的验收范围。

**状态：部分完成。** 可见窗口、真实 GPU、150% 缩放的自动驱动运行已完成并保留截图；
第二轮人工试跑发现的问题已修复并复验（见「人工试跑反馈与修复」）。维护者报告已用
真实鼠标、键盘与微软拼音自测 #60 的原生输入与中英文编辑清单，报告未发现异常
（见「人工验收」；本次未提供截图、录屏或逐项观察记录）；跨显示器混合 DPI 与实机
device loss 仍待确认（#63）。样式里程碑的第四轮自动驱动验收记录了按下、拖出、松开
激活、`focus-visible:` 与深色主题的可见窗口截图（125%）；第五轮（#287）在 100% 与
150% 下自动驱动 hover、按下/拖出/松开、`focus-visible:` 与深色主题并保留截图与读数
（见「100% 与 150% 的样式状态」）。维护者随后按同一清单在真实桌面上
自测 hover、按下/拖出/松开、`focus-visible:` 与深色主题，并在 100% 与 150% 缩放下
复测，报告未发现异常（未提供截图、录屏或逐项记录，见「维护者自测报告」）。
本页不把脚本投递的原生消息当作人工操作，也不把 headless 或 CI WARP 结果当作原生呈现验收。

验收日期：2026-11-21。首轮验收对象为 `edd0383`（#80 合并后的 `main`）与本分支的
文本测量修复；第二轮为 `fix/manual-acceptance` 上的人工试跑反馈修复。命令：

```sh
cargo run -p zenui-windows --example todo_host --locked
```

## 环境

| 项 | 值 |
| --- | --- |
| 操作系统 | Windows 11 专业版 10.0.26300，64 位，交互式会话 |
| GPU | NVIDIA GeForce RTX 4060 Laptop GPU，驱动 32.0.15.7283 |
| 图形后端 | DX12，`DeviceType::DiscreteGpu` |
| 显示 | 单显示器，150% 缩放（`GetDpiForWindow` = 144） |
| 输入语言 | `HKCU\Keyboard Layout\Preload` 为 `00000804`（中文，简体） |

GPU 型号与后端由一次性的 `adapter.get_info()` 探针确认，探针未保留在代码中；宿主按
`PowerPreference::HighPerformance` 先请求硬件适配器，失败才回退 WARP。

## 方法

- 宿主以可见窗口运行，同一 Todo 组件树与 headless 示例一致（`zenui-demo`）。
- 输入用 PowerShell 向 HWND 投递 `WM_LBUTTONDOWN`、`WM_CHAR`、`WM_KEYDOWN`（Enter）
  和 `WM_CLOSE`，走与隐藏窗口 smoke 相同的原生处理路径；IME 与交互式缩放的截图分别用
  `keybd_event`（真实微软拼音引擎）和 `mouse_event`（真实边框拖动）产生。两者都不是
  人工操作，也不代替人工清单。
- 截图是客户区物理像素：初始 1350×1050（900×700 逻辑 × 1.5），resize 后 960×720
  （640×480 逻辑 × 1.5）。截图来自三次独立的可见运行（会话 A/B/C），不是一次
  连续操作；下表的会话列说明每个状态属于哪一次运行。07–13 来自第二轮及其修复后的
  独立运行；12 与 13 是松开鼠标之前截取的客户区（12 为拖动中的 900×678 物理客户区）。

## 可见窗口结果

| 会话 | 步骤 | 结果 | 截图 |
| --- | --- | --- | --- |
| A | 启动 | 900×700 逻辑布局在 150% 缩放呈现；文字清晰、无裁剪；种子任务与 "1 remaining" 可见 | [01](证据/stage-two/01-visible-baseline.png) |
| A | Active 筛选 | 已完成行隐藏，仅剩 Read docs；Active 按钮高亮 | [02](证据/stage-two/02-active-filter.png) |
| A | 回到 All、勾选 Write tests | All 还原两行；勾选后 Write tests 取消完成，计数变为 2 remaining | — |
| A | 异步任务 | 点击 Load sample 后不再输入，窗口自行重绘为 4 remaining 并出现 Imported A/B，验证 #80 的唤醒路径在可见窗口生效 | [04](证据/stage-two/04-async-load-sample.png) |
| B | 输入与提交 | 点击草稿框后输入 "Milk" 逐个出现；Enter 新增 Milk 行、清空草稿、计数变为 2 remaining | [03](证据/stage-two/03-typing-and-enter.png) |
| B | 重排 | 同一会话中 Reverse 后顺序变为 Milk、Read docs、Write tests；行内容与像素无残留 | [05](证据/stage-two/05-reverse-order.png) |
| C | resize | 新会话启动后把客户区调整为 960×720 物理像素（640×480 逻辑）；按钮与文本不裁剪、不拉伸 | [06](证据/stage-two/06-resize-640x480.png) |
| — | 关闭 | 每次运行都以 `WM_CLOSE` 结束消息循环；重新启动都是新的窗口与种子状态 | — |

截图 05 拍摄于 caret 聚焦修复之前，每个 text-input 都绘制了 caret；修复后只有聚焦
输入绘制 caret，并由定时器闪烁（证据 10/11）。焦点与 caret 的连续性仍由隐藏 smoke
`reorder_keeps_focus_and_caret_on_the_same_row` 按 Node ID 与 caret x 断言。

## 验收发现与修复

**标签断行。** 在 150% 缩放下，"Load sample" 与当时的 "Preview import"（#152 后为 "Import file"）被拆成两行并
溢出按钮边框。原因不在 DPI 换算：Taffy 把叶子尺寸取整到整像素，而 Parley 按精确
的 max-content 宽度断行，测量宽度比最终布局少不足一个像素时就会拆行。修复让
`TextShaper::measure` 把 intrinsic 宽度向上取整到像素网格，并恢复 `shape` 只按
分配宽度断行；回归测试为
`crates/render/src/text.rs::measured_widths_land_on_the_pixel_grid` 与
`crates/render/tests/scene.rs::long_toolbar_labels_stay_inside_their_buttons`。

**带 padding 的文本测量宽度。** 同一轮评审发现 `measure_leaf` 把 Taffy 的
border-box known dimension 直接交给文本测量，而最终绘制按 content-box 宽度断行；
带 padding/border 的 text-input 因此少算行数，长值会把文本和 caret 画到输入框
下方。修复在测量前减去 padding 与 border，由 `compute_leaf_layout` 再加回边框盒。
回归测试 `crates/render/tests/scene.rs::a_long_value_stays_inside_its_input`
在窄视口下断言所有行与 caret 都在输入框内。

这两个缺陷都没有被既有测试覆盖：隐藏窗口 smoke 与离屏像素测试没有断言标签是否
在按钮内断行，也没有覆盖窄窗口下的长输入值。修复后的截图即上表 01–06。

## 人工试跑反馈与修复（第二轮）

第二轮人工试跑报告了四个问题；其中三个属于本切片的缺陷，已修复并复验：

**中文只显示草稿，候选确认后仍是草稿。** 微软拼音在候选确认时把提交字符作为
`WM_CHAR` 送达，而宿主只在非组合状态调用 `insert_text_node`，组合中的字符被
reducer 拒绝，模型从未写入。修复让组合中的 `WM_CHAR` 走 `compose_commit_node`。
第二轮人工试跑又发现草稿被画了两层：应用绘制 preedit，输入法又在自己的组合窗口里
画了一份拼音（白色浮层）。窗口过程现在消费 `WM_IME_STARTCOMPOSITION` 而不交给
默认过程，微软拼音不再绘制自己的组合窗口；preedit 由场景按输入框的普通文本样式
绘制，候选窗仍由系统绘制并锚在 preedit 起点。真实微软拼音配合合成按键的证据见
[07](证据/stage-two/07-ime-preedit-with-candidates.png)（单一 preedit，候选窗）
与 [08](证据/stage-two/08-ime-committed.png)（提交为“中文”）。

**Escape 取消失败。** 微软拼音吞掉 Escape 后只送达 `VK_PROCESSKEY`，不发送
`WM_IME_ENDCOMPOSITION`，本地组合因此残留。修复在 `VK_PROCESSKEY` 时比较本地
组合状态与 `ImmGetCompositionStringW(GCS_COMPSTR)`：本地仍在组合而 OS 已无组合串
时补一次取消。证据 [09](证据/stage-two/09-ime-cancelled.png)：取消后输入框仍是
已提交的“中文”。

**每个输入框常显不闪烁的 caret。** 场景原先为每个 text-input 生成 caret 装饰，
GPU 全部绘制。修复让 caret 只属于聚焦输入，并由 `SetTimer`（530ms）驱动闪烁；
组合中、失去前台或没有聚焦输入时不闪烁。证据
[10](证据/stage-two/10-caret-blink-off.png) 与
[11](证据/stage-two/11-caret-blink-on.png) 是同一输入框相隔约 220ms 的无 caret /
有 caret 两帧。

**交互式缩放整页拉伸。** 拖动窗口边框时，系统在自己持有的模态缩放循环里直接调用
窗口过程，宿主的消息循环在这段时间收不到 `WM_SIZE`/`WM_PAINT`，于是窗口只显示被
DWM 拉伸的最后一帧。修复把消息处理移进窗口过程：`run`/`pump` 只负责取消息与派发，
期间不持有宿主引用；窗口过程通过 `GWLP_USERDATA` 取回宿主，处理输入并在 `WM_SIZE`
到达时同步按新客户区重排并呈现，`WM_PAINT` 只处理有实际更新区域的失效。
回归测试 `crates/windows/src/host.rs::a_paint_message_repaints_after_an_external_resize`
在消息循环之外改窗口尺寸并派发 `WM_PAINT`，断言场景按新的客户区重排；证据
[12](证据/stage-two/12-live-resize-during-drag.png) 是拖动过程中（松开鼠标之前）
截取的画面，布局已按新尺寸重排且文字未缩放。

**快速拖动时文字先拉伸再回弹。** 同步重绘缩短了拉伸帧的存续时间，但没有消除它：
经典 HWND 交换链由 DWM 按 `DXGI_SCALING_STRETCH` 把上一帧缩放到新客户区，任何两次
呈现之间的尺寸变化都会先显示拉伸帧，快速拖动时尤其明显，慢速拖动几乎看不出来。用
真实边框拖动、逐帧读取客户区里 "1 remaining" 的暗像素跨度量化（正常跨度 [26,131]）：
经典交换链快速拖动约 200 帧中约 20 帧偏离（最大到 135）。修复把 DX12 instance 配置
为 `Dx12SwapchainKind::DxgiFromVisual`，surface 由 DirectComposition 视觉承载，DWM
合成的是视觉内容而不是上一帧位图；同一探针 190 帧全部保持 [26,131]，没有异常帧。
代价是 RenderDoc 无法抓取这类交换链。证据
[13](证据/stage-two/13-directcompose-drag.png) 是 DirectComposition 下拖动过程中
（松开鼠标之前）的客户区截图。

第四个问题“按钮没有 hover 等状态效果”不在本切片的范围：spec #68 明确本阶段不做
class 样式，状态样式属于路线图的样式里程碑，已记录为 #96。#96 已由 #113 关闭，
class 状态变体与深色主题随后由 #110、#114 完成；本轮的可见窗口证据见下一节。

## 人工验收（第三轮）

2026-11-21，维护者在可见窗口上用真实鼠标、键盘与微软拼音自测了 #60 的验收清单，
报告「我测过了，没有问题」。本次未提供截图、录屏或逐项观察记录；下面只记录清单
覆盖的检查项，不对每项结果作独立断言：

- 原生工作流：键盘输入 `Milk` 后 Enter 新增并清空草稿；勾选复选框；
  All/Active/Done 过滤；Reverse 重排；点击行内 Delete 删除（此前只在隐藏 smoke
  中覆盖）；Tab/Shift+Tab 遍历焦点。
- 键盘编辑：←/→/Home/End 移动 caret，Shift+方向键选择，Ctrl+A 全选，
  Backspace/Delete 编辑。
- 中文 IME（草稿框与行编辑框各一次）：preedit 层数、候选窗相对组合起点的位置、
  Space 选词、Escape 取消、组合中与组合后的 Enter 行为。具体行为的既有证据是
  07–09（真实 IME + 合成按键）。
- 焦点连续性：聚焦行编辑框后 Reverse 与切换过滤时焦点/caret 的落点。行被过滤时
  运行时结算到仍可聚焦的控件，而不是停在已卸载的行；该行为由
  `focus_survives_reorder_and_settles_after_delete` 与
  `filtering_out_the_focused_row_does_not_let_the_remount_steal_focus` 断言。

## 样式里程碑的可见窗口验收（第四轮）

2026-11-21，针对 #115 的样式里程碑（class 词表、宿主瞬态与深色主题）在可见窗口上
做了一轮自动驱动验收。**状态：自动驱动完成（125%，以及 100% 与 150% 的第五轮）；维护者人工自测报告通过。** 按下、拖出后松开、松开激活、`focus-visible:` 与深色主题切换有可见窗口截图；hover 与 100%/150% 缩放的自动驱动运行见「100% 与 150% 的样式状态」。维护者随后在真实桌面上按清单自测，包括 hover 与 100%/150% 缩放，报告未发现异常（见「维护者自测报告」）。

### 环境与方法

- 与第二、三轮同一台机器：Windows 11 10.0.26300，RTX 4060 Laptop GPU，DX12；
  单显示器。本轮 `GetDpiForWindow` = 120，即 **125%** 缩放；第二、三轮记录的是
  150%，因此本节的截图不能当作 100% 或 150% 的证据；100% 与 150% 的第五轮见下一节。
- 源码为 #114 合并后的分支，验收提交见本节所在 PR；构建命令
  `cargo build -p zenui-windows --example todo_host --locked`，窗口标题
  `ZenUI Todo`，客户区 1125×875 物理像素。
- 录制期间共享桌面正在被使用：窗口可见但不是前台，且被置于 z-order 底部，前台应用
  保持焦点。状态由 `PostMessage` 投递原生鼠标/键盘消息（与隐藏 smoke 相同的窗口
  过程路径），不是人工操作；画面用 `PrintWindow(PW_CLIENTONLY|PW_RENDERFULLCONTENT)`
  读取窗口客户区，不是桌面截屏。因此这些截图证明可见原生窗口、真实 GPU 与真实窗口
  过程上的状态和像素，不证明用户注视下的完整 DWM 呈现时序，也不等于人工试跑。
- hover 由真实光标触发并依赖 `TrackMouseEvent`。第四轮录制期间光标无法稳定停在窗口上（共享桌面持续有其它输入），未取得 hover 截图；hover 的像素与命中行为由自动测试覆盖（见下）。第五轮在 100% 与 150% 下用 `SendInput` 稳定光标并取得了 hover 截图（见「100% 与 150% 的样式状态」）。

### 可见窗口结果

| # | 步骤 | 预期 | 结果 | 截图 |
| --- | --- | --- | --- | --- |
| 1 | 启动后的明亮基线 | All 选中，两条种子任务 | 与隐藏 smoke 的种子状态一致 | [01](证据/styling/01-visible-baseline-125.png) |
| 2 | 在 Reverse 上按下并保持 | 内建按钮变深（`control_pressed`），不立即激活 | 底色变深，顺序未变 | [02](证据/styling/02-pressed-reverse-125.png) |
| 3 | 按住后拖到空白处松开 | 不激活 | 顺序仍为 Write tests、Read docs | [03](证据/styling/03-drag-off-no-activation-125.png) |
| 4 | 在 Reverse 上按下并松开 | 松开时恰好激活一次 | 顺序变为 Read docs、Write tests | [04](证据/styling/04-release-activates-125.png) |
| 5 | 指针点击草稿输入框 | 获得焦点与 caret，不匹配 `focus-visible:` | 边框保持默认色 | [05](证据/styling/05-focus-pointer-125.png) |
| 6 | 点击空白后按 Tab（键盘模态） | 草稿框获得键盘焦点，显示 accent 边框 | 边框变为 accent 色 | [06](证据/styling/06-focus-visible-125.png) |
| 7 | 点击 Theme | 切换深色调色板，焦点与 caret 保留；指针点击切回指针模态 | 深色呈现；草稿框仍聚焦，但 `focus-visible:` 不再匹配，边框回到默认色 | [07](证据/styling/07-dark-theme-125.png) |

`focus-visible:` 的前后对照（05 与 06）使用同一控件与同一 `focus-visible:border-accent`
声明，只改变输入模态；07 显示外观切换不重组界面树，同时说明键盘模态是宿主瞬态：
点击 Theme 后焦点仍在草稿框，但 accent 边框按预期消失。

### 100% 与 150% 的样式状态（#287，2026-11-26）

在单显示器上用 DisplayConfig 的当前 DPI 相对档位临时切换缩放：先跑 150%（144 DPI），再跑 100%（96 DPI），运行后恢复 150%。每个缩放都启动 `todo_host`、以前台窗口接收真实 `SendInput` 鼠标与 Tab，并用 UI Automation 读取控件矩形、以 `CopyFromScreen` 采集客户区。脚本位于仓库外，未提交；时间戳与完整读数见 [100-150-states.txt](证据/styling/100-150-states.txt)。

| # | 步骤 | 100% 读数 | 150% 读数 | 截图 |
| --- | --- | --- | --- | --- |
| 1 | 明亮基线 | Reverse 底色 `#E4E4E7`，页面 `#FFFFFF`，顺序 Write tests、Read docs | 同色值 | [100-01](证据/styling/100-01-baseline.png) / [150-01](证据/styling/150-01-baseline.png) |
| 2 | 光标停在 Reverse | `#D4D4D8`（hover 变深），顺序不变 | 同色值 | [100-02](证据/styling/100-02-hover.png) / [150-02](证据/styling/150-02-hover.png) |
| 3 | 按下并保持 | `#A1A1AA`（pressed），顺序不变 | 同色值 | [100-03](证据/styling/100-03-pressed.png) / [150-03](证据/styling/150-03-pressed.png) |
| 4 | 拖到空白处 | `#E4E4E7` 复原；松开不激活，顺序仍 Write tests、Read docs | 同色值 | [100-04](证据/styling/100-04-drag-off.png) / [150-04](证据/styling/150-04-drag-off.png) |
| 5 | 在 Reverse 上按下并松开 | 顺序变为 Read docs、Write tests | 同色值 | [100-05](证据/styling/100-05-release.png) / [150-05](证据/styling/150-05-release.png) |
| 6 | 指针点击草稿框 | 焦点为草稿框；左边框 `#C3C3C8`，不匹配 accent | 左边框 `#B2B2B9`，不匹配 accent | [100-06](证据/styling/100-06-focus-pointer.png) / [150-06](证据/styling/150-06-focus-pointer.png) |
| 7 | 点击空白后按一次 Tab | 草稿框键盘焦点；左边框 `#85A4E9`（accent） | 左边框 `#5583EA`（accent） | [100-07](证据/styling/100-07-focus-visible.png) / [150-07](证据/styling/150-07-focus-visible.png) |
| 8 | 点击 Theme | 页面 `#18181B`；焦点仍在草稿框，`focus-visible:` 不再匹配 | 同色值 | [100-08](证据/styling/100-08-dark.png) / [150-08](证据/styling/150-08-dark.png) |

两种缩放下按钮的三档底色一致，说明颜色来自场景的已解析状态而不是 DPI 缩放；边框采样值不同是因为 1 物理像素的边框在 150% 下覆盖更多物理像素、抗锯齿不同，但 accent 与默认边框的区分在两种缩放下都成立。

### 自动覆盖（不能代替实机）

- hover/press 像素：`crates/render/tests/gpu.rs::hovering_and_pressing_a_control_change_its_pixels`；
  100%/150% 文本与绘制：`text_classes_render_at_both_scales_without_clipping`；
  深色调色板：`the_dark_palette_paints_dark_surfaces`。
- 原生消息路径：`crates/windows/tests/hidden_smoke.rs` 的
  `hover_press_and_release_activate_a_filter_button_once`、`dragging_off_cancels_activation`、
  `hover_does_not_change_focus_or_caret`、`a_relayout_keeps_hover_consistent_with_the_hit_test`、
  `focus_visible_follows_keyboard_not_pointer_modality`、
  `a_configured_dark_appearance_paints_and_the_toggle_preserves_edit_state`。
- Windows CI 的隐藏 smoke 运行在 WARP 上；即使像素断言通过，也不构成本节的可见
  窗口验收。
- 100% 与 150% 的自动驱动运行（#287）在单显示器上通过 DisplayConfig 临时切换缩放完成并恢复；它证明两种缩放下的可见像素与控件状态，不覆盖跨显示器混合 DPI 或运行中切换缩放的 DWM 时序（#63）。

### 维护者自测报告（第五轮，2026-11-22）

维护者在真实 Windows 桌面上按本清单自测，报告「按清单测试都是没问题的」。覆盖：

- hover：FilterButton 的 `hover:` 与无 class 内建按钮的 intrinsic hover 都可见，移出
  窗口后清除；hover 不改变焦点或 caret。
- 按下/拖出/松开：按住时按钮变深；拖出后松开不激活；在同一按钮上按下并松开恰好
  激活一次。
- `focus-visible:`：鼠标点击与 Tab 聚焦的边框不同；指针点击后 accent 边框消失。
- 深色主题：切换后焦点、caret、selection 与输入内容保留；深色下重复 hover、按下与
  Tab 焦点检查，对比度可接受。
- 缩放：在 100% 与 150% 显示缩放下复测以上项目，无裁剪，文字与 caret 对齐。

本次报告未提供截图、录屏或逐项观察记录，证据级别与 #60 的「维护者自测报告」相同：
它记录维护者的实机结论，不等同于可复跑的自动证据。

### #96 关闭理由

#96 要求宿主跟踪 hover/press、按调色板绘制内建控件状态、按下改为松开激活，并保持
`aria:pressed` 与按下瞬态分离。这些由 #113 完成（宿主瞬态、`SceneEnv`、松开激活与
拖出取消）；#96 明确排除的 class 状态变体与深色主题分别由 #110（条件声明）与
#114（应用外观与深色调色板）完成。因此 #96 随 #113 的 PR 关闭。设计与取舍见
[ADR-0006](../adr/0006-presentation-transients-and-style-resolution.md)。

### 人工清单状态

hover、按下/拖出/松开、`focus-visible:`、深色主题与 100%/150% 缩放的清单已由维护者
自测，报告无异常；#287 补齐了 100% 与 150% 下可复跑的自动截图与读数（见「100% 与 150% 的样式状态」）。跨显示器混合 DPI 与实机 device loss 仍属 #63，不在此清单内。

## 已知限制

窗口窄于工具栏内容宽度时，toolbar 行现在会换行（通用 `flex-wrap` 布局能力），
右侧按钮不再被裁掉；420×700 与 900×700 的 100%/150% 截图见
[stage-two 工具栏换行验收](stage-two-工具栏换行-验收.md)。

## 判据对照（#81）

| 判据 | 状态 | 证据 |
| --- | --- | --- |
| 同一 Todo 在可见 Windows 11 窗口、真实 GPU 上运行 | 通过 | 上表 01–05；DX12 离散 GPU |
| 完整工作流（增删、勾选、筛选、重排） | 通过（自动 + 维护者自测报告） | 可见自动运行 01–05；隐藏 smoke `delete_removes_a_native_row`；「人工验收」（报告含 Delete，无截图） |
| 中文 IME 组合、提交、取消与候选位置 | 通过（真实 IME + 合成按键；维护者自测报告） | 07–09；宿主单元测试覆盖 `WM_IME_*` 路由；「人工验收」（报告无异常，未逐项确认候选位置） |
| 焦点与 Tab 顺序 | 通过（自动 + 维护者自测报告） | `tab_moves_focus_to_the_next_control`、`shift_tab_moves_focus_backward`；「人工验收」（报告无异常） |
| resize | 通过（程序化、交互式拖动与快速拖动探针） | 06、12、13；`resizing_relayouts_the_scene_and_surface`、`a_paint_message_repaints_after_an_external_resize` |
| 混合 DPI 移动 | 部分（单显示器缩放切换已预检通过；跨屏移动未执行） | [混合 DPI 检查](windows-混合dpi-检查.md)「单显示器预检」；跨屏步骤同页 |
| 关闭/重开为新 activation，无旧任务或旧呈现 | 自动通过 | `closing_the_window_cancels_window_owned_tasks`、`reopening_uses_a_new_activation_and_discards_the_old_delivery`、`closing_the_activation_prevents_stale_wakes_and_delivery` |
| 重排与筛选后无残留焦点、caret 或像素 | 通过（像素可见；焦点/caret 自动 + 维护者自测报告） | 上表 02、05 验证顺序与像素；10/11 验证 caret 只随焦点出现并闪烁；`reorder_keeps_focus_and_caret_on_the_same_row` 按 Node ID 与 caret x 断言；「人工验收」（报告无异常） |
| device/surface 重建 | 自动通过，实机未触发 | `a_surface_rebuild_preserves_focus_and_caret`；`Lost` 路径未在实机强制触发 |
| 证据区分 headless、离屏、WARP 与实机 | 通过 | 本页与 `docs/SDK/窗口与任务.md`、ADR-0005 |
| 更新验收文档并确认 #60/#63 覆盖 | 通过 | 本页下方两节 |

「维护者自测报告」指 2026-11-21 的口述结果：维护者报告清单无异常，未提供截图、
录屏或逐项记录；它不等同于可复跑的自动证据。

## #60 覆盖

| #60 验收 | 证据 | 状态 |
| --- | --- | --- |
| 同一 `examples/minimal_todo.rs` 的 Todo 挂载到真实 Windows 窗口 | `zenui-windows::Host` 挂载 `zenui-demo` 的 TodoApp；上表 01–05 | 通过 |
| add/toggle/filter/reorder/delete 经原生输入进入同一 dispatch 路径，无第二棵 UI 树 | 隐藏 smoke、可见自动运行与维护者自测（含 Delete，无截图）；宿主只保存 Node ID 派发，不保存 caret/值（`docs/SDK/窗口与任务.md`） | 通过 |
| headless 与 native 共享 target runtime，且不互相冒充完成 | 两边使用同一 `Runtime`/组件树；本页明确分区 | 通过 |

#60 的验收：前两条有实机运行记录（可见窗口与真实 GPU：01–05；真实鼠标/键盘工作流：
维护者自测报告，含 Delete，无截图或逐项记录），第三条由 headless 与 native 共用同一
Runtime/组件树、本页分区说明支持，属代码与文档事实而非实机证据。文档合并后关闭 #60。

## #63 覆盖

| #63 验收 | 证据 | 状态 |
| --- | --- | --- |
| 同一个 target Todo 在真实 Windows 窗口呈现，并在 add/toggle/filter 后更新 | 上表 01–05；`zenui-render` 的 Scene/GPU 路径 | 通过 |
| DPI 变化与 device/surface 重建保留挂载身份，无旧呈现 | 150% DPI 实机呈现；单显示器缩放切换预检；resize 与重建测试；跨显示器变化未执行 | 部分 |
| 不支持的能力显式报错，不用 validation panic 代替 | 不支持的标签/属性返回 `SceneError`；offscreen 格式返回 `RenderError::UnsupportedFormat`；Windows surface 只接受非 sRGB `Bgra8Unorm`（`HostError::Surface`） | 通过 |
| headless 测试仍是逻辑测试，不推断像素/native 结论 | 本页分级 | 通过 |

#63 仍保持打开：跨显示器混合 DPI 移动与实机 device loss 尚无证据（单显示器缩放切换
已预检通过，只覆盖同屏 DPI 变化）。

## 仍需人工执行

1. 按[混合 DPI 检查](windows-混合dpi-检查.md)在两块不同缩放率显示器间移动窗口。
   单显示器上的运行中缩放切换已于 2026-11-21 预检通过，但它不覆盖跨屏移动与拓扑变化。
2. 若可触发，记录真实 device loss（例如驱动重置）后的重建与重绘；否则保留自动化
   重建测试的覆盖说明。

## 结论

已在真实 Windows 11、离散 GPU、150% 缩放的可见窗口上验证：同一 Todo 正常呈现，
原生消息驱动输入/添加/勾选/筛选/重排/异步更新，后台任务无需轮询即可重绘，resize
重新布局且不裁剪；实机运行与随后的评审发现并修复了标签断行和带 padding 的
text-input 测量宽度两个缺陷，并各自补了回归测试。

第二轮人工试跑又发现并修复了中文候选提交、Escape 取消、caret 聚焦与闪烁、交互式
缩放重绘四个问题；随后又用快速拖动探针定位并修复了经典交换链快速拖动时残留的拉伸
帧，改为 DirectComposition 呈现，新增证据 07–13 与对应回归测试；按钮状态样式不在
本切片范围，当时记录为 #96，该问题随后由 #113（宿主瞬态）与 #110/#114（class
状态变体与深色主题）完成，见「样式里程碑的可见窗口验收」。

样式里程碑的第四轮自动驱动证据（125%）与维护者第五轮人工自测（hover、按下/拖出/
松开、`focus-visible:`、深色主题及 100%/150% 缩放）均未发现异常；hover 的自动截图
与 100%/150% 的自动驱动运行仍未取得，但人工清单已由维护者报告通过。#96 据此关闭。

跨显示器混合 DPI 与实机 device loss 仍缺证据（单显示器缩放切换已预检通过，只覆盖
同屏 DPI 变化），因此本页不声明阶段二原生验收完成，#63 保持打开；#60 的人工清单经
维护者自测报告通过（未提供截图、录屏或逐项记录），据此记录为通过。
