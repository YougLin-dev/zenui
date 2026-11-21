# Stage-two Windows 验收（实机记录）

本页记录阶段二 Windows 原生切片的实机运行结果，并对照 #60、#63 的验收范围。

**状态：部分完成。** 可见窗口、真实 GPU、150% 缩放的自动驱动运行已完成并保留截图；
第二轮人工试跑发现的问题已修复并复验（见「人工试跑反馈与修复」）。中文 IME 候选、
混合 DPI 显示器切换与真实硬件键盘操作仍待人工确认：候选、提交与取消已用真实微软
拼音引擎配合合成按键验证，但这不代替人工操作。
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
  连续操作；下表的会话列说明每个状态属于哪一次运行。07–12 来自第二轮的独立运行；
  12 是松开鼠标之前截取的 900×678 物理客户区。

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

**标签断行。** 在 150% 缩放下，"Load sample" 与 "Preview import" 被拆成两行并
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
到达时同步按新客户区重排并呈现，避免 DWM 先把上一帧拉伸到新尺寸；`WM_PAINT` 只处理
有实际更新区域的失效。
回归测试 `crates/windows/src/host.rs::a_paint_message_repaints_after_an_external_resize`
在消息循环之外改窗口尺寸并派发 `WM_PAINT`，断言场景按新的客户区重排；证据
[12](证据/stage-two/12-live-resize-during-drag.png) 是拖动过程中（松开鼠标之前）
截取的画面，布局已按新尺寸重排且文字未缩放。

第四个问题“按钮没有 hover 等状态效果”不在本切片的范围：spec #68 明确本阶段不做
class 样式，状态样式属于路线图的样式里程碑，已记录为 #96。

## 已知限制

窗口窄于工具栏内容宽度（约 470 逻辑像素）时，toolbar 行不换行，右侧按钮会被
窗口边缘裁掉。这是当前 demo 的响应式布局缺口，不在 640×480 的 resize 判据内，
留待后续布局能力处理。

## 判据对照（#81）

| 判据 | 状态 | 证据 |
| --- | --- | --- |
| 同一 Todo 在可见 Windows 11 窗口、真实 GPU 上运行 | 通过 | 上表 01–05；DX12 离散 GPU |
| 完整工作流（增删、勾选、筛选、重排） | 部分 | 可见：输入/添加/勾选/筛选/重排/异步；删除由隐藏 smoke `delete_removes_a_native_row` 覆盖，未在本次可见运行中执行 |
| 中文 IME 组合、提交、取消与候选位置 | 自动通过（真实 IME + 合成按键），人工未执行 | 07–09；宿主单元测试覆盖 `WM_IME_*` 路由 |
| 焦点与 Tab 顺序 | 自动通过，人工未执行 | `tab_moves_focus_to_the_next_control`、`shift_tab_moves_focus_backward` |
| resize | 通过（程序化与交互式拖动） | 06、12；`resizing_relayouts_the_scene_and_surface`、`a_paint_message_repaints_after_an_external_resize` |
| 混合 DPI 移动 | 未执行（单显示器） | 步骤见[混合 DPI 检查](windows-混合dpi-检查.md) |
| 关闭/重开为新 activation，无旧任务或旧呈现 | 自动通过 | `closing_the_window_cancels_window_owned_tasks`、`reopening_uses_a_new_activation_and_discards_the_old_delivery`、`closing_the_activation_prevents_stale_wakes_and_delivery` |
| 重排与筛选后无残留焦点、caret 或像素 | 部分（像素可见；焦点/caret 自动） | 上表 02、05 验证顺序与像素；10/11 验证 caret 只随焦点出现并闪烁；`reorder_keeps_focus_and_caret_on_the_same_row` 按 Node ID 与 caret x 断言 |
| device/surface 重建 | 自动通过，实机未触发 | `a_surface_rebuild_preserves_focus_and_caret`；`Lost` 路径未在实机强制触发 |
| 证据区分 headless、离屏、WARP 与实机 | 通过 | 本页与 `docs/SDK/窗口与任务.md`、ADR-0005 |
| 更新验收文档并确认 #60/#63 覆盖 | 通过 | 本页下方两节 |

## #60 覆盖

| #60 验收 | 证据 | 状态 |
| --- | --- | --- |
| 同一 `examples/minimal_todo.rs` 的 Todo 挂载到真实 Windows 窗口 | `zenui-windows::Host` 挂载 `zenui-demo` 的 TodoApp；上表 01–05 | 通过 |
| add/toggle/filter/reorder/delete 经原生输入进入同一 dispatch 路径，无第二棵 UI 树 | 隐藏 smoke 与本次可见运行；宿主只保存 Node ID 派发，不保存 caret/值（`docs/SDK/窗口与任务.md`） | 通过（delete 仅在 smoke） |
| headless 与 native 共享 target runtime，且不互相冒充完成 | 两边使用同一 `Runtime`/组件树；本页明确分区 | 通过 |

#60 仍保持打开：真实硬件输入与中文 IME 的人工确认属于它的原生验收范围。

## #63 覆盖

| #63 验收 | 证据 | 状态 |
| --- | --- | --- |
| 同一个 target Todo 在真实 Windows 窗口呈现，并在 add/toggle/filter 后更新 | 上表 01–05；`zenui-render` 的 Scene/GPU 路径 | 通过 |
| DPI 变化与 device/surface 重建保留挂载身份，无旧呈现 | 150% DPI 实机呈现；resize 与重建测试；跨显示器变化未执行 | 部分 |
| 不支持的能力显式报错，不用 validation panic 代替 | 不支持的标签/属性返回 `SceneError`；offscreen 格式返回 `RenderError::UnsupportedFormat`；Windows surface 只接受非 sRGB `Bgra8Unorm`（`HostError::Surface`） | 通过 |
| headless 测试仍是逻辑测试，不推断像素/native 结论 | 本页分级 | 通过 |

#63 仍保持打开：混合 DPI 显示器移动与实机 device loss 尚无证据；真实 IME 候选与
提交/取消已由真实引擎配合合成按键覆盖，人工 IME 操作仍未执行。

## 仍需人工执行

1. 在装有微软拼音的 Windows 11 上，用真实键盘在草稿框和行编辑框组合中文；
   确认 preedit 出现在 caret 处、候选窗贴近 caret、Space 选词提交一次、Escape 恢复
   组合前的文本与 selection。第二轮已用真实 IME 引擎配合合成按键复验，并以 07–09
   保留候选窗与提交/取消结果；仍缺人工键盘操作。
2. 用真实鼠标/键盘完成 add、toggle、filter、reverse、delete 与 Tab/Shift+Tab，
   确认焦点与 caret 不随重排跳到别的行。
3. 按[混合 DPI 检查](windows-混合dpi-检查.md)在两块不同缩放率显示器间移动窗口。
4. 若可触发，记录真实 device loss（例如驱动重置）后的重建与重绘；否则保留自动化
   重建测试的覆盖说明。

## 结论

已在真实 Windows 11、离散 GPU、150% 缩放的可见窗口上验证：同一 Todo 正常呈现，
原生消息驱动输入/添加/勾选/筛选/重排/异步更新，后台任务无需轮询即可重绘，resize
重新布局且不裁剪；实机运行与随后的评审发现并修复了标签断行和带 padding 的
text-input 测量宽度两个缺陷，并各自补了回归测试。

第二轮人工试跑又发现并修复了中文候选提交、Escape 取消、caret 聚焦与闪烁、交互式
缩放重绘四个问题，新增证据 07–12 与对应回归测试；按钮状态样式不在本切片范围，
记录为 #96。

中文 IME 候选、混合 DPI、真实硬件键盘/焦点与实机 device loss 仍缺证据，因此本页
不声明阶段二原生验收完成，#60/#63 保持打开。
