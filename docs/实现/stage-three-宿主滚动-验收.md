# Stage-three 宿主滚动验收（实机记录）

本页记录 #179 的可见 Windows 验收：`zenui-windows` 宿主把真实滚轮与翻页键转换成整页滚动偏移，`Scene` 按偏移平移几何，指针、caret 与 IME 继续使用同一份已平移场景。契约见 [ADR-0010](../adr/0010-scroll-is-presentation-state.md)；本页只覆盖 #179 当时交付的范围。

**后续状态（2026-11）：** 文中提到的焦点/IME 自动显露已由 #180 交付（[焦点显露验收](stage-three-焦点显露-验收.md)），滚动条已由 #181/#182 交付（[滚动条验收](stage-three-滚动条-验收.md)），窗口化 `For` 已由 #184 交付（[窗口化 For 验收](stage-three-窗口化-for-验收.md)）。hide/show 与 close/reopen 的可见运行、滚轮精度累积与模拟设备丢失的证据边界汇总在 [stage-three 滚动总验收](stage-three-滚动总验收.md)。

**状态：可见自动驱动验收完成（100% 与 150%）。** 运行使用真实可见 HWND、真实 `WM_MOUSEWHEEL` 路径与宿主默认 DX12 presenter；输入由 OS 级 `SendInput` 注入（鼠标移动、左键按下/松开、真实滚轮 notch、键盘按键），不是人工鼠标键盘。截图证明可见窗口的像素与场景状态，不证明用户注视下的 DWM 时序。

验收日期：2026-11-23。验收对象为 `921a1e8`（`feat/179-host-scroll`）；可见运行使用一个临时示例探针（未提交），运行后、提交前唯一的源码变化是一个 `#[cfg(test)]` 设备丢失用例，非测试源码与 `921a1e8` 相同。

## 环境

| 项 | 值 |
| --- | --- |
| 操作系统 | Windows 11 专业版 10.0.26300，64 位，交互式控制台会话 |
| 显示缩放 | 100%：`GetDpiForWindow=96`，客户区 900×700；150%：`GetDpiForWindow=144`，客户区 1350×1050 |
| 切换方式 | Settings > 显示 > 缩放（UI Automation）+ `rundll32 user32.dll,UpdatePerUserSystemParameters`；结束后恢复 150%（`GetDpiForSystem=144`） |
| 图形 | NVIDIA GeForce RTX 4060 Laptop GPU，驱动 32.0.15.7283；可见宿主使用默认 DX12 路径 |
| 显示 | 单屏 2560×1600 @ 240 Hz |
| 滚轮设置 | `WheelScrollLines=3`，`MouseWheelRouting=2`（指针下窗口路由，SPI_GETMOUSEWHEELROUTING 同值） |
| 输入方式 | OS 级 `SendInput`：真实光标移动、左键按下/松开、`MOUSEEVENTF_WHEEL` notch、键盘 PageUp/PageDown/Home/End；窗口用标题栏点击激活 |

## 方法

- 临时探针（未提交）用公开的 `Runtime::input` / `submit` 在 `Host::open` 之前种入 500 行，用 `HostConfig::visible()` 运行宿主，并以 10ms 周期 `pump()`；场景状态变化时打印一行 `state`（`offset`、`max`、`viewport`、`content`、`elements`、hover、press、focus、checked）。探针不绘制滚动条，也不注入额外 UI。
- 滚轮、键盘和指针输入由外部 PowerShell 脚本通过 `SendInput` 注入；截图用 `PrintWindow(PW_CLIENTONLY|PW_RENDERFULLCONTENT)` 读取客户区，先写 BMP 再转 PNG。
- 每个 DPI 下重复同一组步骤：一个 notch、再十个 notch、PageDown/PageUp、End、底部反向 notch、Home、顶部继续向上滚、悬停后滚动、按住节点滚动后在新位置松开、在旧位置松开、草稿聚焦时的编辑键与翻页键、窗口缩放。
- 场景与截图证据：两端都是 2519 个场景元素；内容 900×18654 逻辑像素；`max=17954`；一个 notch = 72 逻辑像素（3 行 × 24）；PageDown/PageUp = 676 逻辑像素（`floor(700-24)`）。完整状态见 [`证据/host-scroll/states.txt`](证据/host-scroll/states.txt)。

## 可见结果

### 100%（DPI 96，客户区 900×700）

| # | 步骤 | 预期 | 结果 | 截图 |
| --- | --- | --- | --- | --- |
| 1 | 启动 | 500 remaining；`offset=0`，`max=17954` | 通过 | [01-startup](证据/host-scroll/100-01-startup.png) |
| 2 | 一个真实 notch（向用户） | `offset=72`；指针下 hover 随内容更新 | 通过；hover NodeId 42 → 52 | — |
| 3 | 再十个 notch | `offset=792` | 通过；逐个 notch 累加，无漂移 | [02-middle](证据/host-scroll/100-02-middle.png) |
| 4 | PageDown | `offset=1468`（+676） | 通过 | [03-pagedown](证据/host-scroll/100-03-pagedown.png) |
| 5 | PageUp | 回到 `792` | 通过 | — |
| 6 | End | `offset=17954=max`；最后一行 Task 499 贴住底部，无空白条 | 通过 | [04-end-bottom](证据/host-scroll/100-04-end-bottom.png) |
| 7 | 底部反向一个 notch | 立即变为 `17882`（max−72），没有幽灵累积 | 通过 | [05-reverse-at-bottom](证据/host-scroll/100-05-reverse-at-bottom.png) |
| 8 | Home；顶部继续向上滚 | 回到 `0`；顶部向上 notch 被消费且 `offset` 保持 0 | 通过 | [06-home-top](证据/host-scroll/100-06-home-top.png) |
| 9 | 指针停在第一个 checkbox 上滚动 | hover 从 NodeId 20 变为 30，旧位置没有残留 hover | 通过 | [07-hover-after-scroll](证据/host-scroll/100-07-hover-after-scroll.png) |
| 10 | 按住 checkbox → 滚动 → 在新位置松开 | press 保持 NodeId 20，hover 变为 8；松开命中同一节点，Task 0 被勾选（`checked=1`） | 通过 | [08-press-scroll-release](证据/host-scroll/100-08-press-scroll-release.png) |
| 11 | 按住 checkbox → 滚动 → 在旧位置松开 | hover 变为 NodeId 30；松开命中另一节点，不激活，`checked` 仍为 1，元素数仍为 2519 | 通过 | [09-press-release-mismatch](证据/host-scroll/100-09-press-release-mismatch.png) |
| 12 | 草稿输入框聚焦后 End/PageDown/Home | End 与 Home 不滚动并保持 `offset=0`，焦点仍是 NodeId(1)；PageDown 滚到 676。可见状态只证明偏移与焦点；glyph 数与 caret 移动由隐藏 smoke 断言 | 通过 | [10-focused-paging](证据/host-scroll/100-10-focused-paging.png) |
| 13 | 在 max 处缩放窗口到 784×601 再恢复 | `offset=17954` 保持，`max` 先变为 18053 再回到 17954 | 通过 | [11-resized](证据/host-scroll/100-11-resized.png) |

### 150%（DPI 144，客户区 1350×1050）

| # | 步骤 | 预期 | 结果 | 截图 |
| --- | --- | --- | --- | --- |
| 1 | 启动 | `offset=0`，`max=17954`，逻辑几何与 100% 相同 | 通过 | [01-startup](证据/host-scroll/150-01-startup.png) |
| 2 | 一个真实 notch | `offset=72`；"500 remaining" 与草稿行滚出视口 | 通过 | [02-one-notch](证据/host-scroll/150-02-one-notch.png) |
| 3 | 再十个 notch | `offset=792`；中间状态逐 notch 记录 | 通过 | [03-middle](证据/host-scroll/150-03-middle.png) |
| 4 | PageDown / PageUp | 1468 / 792 | 通过 | [04-pagedown](证据/host-scroll/150-04-pagedown.png) |
| 5 | End；底部反向 notch | `17954=max`；反向立即到 `17882` | 通过；Task 499 贴底，无空白条 | [05-end-bottom](证据/host-scroll/150-05-end-bottom.png) · [06-reverse-at-bottom](证据/host-scroll/150-06-reverse-at-bottom.png) |
| 6 | Home；顶部继续向上滚 | `0`；顶部向上 notch 无位移 | 通过 | [07-home-top](证据/host-scroll/150-07-home-top.png) |
| 7 | 指针停在 checkbox 上滚动 | hover NodeId 20 → 30 | 通过 | [08-hover-after-scroll](证据/host-scroll/150-08-hover-after-scroll.png) |
| 8 | 按住 checkbox → 滚动 → 在新位置松开 | press 保持 NodeId 60，hover 变为 74；松开后 `checked=1` | 通过；Task 8 被勾选 | [09-press-scroll-release](证据/host-scroll/150-09-press-scroll-release.png) |
| 9 | 按住 Delete → 滚动 → 在旧位置松开 | press 保持 NodeId 73，hover 变为 83；松开不激活，元素数仍为 2519 | 通过 | [10-press-release-mismatch](证据/host-scroll/150-10-press-release-mismatch.png) |
| 10 | 在 `offset=144` 缩放窗口到 1078×844 再恢复 | 偏移保持 144；`max` 先变为 18091 再回到 17954 | 通过 | [11-resized](证据/host-scroll/150-11-resized.png) |
| 11 | 草稿输入框聚焦后 End/PageDown/Home | End/Home 不滚动；PageDown 到 676 且焦点保持；PageUp 回到 0。可见状态只证明偏移与焦点；glyph 数与 caret 移动由隐藏 smoke 断言 | 通过 | [12-focused-paging](证据/host-scroll/150-12-focused-paging.png) |

关键状态（完整行见 `states.txt`）：

| 状态 | 100% | 150% |
| --- | --- | --- |
| 启动 | `offset=0` / `max=17954` / elements 2519 | 同左，scale 1.5 |
| 一个 notch | `offset=72`；第一个 checkbox 中心 210→138 | 同左 |
| 十个 notch 后 | `offset=792` | 同左 |
| PageDown / PageUp | 1468 / 792（+676） | 同左 |
| End | 17954 = max | 同左 |
| 底部反向 | 17882 | 同左 |
| press 期间滚动 | pressed 保持；hover 单独更新 | 同左 |
| 新位置松开 / 旧位置松开 | `checked=1` / 不激活 | `checked=1` / 元素数不变 |
| 草稿聚焦 PageDown | 676，焦点 NodeId(1) 保持 | 同左 |
| 缩放 | 17954 保持；max 18053 后恢复 | 144 保持；max 18091 后恢复 |

## 证据等级与边界

- **可见自动驱动（本页）：** 真实可见 HWND、真实 OS 滚轮路由与 `SendInput`、宿主默认 DX12 presenter、真实窗口过程与 `PrintWindow` 像素。不是人工操作；不证明用户注视下的 DWM 合成、z-order 或被其它窗口遮挡时的表现。
- **隐藏 smoke：** `crates/windows/tests/hidden_smoke.rs` 覆盖 `WM_MOUSEWHEEL`/翻页键的偏移与夹取、滚动后按同一指针位置重新命中并把 hover 与 `hit_test` 对齐、滚动不刷新快照与 Node ID、hide/show 保留偏移、close/reopen 归零、聚焦编辑器的 glyph 数与 End/Home caret 移动（并断言同一 NodeId 保持焦点）、滚动后重绘帧像素变化。它们是路由与状态证据，不是原生验收。滚轮移动断言只在系统 `SPI_GETWHEELSCROLLLINES` 非零时执行；禁用分支由单元测试覆盖。
- **单元测试：** `WheelScroll` 的 notch 累积、亚 notch residue（5 × 24 单位 = 72 px，逐事件截断会得到 70）、符号、行数变化丢弃 residue、`SPI_GETWHEELSCROLLLINES` 原始值 0/`WHEEL_PAGESCROLL`/N 的映射与极端 delta；`host::tests::device_loss_preserves_the_applied_scroll_offset` 覆盖重建 surface 后偏移保留。
- **未覆盖与后续证据：** 本页当时没有 hide/show 的可见运行（外部脚本只能 `ShowWindow`，不改变 activation）；该缺口已由滚动总验收使用公开 `Host::hide`/`Host::show` 补齐，hide/show 与 close/reopen 以汇总页为准。模拟设备丢失仍只有单元测试证据：宿主没有公开的模拟设备丢失触发器，汇总页将其标为部分证据。`WM_SETTINGCHANGE` 消息处理本身没有直接测试（只测 SPI 值映射与 `set_lines` 的丢弃行为）；IME 候选窗定位与焦点自动显露属于 #180；滚动条属于 #181/#182；列表虚拟化已由 #184 交付，见[窗口化 For 验收](stage-three-窗口化-for-验收.md)；混合 DPI 多显示器与真实硬件设备丢失仍属 #63；实机丢失尝试未观察到丢失（[记录](stage-three-设备丢失-验收.md)）。
- `states.txt` 中的 hover/press NodeId 是场景元素身份；press 的 capture 语义与松开命中判定来自窗口过程，截图只能证明最终视觉状态。

## 判据对照

| 要求 | 状态 | 证据 |
| --- | --- | --- |
| 实机滚轮移动到中部、底部并返回 | 通过 | 步骤 2–3、7；100% 与 150% 状态与截图 |
| 底部/顶部边界的反向 notch 立即生效、无可累积幻影 | 通过 | 步骤 7–8；17882 与 0 的状态转换 |
| PageUp/PageDown 按 `floor(viewport−24)` 翻页 | 通过 | 步骤 4–5；1468−792=676 |
| 内容底部没有空白条 | 通过 | 步骤 6；Task 499 贴底截图 |
| 滚动后 hover 不残留、press 不被滚动重新评估 | 通过 | 步骤 9–11；press 保持、hover 更新；两种松开位置 |
| 聚焦编辑器保留编辑键，翻页键仍滚动 | 通过 | 可见步骤 12 / 11：End/Home 不滚动，PageDown 676 且焦点保持；隐藏 smoke 断言 glyph 数与 caret 移动 |
| 缩放保留并重新夹取偏移 | 通过 | 步骤 13 / 10 |
| hide/show 与 close/reopen 偏移生命周期 | 隐藏 smoke 覆盖，本页不宣称可见证据 | `hide_show_keeps_the_offset_and_reopen_starts_at_zero` |
| 模拟设备丢失保留偏移 | 单元测试覆盖，本页不宣称可见证据 | `device_loss_preserves_the_applied_scroll_offset` |
| 隐藏/WARP 证据不冒充原生验收 | 通过 | 本页「证据等级与边界」 |

## 结论

在 Windows 11、单屏 100% 与 150% 缩放的可见窗口上，宿主把真实滚轮 notch 与 PageUp/PageDown/Home/End 转换成整页滚动偏移：一个 notch 为 72 逻辑像素，翻页为 676，`End` 到达 `max=17954` 且最后一行贴住底部无空白条，边界处的反向输入立即生效。滚动只改变呈现环境：两端每次状态都保持 2519 个场景元素与相同的 Node ID，hover 随平移后的场景重新命中；按住节点滚动时 press 目标保持，在新位置松开才激活，在旧位置松开不激活。草稿聚焦时 End/Home 不滚动且焦点保持（caret 移动与 glyph 数由隐藏 smoke 断言），PageDown 滚动且焦点保持；窗口缩放保留并重新夹取偏移。证据由 OS 级 `SendInput` 自动驱动而非人工；隐藏 smoke 与单元测试只作为路由、生命周期与重建证据，不替代原生验收。
