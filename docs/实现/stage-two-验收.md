# Stage-two Windows 验收（实机记录）

本页记录阶段二 Windows 原生切片的实机运行结果，并对照 #60、#63 的验收范围。

**状态：部分完成。** 可见窗口、真实 GPU、150% 缩放的自动驱动运行已完成并保留截图；
中文 IME 候选窗口、混合 DPI 显示器切换、真实硬件键盘与焦点操作仍待人工执行。
本页不把脚本投递的原生消息当作人工操作，也不把 headless 或 CI WARP 结果当作原生呈现验收。

验收日期：2026-11-21。验收对象：`edd0383`（#80 合并后的 `main`）与本分支的
文本测量修复。命令：

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
- 用 PowerShell 捕获客户区像素，并向 HWND 投递 `WM_LBUTTONDOWN`、`WM_CHAR`、
  `WM_KEYDOWN`（Enter）和 `WM_CLOSE`。这走与隐藏窗口 smoke 相同的原生处理路径，
  但不是物理鼠标/键盘输入，也不代替人工清单。
- 截图是客户区物理像素：初始 1350×1050（900×700 逻辑 × 1.5），resize 后 960×720
  （640×480 逻辑 × 1.5）。截图来自三次独立的可见运行（会话 A/B/C），不是一次
  连续操作；下表的会话列说明每个状态属于哪一次运行。

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

截图 05 中的每个 text-input 都会绘制 caret，因此截图不能用来判断焦点属于哪一行；
焦点与 caret 的连续性由隐藏 smoke `reorder_keeps_focus_and_caret_on_the_same_row`
按 Node ID 与 caret x 断言。

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

## 已知限制

窗口窄于工具栏内容宽度（约 470 逻辑像素）时，toolbar 行不换行，右侧按钮会被
窗口边缘裁掉。这是当前 demo 的响应式布局缺口，不在 640×480 的 resize 判据内，
留待后续布局能力处理。

## 判据对照（#81）

| 判据 | 状态 | 证据 |
| --- | --- | --- |
| 同一 Todo 在可见 Windows 11 窗口、真实 GPU 上运行 | 通过 | 上表 01–05；DX12 离散 GPU |
| 完整工作流（增删、勾选、筛选、重排） | 部分 | 可见：输入/添加/勾选/筛选/重排/异步；删除由隐藏 smoke `delete_removes_a_native_row` 覆盖，未在本次可见运行中执行 |
| 中文 IME 组合、提交、取消与候选位置 | 未执行（需人工） | 宿主单元测试覆盖 `WM_IME_*` 路由；真实候选 UI 未验证 |
| 焦点与 Tab 顺序 | 自动通过，人工未执行 | `tab_moves_focus_to_the_next_control`、`shift_tab_moves_focus_backward` |
| resize | 通过 | 上表 06；`resizing_relayouts_the_scene_and_surface` |
| 混合 DPI 移动 | 未执行（单显示器） | 步骤见[混合 DPI 检查](windows-混合dpi-检查.md) |
| 关闭/重开为新 activation，无旧任务或旧呈现 | 自动通过 | `closing_the_window_cancels_window_owned_tasks`、`reopening_uses_a_new_activation_and_discards_the_old_delivery`、`closing_the_activation_prevents_stale_wakes_and_delivery` |
| 重排与筛选后无残留焦点、caret 或像素 | 部分（像素可见；焦点/caret 自动） | 上表 02、05 验证顺序与像素；截图不能判断焦点，`reorder_keeps_focus_and_caret_on_the_same_row` 按 Node ID 与 caret x 断言 |
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

#63 仍保持打开：混合 DPI 显示器移动、真实 IME 候选窗口与实机 device loss 尚无证据。

## 仍需人工执行

1. 在装有微软拼音的 Windows 11 上，用真实键盘在草稿框和行编辑框组合中文；
   确认 preedit 出现在 caret 处、候选窗贴近 caret、Space 选词提交一次、Escape 恢复
   组合前的文本与 selection。截图需包含候选窗。
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

中文 IME 候选、混合 DPI、真实硬件键盘/焦点与实机 device loss 仍缺证据，因此本页
不声明阶段二原生验收完成，#60/#63 保持打开。
