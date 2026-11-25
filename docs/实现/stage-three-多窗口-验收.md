# Stage-three 多窗口与关闭确认验收（实机记录）

本页记录阶段三第一个里程碑的可见窗口验收：窗口目录、多窗口显示、隐藏与重开、关闭确认、父窗口模态禁用与焦点恢复。验收在 100% 与 150% 两种显示缩放下各执行一次，并保留每个状态的客户区截图与状态日志。设计取舍见 [ADR-0007](../adr/0007-window-catalog-and-close-epoch.md)。

**状态：可见自动驱动验收完成（100% 与 150%）。** 本页不包含维护者口述报告；两种缩放都有截图和状态日志。跨显示器混合 DPI 仍未执行，属于 #63；实机 device loss 已由 #214 在单屏可见窗口上尝试但未观察到丢失（[记录](stage-three-设备丢失-验收.md)），不在本页结论内。CI 的隐藏窗口与 WARP 结果不是原生验收。

验收日期：2026-11-22。验收对象为 `3ccaab5`（#135 合并后的 `main`）。文档提交不改变行为。

## 环境

| 项 | 值 |
| --- | --- |
| 操作系统 | Windows 11 专业版 10.0.26300，64 位，交互式会话 |
| GPU / 驱动 | NVIDIA GeForce RTX 4060 Laptop GPU，驱动 32.0.15.7283 |
| 图形后端 | DX12；探针记录 `AdapterInfo { name: "NVIDIA GeForce RTX 4060 Laptop GPU", device_type: DiscreteGpu, backend: Dx12, driver: "32.0.15.7283" }`，不是 WARP |
| 显示 | 单显示器 BOE0C8E0，2560×1600（型号/分辨率来自系统探针）；有效 DPI 144（150%）与 96（100%） |
| 输入方式 | 向 HWND 投递原生鼠标、键盘与 `WM_CLOSE` 消息；未使用真实鼠标或 IME |

150% 是本机当前设置；100% 运行前通过 CCD 的 DPI scale 设备信息接口（Windows 显示设置使用的未公开参数）临时切到 96 DPI，运行结束后立即恢复 144 DPI。探针在每次运行开始时用 `GetDpiForWindow` 断言目标 DPI，并记录客户区物理尺寸：

| 缩放 | 断言 DPI | 主窗口客户区 | 详情窗口客户区 | 确认窗口客户区 |
| --- | --- | --- | --- | --- |
| 150% | 144 | 1350×1050 | 780×630 | 630×330 |
| 100% | 96 | 900×700 | 520×420 | 420×220 |

## 方法

- 使用与 `todo_host` 相同的 `zenui_demo::open_todo_window`、`HostConfig::visible()` 和运行时；主窗口、详情窗口与确认窗口都是真实可见的顶层 HWND，真实 GPU 绘制。
- 输入由临时验收探针向目标 HWND 投递 `WM_LBUTTONDOWN` / `WM_LBUTTONUP`、`WM_CHAR`、`WM_KEYDOWN` 和 `WM_CLOSE`，走的窗口过程与隐藏 smoke 相同；不是人工鼠标键盘，也不代替人工试跑。
- 每个状态先 `pump` 并重绘，再对每个窗口客户区调用 `PrintWindow(PW_CLIENTONLY | PW_RENDERFULLCONTENT)` 读回 BGRA，先写 BMP，再由 System.Drawing 转为同名 PNG；`states.txt` 首行说明其 capture 行使用提交的 PNG 名称。它不是桌面截屏，不能证明 z-order、遮挡关系或用户注视下的完整 DWM 合成；它证明可见原生窗口、真实 GPU 与真实窗口过程中的状态和像素。
- 主窗口草稿的焦点与 caret 由运行时快照与场景几何记录并断言：Deny 后焦点仍是草稿输入框的同一个 Node ID，caret 的 logical rect 与关闭前一致。详情窗口的焦点/caret 由截图中的输入文本与 caret 呈现，不单独写入日志。
- 确认窗口的父窗口禁用通过 `IsWindowEnabled(main)` 为 false 观察；这是宿主同步运行时输入挂起后的原生结果。
- 临时探针未提交到仓库；它临时启用了窗口截图的 GDI/Xps feature，并读取宿主已选中的 adapter 信息，取证后一并移除，不改变窗口行为。每轮的状态、HWND 值、可见性、启用状态、焦点、caret、draft/note 与 DPI 记录在 `证据/multi-window/150/states.txt` 与 `证据/multi-window/100/states.txt`。

## 可见窗口结果

下表每个“截图”单元给出 150% 与 100% 的对应客户区截图。步骤在两档缩放下使用同一序列。

| # | 步骤 | 预期 | 150% | 100% | 截图 |
| --- | --- | --- | --- | --- | --- |
| 1 | 启动 | 主窗口可见，两条种子任务，1 remaining | 通过 | 通过 | [150](证据/multi-window/150/01-main.png) · [100](证据/multi-window/100/01-main.png) |
| 2 | 点击 Details | 详情窗口出现，与主窗口共享模型 | 通过 | 通过 | 主 [150](证据/multi-window/150/02-main-with-details.png)/[100](证据/multi-window/100/02-main-with-details.png)；详情 [150](证据/multi-window/150/03-details.png)/[100](证据/multi-window/100/03-details.png) |
| 3 | 在详情 note 输入 `note` | 详情显示 `note` 与可见 caret，主窗口草稿未被改动 | 通过 | 通过 | [150](证据/multi-window/150/04-details-note.png) · [100](证据/multi-window/100/04-details-note.png) |
| 4 | 点击 Hide | 详情 HWND 不可见，主窗口仍可见且启用 | 通过 | 通过 | 主窗口 [150](证据/multi-window/150/05-main-details-hidden.png) · [100](证据/multi-window/100/05-main-details-hidden.png)；详情状态见日志 |
| 5 | 主窗口再次点击 Details | 复用同一详情 HWND，note 仍为 `note` | 通过 | 通过 | [150](证据/multi-window/150/06-details-restored.png) · [100](证据/multi-window/100/06-details-restored.png) |
| 6 | 在详情点击 Close | 详情 HWND 销毁，主窗口存活 | 通过 | 通过 | 主窗口 [150](证据/multi-window/150/07-main-details-closed.png)/[100](证据/multi-window/100/07-main-details-closed.png)；详情状态见日志 |
| 7 | 再次打开 Details | 新 HWND，note 为空 | 通过 | 通过 | [150](证据/multi-window/150/08-details-reopened.png) · [100](证据/multi-window/100/08-details-reopened.png) |
| 8 | 关闭详情，在主窗口草稿输入 `Milk` | 草稿框聚焦，caret 位于文本末尾 | 通过 | 通过 | [150](证据/multi-window/150/09-main-draft.png) · [100](证据/multi-window/100/09-main-draft.png) |
| 9 | 对主窗口发送 `WM_CLOSE` | 出现确认窗口；父 HWND 禁用；草稿仍为 `Milk` | 通过 | 通过 | 提示 [150](证据/multi-window/150/10-prompt.png)/[100](证据/multi-window/100/10-prompt.png)；主 [150](证据/multi-window/150/11-main-disabled.png)/[100](证据/multi-window/100/11-main-disabled.png) |
| 10 | 确认窗口 X（`WM_CLOSE`） | Deny：提示关闭，父 HWND 重新启用，草稿、焦点与 caret 保持 | 通过 | 通过 | [150](证据/multi-window/150/12-main-after-x-deny.png) · [100](证据/multi-window/100/12-main-after-x-deny.png) |
| 11 | 再次关闭，确认窗口 Escape | Deny：与 X 相同 | 通过 | 通过 | [150](证据/multi-window/150/13-main-after-escape-deny.png) · [100](证据/multi-window/100/13-main-after-escape-deny.png) |
| 12 | 再次关闭，点击 Continue editing | Deny：恢复同一 Node ID 的焦点与同一 caret | 通过 | 通过 | [150](证据/multi-window/150/14-main-after-continue.png) · [100](证据/multi-window/100/14-main-after-continue.png) |
| 13 | 再次关闭，点击 Discard draft | Allow：主窗口与确认窗口作为一组关闭，消息循环退出 | 通过 | 通过 | 无存活窗口；见 `states.txt` |

以 150% 运行为例，`states.txt` 记录的关键状态如下（100% 的 Node ID、caret 与 draft 相同，HWND、DPI 与物理尺寸不同；完整行见两个 `states.txt`）：

| 状态 | main enabled | main focus / caret | details HWND | prompt HWND |
| --- | --- | --- | --- | --- |
| 启动 | true | — | — | — |
| 详情打开 | true | — | 190908546，note `""` | — |
| 详情隐藏 | true | — | 190908546，visible=false | — |
| 详情恢复 | true | — | 190908546，note `"note"` | — |
| 详情关闭 | true | — | — | — |
| 详情重开 | true | — | 190974082，note `""` | — |
| 草稿 `Milk` | true | `NodeId(1)`，caret 47.88/49 | — | — |
| 确认待决 | **false** | `NodeId(1)`，同一 caret | — | 191039618 |
| X 后 | true | `NodeId(1)`，同一 caret | — | — |
| Escape 后 | true | `NodeId(1)`，同一 caret | — | — |
| Continue editing 后 | true | `NodeId(1)`，同一 caret | — | — |
| Discard 后 | 窗口已关闭 | — | — | — |

Deny 前后的客户区像素可以完全相同：草稿、焦点与 caret 本来就没有变化，父 HWND 的重新启用不是客户区像素，而是 `IsWindowEnabled` 与运行时输入状态；上表与 `states.txt` 才是这些断言的证据。截图证明对应可见窗口的客户区内容，不单独证明启用位。隐藏/关闭/重开行由 `states.txt` 的 details `hwnd/visible/note` 字段证明：隐藏行 `note=None` 是因为 `presentation_of` 拒绝隐藏窗口，不代表本地 note 被清空；`details shown` 行恢复 `note="note"` 证明保留。第 3 步的详情焦点/caret 由截图中的 caret 与输入文本支持，日志只记录主窗口的 focus/caret。

## 证据等级与边界

- **可见自动驱动（本页）**：可见 HWND、真实 GPU、真实窗口过程、原生消息路径和真实 `PrintWindow` 客户区像素。不是人工操作；不证明用户注视下的 DWM 时序，也不证明窗口是否被其它应用遮挡。
- **隐藏 smoke 与 CI WARP**：`crates/windows/tests/hidden_smoke.rs` 在隐藏 HWND 上覆盖主窗口与第二个 HWND（详情或确认窗口）、焦点隔离、关闭协议和 `read_frame` 像素；它不要求三个窗口同时存活，也不是可见窗口验收，不能替代本页。
- **维护者人工自测**：本里程碑未提供口述报告。真实鼠标/键盘、中文 IME 与 hover 的人工项仍按 stage-two 的清单处理，本页不重复声明；本页的自动驱动只覆盖多窗口与关闭确认。
- **#63 缺口**：本页只做单显示器上的缩放切换；跨显示器混合 DPI 与跨屏拖动仍未执行，#63 保持打开。实机 device loss 由 #214 在单屏上尝试但未观察到丢失，见[记录](stage-three-设备丢失-验收.md)。100% 档位是临时切换，运行后已恢复 150%；截图不能当作混合 DPI 证据。

## 自动覆盖（不能代替本页）

- 多窗口生命周期与共享模型：`tests/multi_window.rs`、`tests/details_window.rs`、`tests/close_prompt.rs`、`tests/close_confirmation.rs`。
- 原生消息路径：`crates/windows/tests/hidden_smoke.rs` 的 `two_windows_have_catalog_titles_sizes_and_independent_frames`、`a_shared_model_change_repaints_both_windows`、`focus_and_drafts_stay_per_window_and_hide_stops_input`、`the_details_action_opens_the_shared_second_window`、`closing_with_a_draft_prompts_and_deny_restores_the_parent`、`discarding_the_draft_closes_the_group_and_ends_the_loop`、`an_empty_draft_closes_without_a_prompt`。
- 宿主关闭与焦点同步：`crates/windows/src/host.rs` 的 `wm_close_on_a_confirm_window_opens_a_disabled_prompt`、`closing_the_confirmation_window_denies_the_parent`、`escape_on_the_confirmation_denies_and_reenables_the_parent`、`allow_from_the_confirmation_closes_the_group`。

这些测试与 CI 的隐藏窗口运行只提供路由、协议和像素逻辑的证据；本页的可见结果与它们分开记录。

## 判据对照（#126）

| 要求 | 状态 | 证据 |
| --- | --- | --- |
| 静态目录、多窗口、共享模型、独立焦点/呈现 | 通过 | 可见步骤 1–8；`tests/multi_window.rs`、`tests/details_window.rs` |
| show 复用、hide 保留 activation 与本地状态、close 释放、reopen 新 activation | 通过 | 可见步骤 2–7；`states.txt`；`tests/multi_window.rs` |
| `Confirm` 关闭建立 epoch、父窗口挂起、确认窗口绑定 | 通过 | 可见步骤 9–13；`tests/close_confirmation.rs`、`tests/close_prompt.rs` |
| Continue editing / Deny 恢复焦点与 caret；X/Escape 等于 Deny | 通过 | 可见步骤 10–12；日志中的 focus Node ID 与 caret rect |
| Discard / Allow 释放父窗口与确认窗口，最后一窗关闭后循环退出 | 通过 | 可见步骤 13；`states.txt` |
| 父窗口在待决期间原生禁用 | 通过 | 可见步骤 9；`IsWindowEnabled=false` |
| CI 隐藏窗口/WARP 不作为原生验收 | 通过 | 本页「证据等级与边界」；ADR-0005 |
| 跨显示器混合 DPI 与实机 device loss 不并入本里程碑 | 通过（保持打开） | #63；实机丢失尝试见[设备丢失实机尝试](stage-three-设备丢失-验收.md)；[windows-混合dpi-检查.md](windows-混合dpi-检查.md) |

## 结论

在 Windows 11、RTX 4060、DX12 的真实可见窗口上，主窗口与详情窗口可以同时呈现并共享模型，hide/show 复用同一详情 HWND 并保留 note，close/reopen 产生新 HWND 与空 note。带草稿关闭主窗口会显示声明的确认窗口并禁用父 HWND；X、Escape 与 Continue editing 都以 Deny 恢复父窗口的草稿、焦点与 caret；Discard 把父窗口与确认窗口作为一组释放并结束消息循环。以上流程在 100% 与 150% 缩放下各执行一次；截图记录可见内容，`states.txt` 记录 HWND、启用位与焦点/caret 断言，两者分别支撑各自能够证明的部分。CI WARP 与隐藏窗口结果未被用作本页的原生验收；#63 的跨屏混合 DPI 仍未完成，实机 device loss 的单屏尝试未观察到丢失（[记录](stage-three-设备丢失-验收.md)）。
