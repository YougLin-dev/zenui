# Stage-three 辅助技术验收（实机）

本页记录 #188 的只读 UIA 树：每个 Windows 窗口在场景提交时发布一棵从 presentation snapshot 与同一份 `Scene` 派生的树，`WM_GETOBJECT` 交回该窗口的 fragment-root provider。契约覆盖角色与控制类型、名称、AutomationId、启用/选中/按下状态、焦点、IsOffscreen、物理边界，以及滚动后的已平移几何。Invoke/Toggle/SetValue、UIA pattern 与事件属于后续工单，不在本页范围。

**状态：可见验收完成（100% 与 150%）。** 证据来自进程外的原生 `IUIAutomation` 客户端（Raw View）与 Inspect 的 UI Automation 模式；窗口是真实可见 HWND，滚轮由 OS 级 `SendInput` 注入，场景与树都走生产代码。

> 本页记录 #188 当时的验收范围与读数。Invoke/Toggle/Value pattern 与 SetFocus 已由 [stage-three 辅助技术动作验收](stage-three-辅助技术动作-验收.md) 交付，UIA 事件已由 [stage-three 辅助技术事件验收](stage-three-辅助技术事件-验收.md) 交付；本页“尚未实现 pattern / 事件”的表述只描述 #188 的基线，不代表当前构建。

验收日期：2026-11-23。功能基线为 `main` 的 `aec5b9b`。可见运行使用临时示例探针（未提交），它在 `zenui_demo` 的 Todo 上启动可见宿主，并把已应用偏移、`max`、逻辑视口与发布树节点数写入状态文件。探针、UIA 客户端与截图/缩放脚本均在提交前删除，仓库只保留本页与证据。

## 环境

| 项目 | 100% | 150% |
| --- | --- | --- |
| 显示缩放 | 96 DPI | 144 DPI |
| 初始物理客户区 | 900×700 | 1350×1050 |
| 初始逻辑视口 | 900×700 | 900×700 |
| 滚动后的逻辑视口 | 784×181 | 518.67×109.33 |
| 滚动后的物理客户区 | 784×181 | 778×164 |

单屏 2560×1600；本页不声明多显示器、混合 DPI 或跨屏拖动结果。两种缩放使用同一份逻辑布局，子元素物理边界按 scale 放大；根边界固定为窗口客户区，不随滚动移动。

## 方法

- 原生 UIA 客户端先调用 `SetProcessDpiAwarenessContext`，再用 `IUIAutomation::ElementFromHandle` 取窗口，沿 `RawViewWalker` 递归输出 ControlType、Name、AutomationId、BoundingRectangle、IsEnabled、IsKeyboardFocusable、HasKeyboardFocus、IsOffscreen、ClassName、FrameworkId 与 RuntimeId。本工单不实现 TogglePattern，dump 的 `toggle` 列为空；`checked`/`aria:pressed` 保存在树的只读状态里。
- `WM_GETOBJECT` 的根对象标识在 `lParam`；宿主只对 `UiaRootObjectId` 交回 provider，其它对象标识仍走默认过程。provider 持有窗口发布单元的弱引用，每次查询解析当前发布。
- 滚动使用真实可见窗口上的 OS 级 `SendInput(MOUSEEVENTF_WHEEL)`，一次向下 notch（`-120`）；点击客户区空白先让窗口获得前台焦点。注入经过系统输入队列、消息循环、`WM_MOUSEWHEEL` 与场景重建。
- 期望边界来自逻辑几何：100% 时物理位移等于逻辑偏移，150% 时等于逻辑偏移 × 1.5；读数来自 UIA 客户端，不来自实现内部字段。

## 角色、名称与状态

| 声明 | UIA 控制类型 | 名称来源 |
| --- | --- | --- |
| 窗口根 | `Window` | 目录标题 |
| `button` | `Button` | `aria:label`，否则直接文本 |
| `checkbox` | `CheckBox` | `aria:label`，否则直接文本 |
| `text-input` | `Edit` | `aria:label`，否则直接文本 |
| `text` | `Text` | `aria:label`，否则直接文本 |
| `form` | `Group` | `aria:label` |
| `view role="toolbar"` | `ToolBar` | `aria:label` |
| `view role="list"` | `List` | `aria:label` |
| `view role="listitem"` | `ListItem` | `aria:label` |
| `view role="spacer"` | 不生成节点 | 子元素并入最近的父级 |
| 其它标签 | `Custom` | `aria:label`，否则直接文本 |

`id` 映射为 AutomationId。`disabled` 映射为 IsEnabled；`checked` 与 `aria:pressed` 进入节点的只读 `toggle_state`，本工单不实现 TogglePattern，因此标准 UIA pattern 客户端不会看到 ToggleState。窗口位于前台且节点持有快照焦点时报告 HasKeyboardFocus；后台窗口不报告桌面焦点，`GetFocus` 也不返回元素。窗口禁用时元素与根都报告 IsEnabled=false。完全离开发布视口的节点 IsOffscreen=true，但仍在树中，直到被窗口化 `For` 卸载。隐藏与关闭中的窗口不发布树，缓存 provider 返回 `UIA_E_ELEMENTNOTAVAILABLE`。

## 初始树（100% 与 150%）

100% 的根边界是 `(48,71,900,700)`，150% 是 `(51,85,1350,1050)`；宿主树中 33 个节点。完整 dump 见 [`100-initial.txt`](证据/host-uia/100-initial.txt) 与 [`150-initial.txt`](证据/host-uia/150-initial.txt)。Raw View 还会列出窗口标题栏、系统菜单和最小化/最大化/关闭按钮，这些节点由 UIA 的宿主窗口 provider 提供，不属于 ZenUI 树。

| 元素 | 控制类型 | 100% 物理边界 | 150% 物理边界 | 状态 |
| --- | --- | --- | --- | --- |
| ZenUI Todo | Window | 48,71,900,700 | 51,85,1350,1050 | 根，FrameworkId=ZenUI |
| 1 remaining | Text | 64,87,868,16 | 75,109,1302,24 | 不可聚焦 |
| Add task | Group | 64,111,868,37 | 75,145,1302,55 | 容器 |
| What needs to be done? | Edit | 64,115,798,29 | 75,151,1197,43 | 可聚焦；点击并处于前台后 HasKeyboardFocus=true（见 150 滚动前 dump） |
| Add | Button | 870,111,62,37 | 1284,145,93,55 | 可聚焦 |
| All | Button | 64,156,42,29 | 75,212,63,43 | 树状态 `toggle_state=On`（`aria:pressed`） |
| Active | Button | 114,156,65,29 | 150,212,97,43 | 树状态 `toggle_state=Off` |
| Complete Write tests | CheckBox | 64,273,16,16 | 75,388,24,24 | 树状态 `toggle_state=On` |
| Write tests | Text | 88,272,66,19 | 111,386,99,28 | 只读文本 |

截图：[100% 客户区](证据/host-uia/100-initial.png)、[150% 客户区](证据/host-uia/150-initial.png)、[Inspect UI Automation 模式](证据/host-uia/inspect-150.png)。Inspect 以 UI Automation 模式识别到 “ZenUI Todo” 窗口根；截图保留的是根节点视图。完整树、边界与状态以上面的进程外 `IUIAutomation` Raw View dump 为准。

## 滚动后的几何

两种缩放都从 `offset=0` 滚动到 `offset=72`（一个 notch，3 行 × 24 逻辑像素），状态行见 [`100-states.txt`](证据/host-uia/100-states.txt) 与 [`150-states.txt`](证据/host-uia/150-states.txt)。

| 元素 | 100% 前 → 后 | 位移 | 150% 前 → 后 | 位移 |
| --- | --- | --- | --- | --- |
| 窗口根 | 48,71 → 48,71 | 0 | 51,85 → 51,85 | 0 |
| Add task | y=111 → 39 | −72 | y=145 → 37 | −108 |
| What needs to be done? | y=115 → 43 | −72 | y=151 → 43 | −108 |
| All | y=156 → 84 | −72 | y=212 → 104 | −108 |
| 1 remaining | y=87 → 15，IsOffscreen=false→true | −72 | y=109 → 1，IsOffscreen=false→true | −108 |

100% 的完整前后树见 [`100-before-scroll.txt`](证据/host-uia/100-before-scroll.txt) 与 [`100-after-scroll.txt`](证据/host-uia/100-after-scroll.txt)；150% 对应 [`150-before-scroll.txt`](证据/host-uia/150-before-scroll.txt) 与 [`150-after-scroll.txt`](证据/host-uia/150-after-scroll.txt)。根边界保持客户区不变，子节点边界只做一次平移；100% 的节点数从 27 变为 33，是行窗口在视口下缘跨入后续行后多挂载了一行（ADR-0011），不是第二棵保留树。存活的 Node ID 仍返回同一 provider 本地 runtime ID。

UIA 客户端看到的 runtime ID 形如 `[42, hwnd, 4, 8]`；`42` 与窗口句柄由 UIA 的宿主窗口前缀提供，`4` 是 fragment root 的本地前缀，末尾整数由 NodeId 分配并在发布之间保持。根节点的 `GetRuntimeId` 返回空数组，由 UIA 使用窗口默认运行时 ID。

## 生命周期与多窗口

- **隐藏与关闭中：** 隐藏窗口、`close_requested` 窗口和零客户区窗口清除发布树；重新显示后的第一帧提交重新发布。缓存 provider 在清除后每个属性查询返回 `UIA_E_ELEMENTNOTAVAILABLE`。
- **关闭确认：** `Confirm` 父窗口被原生禁用时保留树，元素与根报告 IsEnabled=false、IsKeyboardFocusable=false；确认窗口有自己的根。
- **独立根：** Details 窗口发布标题为 “ZenUI Details” 的独立根，与主窗口的 “ZenUI Todo” 根并存；每个 HWND 只应答自己的 `UiaRootObjectId`。
- **窗口化 `For`：** 行窗口内的行产生节点；卸载行不产生 Node ID。滚动导致行窗口推进时，新挂载行获得新 Node ID 与新的 provider 本地 ID。

以上生命周期由 `crates/windows/src/host.rs` 与 `crates/windows/tests/hidden_smoke.rs` 的隐藏 smoke 覆盖：树映射与状态、焦点、滚动边界与 runtime ID 稳定、隐藏/显示、确认父窗口禁用、Details 独立根、spacer 透明与 stale provider。

## 判据对照

| #188 要求 | 状态 | 证据 |
| --- | --- | --- |
| 角色/标签到控制类型 | 通过 | 本页映射表；100/150 dump |
| 名称优先 `aria:label`、否则可见文本 | 通过 | “Complete Write tests”、“Reverse” |
| AutomationId 来自 `id` | 通过 | `spacer_views_are_transparent_for_accessibility` 的 `inner-button` |
| 启用、checked、pressed、焦点 | 通过 | IsEnabled、树 `toggle_state`、HasKeyboardFocus 行 |
| IsOffscreen 与物理边界 100%/150% | 通过 | 初始树表；滚动前后表 |
| `WM_GETOBJECT` 返回 fragment root | 通过 | 原生 `IUIAutomation` 客户端与 Inspect |
| provider 本地 runtime ID 按 NodeId 稳定 | 通过 | 滚动前后 runtime ID；单元测试 |
| provider 不可变且 Send+Sync | 通过 | `providers_and_publications_are_send_and_sync` |
| 隐藏/不合格窗口无树 | 通过 | 隐藏 smoke；stale provider 返回 `UIA_E_ELEMENTNOTAVAILABLE` |
| 关闭确认父窗口报告禁用 | 通过 | `wm_close_on_a_confirm_window_opens_a_disabled_prompt` |
| Details 独立根 | 通过 | `details_is_an_independent_accessibility_root` |
| 滚动边界跟随已平移几何 | 通过 | 100% −72 px；150% −108 px；根不动 |
| 无 Invoke/Toggle/SetValue/事件 | 通过 | provider 不实现 pattern；`GetPatternProvider` 返回空 |
| fmt / clippy / tests | 通过 | 本工单提交的 workspace 门禁 |

## 证据等级与边界

- **可见原生 UIA（主证据）：** 进程外 `IUIAutomation` 客户端读取真实 HWND 的 Raw View；100% 与 150% 各有初始树与滚动前后树。Inspect 以 UI Automation 模式识别同一窗口根。
- **隐藏 smoke（部分证据）：** `cargo test -p zenui-windows --lib` 与 `crates/windows/tests/hidden_smoke.rs` 覆盖树形状、名称、状态、焦点、边界、runtime ID、隐藏/显示、确认禁用、Details 独立根与 stale provider；它不显示窗口，不代替可见 UIA。
- **未覆盖的能力：** 没有 `InvokePattern`、`TogglePattern`、`ValuePattern`、`ScrollPattern` 或 UIA 事件。`checked`/`aria:pressed` 存在于只读树；标准 UIA pattern 客户端不会看到 ToggleState，原生 dump 的 `toggle` 列为空。读屏软件的实际朗读顺序、焦点事件订阅与文本范围接口也不在本工单。隐藏/CI 结果只按部分证据记录。
- **单屏声明：** 只在单屏 2560×1600 上运行；100% 与 150% 是同一显示器的临时切换，不声称混合 DPI 或多显示器。
- **脱敏：** 截图只含应用客户区与 Inspect 窗口，不含标题栏、用户名或本机路径。临时探针、UIA 客户端与缩放/截图脚本在提交前删除；`git ls-files` 中没有本工单的探针源码或脚本。

## 后续状态

本页关闭 #188 的只读树与可见证据；写 pattern 已由 [stage-three 辅助技术动作验收](stage-three-辅助技术动作-验收.md) 交付，事件已由 [stage-three 辅助技术事件验收](stage-three-辅助技术事件-验收.md) 交付；`TextPattern` 已由 [stage-three 辅助技术文本验收](stage-three-辅助技术文本-验收.md) 交付；`ScrollPattern` 仍由 #218 交付。辅助技术决策仍由后续工单记录，本页不改变该范围。
