# Stage-three 滚动总验收（实机汇总）

本页汇总 #185 要求的滚动验收：滚轮单位与精度累积、边界夹取与反向、PageUp/PageDown、焦点与 caret 显露、IME 候选对齐、覆盖层滚动条、hide/show、close/reopen、窗口化 `For` 与模拟设备丢失的证据边界。契约见 [ADR-0010](../adr/0010-scroll-is-presentation-state.md) 与 [ADR-0011](../adr/0011-windowed-for-mounting.md)；各专题的完整过程见[宿主滚动](stage-three-宿主滚动-验收.md)、[焦点显露](stage-three-焦点显露-验收.md)、[滚动条](stage-three-滚动条-验收.md)与[窗口化 For](stage-three-窗口化-for-验收.md)验收页。

**状态：可见自动驱动验收完成（100% 与 150%）。** 运行使用真实可见 HWND、宿主默认 DX12 presenter 与真实窗口过程；滚轮由 OS 级 `SendInput` 注入，不是人工鼠标键盘。截图证明可见窗口的像素与场景状态，不证明用户注视下的 DWM 时序。

验收日期：2026-11-23。功能基线为 `main` 的 `663ddb6`；可见运行使用一个临时示例探针（未提交），它在 `zenui_demo` 的 Todo 上预置 502 行，并以公开的 `Host::hide` / `Host::show` / `Host::close` 驱动生命周期。探针、截图脚本与缩放脚本均在提交前删除，仓库只保留本页与截图/状态证据。

## 环境

| 项目 | 100% | 150% |
| --- | --- | --- |
| 显示缩放 | 96 DPI（从 150% 临时切换，运行后恢复） | 144 DPI（本机当前设置） |
| 物理客户区 | 900×700 | 1350×1050 |
| 逻辑视口 | 900×700 | 900×700 |
| 滚动偏移单位 | 逻辑像素 | 逻辑像素（不按 scale 重缩放） |

单屏 2560×1600；本页不声明多显示器或混合 DPI 结果。两种缩放下的 `max`、内容高度、行带与滚轮步进完全相同（`max=18086`，内容高 18786，视口 700，502 行），这是偏移与几何使用逻辑像素的直接结果。

## 方法

- 窗口可见、前台且置顶；截图用桌面 `CopyFromScreen` 读取客户区矩形（DPI-aware 脚本），因此 150% 截图是 1350×1050、100% 是 900×700，均不含标题栏与桌面像素。
- 滚轮由 `SendInput(MOUSEEVENTF_WHEEL)` 注入：精度组每次 24 单位（低于一个 `WHEEL_DELTA`），常规组每次 120 单位。注入经过系统输入队列与真实 `WM_MOUSEWHEEL` 路径，窗口过程、`SceneEnv` 与场景重建都走生产代码。
- 每次事件后由探针读取 `Scene::scroll_offset()`、`Scene::max_scroll_offset()`、`content_extent()`、`viewport()` 与行带，写入 `states.txt`。偏移期望值来自 ADR-0010 的截断累积规则，不来自实现常量的回读。
- 临时探针的构建与删除记录见本页末尾；`states.txt` 保留事件顺序与完整数值。

## 精度滚轮

五个 24 单位 notch（向下）的已应用偏移与反向五个 notch：

| 事件 | 100% offset | 150% offset | 说明 |
| --- | --- | --- | --- |
| 初始 | 0 | 0 | 内容顶 |
| down 1 | 14 | 14 | 24 × 72 / 120 = 14.4，截断为 14 |
| down 2 | 28 | 28 | 残留 0.8 累积 |
| down 3 | 43 | 43 | 43.2 截断 |
| down 4 | 57 | 57 | 57.6 截断 |
| down 5 | 72 | 72 | 恰好一个常规 notch 的 72 逻辑像素 |
| up 1–5 | 57、43、28、14、0 | 57、43、28、14、0 | 反向对称，无幽灵偏移 |

100% 与 150% 的状态行见 [`100-states.txt`](证据/host-scroll-total/100-states.txt) 与 [`150-states.txt`](证据/host-scroll-total/150-states.txt)。截图：[100% offset 14](证据/host-scroll-total/100-01-precision-14.png)、[150% offset 14](证据/host-scroll-total/150-01-precision-14.png)、[100% offset 72](证据/host-scroll-total/100-02-precision-72.png)、[150% offset 72](证据/host-scroll-total/150-02-precision-72.png)。

这两张截图只相差 58 逻辑像素的平移（顶部工具栏与首行位置变化），焦点、模型与组件身份不变；滚动条 thumb 仍按同一份偏移定位。视口下缘在 14→72 之间跨入后续行时，窗口化 `For` 会按契约多挂载一行，这是行窗口更新，不是模型变化。

## 边界、翻页与显露

- 边界夹取与反向、PageUp/PageDown、Home/End、hover 复核与 press capture 的可见过程见[宿主滚动验收](stage-three-宿主滚动-验收.md)（100%/150%）。
- 焦点/caret 显露、Tab 到视口外目标、滚动不移动焦点与组合中候选窗锚点见[焦点显露验收](stage-three-焦点显露-验收.md)（100%/150%）。
- 滚动条 thumb 拖动、轨道翻页、与键鼠共享偏移见[滚动条验收](stage-three-滚动条-验收.md)（100%/150%）。
- 窗口化 `For` 的 10,000 行滚动、编辑、重排与过滤见[窗口化 For 验收](stage-three-窗口化-for-验收.md)（100%/150%）。

## 偏移生命周期

探针先以十个 120 单位 notch 滚到 `offset=720`，此时行带 `first_row=14`，滚动条 thumb 位于轨道顶部附近（约 4%）。

| 步骤 | 100% | 150% | 证据 |
| --- | --- | --- | --- |
| 滚到 720 | offset=720，first=14，max=18086 | 同左 | [100](证据/host-scroll-total/100-03-scrolled-720.png) · [150](证据/host-scroll-total/150-03-scrolled-720.png) |
| hide → 隐藏窗口上注入一个 notch → show | offset=720，first=14 | 同左 | [100](证据/host-scroll-total/100-04-after-show.png) · [150](证据/host-scroll-total/150-04-after-show.png) |
| close → show | offset=0，first=0，max 仍为 18086 | 同左 | [100](证据/host-scroll-total/100-05-after-reopen.png) · [150](证据/host-scroll-total/150-05-after-reopen.png) |

hide/show 前后的 PNG 在两种缩放下都逐字节相同（100%：`494e9e…`；150%：`50f754…`），说明隐藏期间的滚轮没有改变偏移、选中行或像素。重开后回到内容顶，max 与内容范围不变；应用级 Todo 模型保留（仍显示 501 remaining），释放的只是窗口 activation 的本地呈现状态。

## 判据对照

| #185 要求 | 状态 | 证据 |
| --- | --- | --- |
| 精度滚轮输入 | 通过 | 本页精度表；100/150 的 24 单位注入与偏移序列 |
| 边界夹取与反向 | 通过 | [宿主滚动验收](stage-three-宿主滚动-验收.md)；本页反向序列回到 0 |
| PageUp/PageDown | 通过 | [宿主滚动验收](stage-three-宿主滚动-验收.md) |
| 焦点与 caret 显露 | 通过 | [焦点显露验收](stage-three-焦点显露-验收.md) |
| IME 候选对齐 | 通过（带 TSF 缓存边界） | [焦点显露验收](stage-three-焦点显露-验收.md) |
| 滚动条拖动、翻页、隐藏/显示 | 通过 | [滚动条验收](stage-three-滚动条-验收.md) |
| hide/show 保留偏移 | 通过 | 本页 03/04 与 states |
| close/reopen 归零 | 通过 | 本页 04/05 与 states |
| 模拟设备丢失 | 部分（仅单元测试；#214 实机尝试未观察到丢失） | `host::tests::device_loss_preserves_the_applied_scroll_offset` |
| 窗口化 `For` | 通过 | [窗口化 For 验收](stage-three-窗口化-for-验收.md) |
| 临时探针移除、截图脱敏 | 通过 | 本页「证据等级与边界」；既有 file-picker 的 09/11/12/13 已补遮蔽账户名 |

## 证据等级与边界

- **可见运行（主证据）：** 真实 HWND、真实 OS 滚轮路由、真实窗口过程与 DX12 呈现；100% 与 150% 各一组截图和状态行。不是人工操作。
- **隐藏 smoke / WARP（部分证据）：** `crates/windows/tests/hidden_smoke.rs` 与 CI 的 WARP 适配器覆盖滚轮偏移与夹取、hide/show、close/reopen 与窗口化行窗口推进；它们不显示窗口，不代替可见验收。
- **模拟设备丢失（部分证据）：** 宿主没有公开的模拟设备丢失触发器，只有单元测试验证重建 surface 后偏移保留。真实硬件 device loss 已在单屏可见窗口上尝试但未观察到丢失（[记录](stage-three-设备丢失-验收.md)），恢复仍无真实证据；混合 DPI 多显示器仍属 #63。
- **物理精度触控板未覆盖：** 本页注入的是 OS 级 24 单位滚轮 delta，和像素级精度输入走同一累积规则，但不证明具体触控板驱动或惯性行为。
- **IME 边界：** 微软拼音等 TSF 候选窗在一次组合内缓存屏幕位置；只滚动时宿主已发出新锚点，候选窗要到下一次 preedit 更新才重排。宿主不因此取消组合或切换 IME 状态，详见[焦点显露验收](stage-three-焦点显露-验收.md)。
- **单屏声明：** 只在单屏 2560×1600 上运行；100% 是临时切换后恢复 150%。本页不声称多显示器、混合 DPI 或跨屏拖动结果。
- **UIA `ScrollPattern`：** 已由 [辅助技术滚动验收](stage-three-辅助技术滚动-验收.md) 交付；窗口根的 `Scroll` 与 `SetScrollPercent` 和滚轮、翻页键、滚动条共享同一个宿主偏移。本页的滚轮与键盘证据不变。
- **脱敏：** 本页截图只含客户区与应用内容，不含标题栏、用户名或本机路径。临时探针 `crates/windows/examples/scroll_acceptance_probe.rs` 与截图/缩放脚本在提交前删除；`git ls-files` 中没有本工单的探针源码或脚本（仓库中仅存的 `probe` 文件名是既有保存对话框证据中的 `probe.md` 结果截图，与本工单无关）。

## 后续状态

本页关闭 #185 的文档与可见证据缺口；它不修改 ADR-0010/0011 的契约，也不改变 #63 的开放边界。滚动条、窗口化 `For`、焦点显露、辅助技术滚动与宿主滚动的专题页保留各自当时的工单范围与验收日期。
