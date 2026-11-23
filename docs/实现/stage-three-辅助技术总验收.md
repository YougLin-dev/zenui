# Stage-three 辅助技术合同与朗读验收（实机）

本页记录 #192 的 UIA 合同验收：每个窗口一个 fragment root、树派生、provider 线程与动作队列、角色与名称映射，以及 primary、Details 和关闭确认窗口在 100%/150% 下的 Inspect 与 Narrator 观察。各 pattern 的行为与读数仍以专题页为准；契约本身见 [ADR-0012](../adr/0012-uia-derived-tree-and-action-queue.md)。

**状态：可见验收完成（100% 与 150%）；Narrator 逐字短语来自 Speech Recap，音频回环未采集到声音。** Inspect 以 UI Automation 模式显示三个窗口根（部分截图展开子节点）及其属性与物理边界；Narrator 的 Speech Recap 记录了窗口与按钮名称。收尾后桌面恢复到 150%，单屏，不声明多显示器或混合 DPI。

验收日期：2026-11-24。功能基线为 `main` 的 `6d5aa13`。可见运行使用仓库内的 `todo_host` 示例；Inspect 副本、截图与裁剪脚本、Narrator 辅助脚本放在仓库外，提交前未进入工作树。

## 环境

| 项目 | 100% | 150% |
| --- | --- | --- |
| 显示缩放 | 96 DPI | 144 DPI |
| 主窗口物理客户区 | 984×661 | 978×644 |
| Details 物理客户区 | 520×420 | 780×630 |
| 关闭确认物理客户区 | 420×220 | 630×330 |
| Inspect | UI Automation 模式 | 同左 |
| Narrator | Windows 讲述人（zh-CN），Speech Recap 窗口 | 同左 |

主窗口在两种缩放下都移动到屏幕右侧并设为 1000×700 物理窗口；Inspect 位于左侧（1440×1019），因此主窗口逻辑客户区随 scale 不同。Details 与关闭确认由应用按逻辑尺寸创建，物理边界按 1.5 倍缩放。

## 方法

- `todo_host` 启动后把主窗口移出与 Inspect 的重叠区域。Inspect 的 Watch Cursor 在选中目标后关闭；100% 运行还关闭了 Watch Focus。数据面板因此不再跟随指针或系统焦点；树选择用键盘在 `UI Tree View` 上完成，数据面板滚回顶部后截图。
- Details 通过主窗口工具栏的 `Details` 按钮（可访问名称 `Open details`）打开。关闭确认通过把草稿写为 `Pending draft` 后向主窗口发送 `WM_CLOSE` 打开；确认窗口打开期间主窗口由宿主禁用。
- 每个窗口根被选中后截图 Inspect 的树与数据面板；树面板只出现桌面、任务栏和系统设置等系统行与三个 ZenUI 窗口根，没有第三方应用窗口。Inspect 数据面板保留 provider 描述中的进程号，不含用户名或本机路径。
- Narrator 的观察来自 Windows 11 讲述人的 Speech Recap 窗口（`Insert+Alt+X`）。WASAPI 回环本身能录到系统声音，但这一轮没有录到讲述人的语音；因此本页只把 Recap 中的逐字短语作为观察记录，不声称音频或朗读次数。Recap 中与这些窗口无关的讲述人主页、设置与 Inspect 文本不引用。
- 主窗口、Details 与关闭确认都使用同一套生产 provider、动作队列和场景几何；没有探针树或第二份状态。

## 合同摘要

- **每 HWND 一个 fragment root：** 只有已发布树且可见、非关闭中的窗口应答 `WM_GETOBJECT` 的 `UiaRootObjectId`；主窗口、Details 和确认窗口各自是独立根。provider 持有窗口发布单元的弱引用，每次调用解析当前不可变树。
- **树派生：** 每棵发布树从同一份 presentation snapshot 与 `Scene` 派生；根是 `Window`，名称取窗口标题，边界为客户区；元素名称优先 `aria:label`，其次可见文本；`id` 映射 AutomationId；`role`/标签映射控制类型；`disabled`、`checked`/`aria:pressed`、焦点与 `IsOffscreen` 按快照状态报告；`role="spacer"` 透明；窗口化 `For` 只发布已挂载行。
- **线程与动作队列：** provider 线程只读当前发布；Invoke、Toggle、SetValue、SetSelection、SetFocus 与 Scroll 进入每窗口 1024 上限的 FIFO，UI 线程按最新快照或场景重验后执行。provider 不修改 Runtime；清除发布时队列一并清空。
- **Pattern：** Invoke/Toggle/Value 的存在性来自快照语义；TextPattern 在 Edit 节点；ScrollPattern 只在窗口根，水平固定 `NoScroll`，垂直 view size 与百分比来自同一份场景几何，无滚动条 NodeId。
- **事件：** 相邻发布按 runtime ID 比较，顺序为结构、属性、焦点；只有前台窗口投递焦点事件；hover/press、caret 闪烁与滚动几何不产生事件。
- **隐藏与确认：** 隐藏、关闭中或零客户区窗口不发布树；关闭确认父窗口保留树并报告禁用；确认窗口有自己的根。

完整决策、拒绝项与延后项见 [ADR-0012](../adr/0012-uia-derived-tree-and-action-queue.md)。

## Inspect 结果

三个窗口根在两个缩放下的读数一致地来自发布树：

| 窗口 | 缩放 | Name | ControlType | BoundingRectangle | IsEnabled | FrameworkId / ClassName |
| --- | --- | --- | --- | --- | --- | --- |
| 主窗口 | 100% | ZenUI Todo | Window | 1508,181,2492,842 | true | ZenUI / ZenUIHostWindow |
| 主窗口 | 150% | ZenUI Todo | Window | 1511,195,2489,839 | true | ZenUI / ZenUIHostWindow |
| Details | 100% | ZenUI Details | Window | 216,239,736,659 | true | ZenUI / ZenUIHostWindow |
| Details | 150% | ZenUI Details | Window | 1315,349,1095,979 | true | ZenUI / ZenUIHostWindow |
| 关闭确认 | 100% | Discard draft? | Window | 242,265,662,485 | true | ZenUI / ZenUIHostWindow |
| 关闭确认 | 150% | Discard draft? | Window | 353,387,983,717 | true | ZenUI / ZenUIHostWindow |

截图：

- 主窗口：[100%](证据/host-uia-contract/100-primary-inspect.png)、[150%](证据/host-uia-contract/150-primary-inspect.png)。
- Details：[100%](证据/host-uia-contract/100-details-inspect.png)、[150%](证据/host-uia-contract/150-details-inspect.png)。
- 关闭确认：[100%](证据/host-uia-contract/100-confirm-inspect.png)、[150%](证据/host-uia-contract/150-confirm-inspect.png)。

数据面板同时显示 `FrameworkId: "ZenUI"`、`ClassName: "ZenUIHostWindow"`、`IsControlElement: true`、`IsContentElement: true` 与 provider 描述。三组截图的数据面板都显示 `Scroll.*` 属性（未单独展开 pattern 子节点）；窗口当时的内容不足一屏，根报告 `Scroll.VerticallyScrollable: false`、`VerticalViewSize: 100.000000`，与场景一致。可滚动情形与 ScrollPattern 的读写见 [stage-three 辅助技术滚动验收](stage-three-辅助技术滚动-验收.md)。

关闭确认打开时，进程外 UIA 读取到主窗口 `IsEnabled=false`、Details 仍为 true；确认窗口自身为 true 且有独立根。该状态由宿主原生的禁用路径产生，隐藏测试与 [只读树验收](stage-three-辅助技术-验收.md) 覆盖同一契约。

## Narrator 观察

讲述人的 Speech Recap 在两种缩放下记录了：

| 窗口 | 100% | 150% |
| --- | --- | --- |
| 主窗口 | `ZenUI Todo 窗口, Add task 组, Add, 按钮,` | 同左；确认打开后另有 `Add 已禁用。` |
| Details | `ZenUI Details 窗口, 工具栏, Hide details, 按钮,` | 同左 |
| 关闭确认 | `Discard draft? 窗口, 工具栏, Continue editing, 按钮,` | 同左 |

100% 的三条短语在同一段 Recap 历史中；150% 的主窗口与确认短语在同一段历史中，Details 短语单独记录。截图：

- [100% 三条短语](证据/host-uia-contract/100-narrator.png)
- [150% 主窗口与确认](证据/host-uia-contract/150-primary-confirm-narrator.png)
- [150% Details](证据/host-uia-contract/150-details-narrator.png)

这些短语证明窗口根、工具栏/按钮角色和禁用状态进入标准 UIA 客户端；它们不证明朗读音频的音量、语速、重复次数或讲述人对未聚焦元素的朗读策略。Recap 中的 `关闭扫描` 等讲述人自身状态文本不引用。

## 隐藏测试与 CI

- `zenui-windows` 的 provider/树/队列测试覆盖角色、名称、AutomationId、启用/焦点/Toggle 状态、边界与 `IsOffscreen`、隐藏与确认父窗口、Details 独立根、动作重验与事件规划；本工单新增的契约文字不改变这些测试。
- 隐藏 smoke 与 CI WARP 只验证 provider 与场景/运行时之间的接口，不显示窗口，不替代本页的 Inspect 与 Narrator 结果。
- `cargo fmt --all -- --check`、workspace clippy `-D warnings` 与 workspace tests 是本页提交的门禁；CI 的 12 个必需检查单独记录在 PR。

## 判据对照

| #192 要求 | 状态 | 证据 |
| --- | --- | --- |
| ADR-0012 记录 per-HWND fragment roots、树派生、provider 线程、动作队列、名称/角色映射、隐藏/挂起行为与延后项 | 通过 | [ADR-0012](../adr/0012-uia-derived-tree-and-action-queue.md) |
| ADR-0012 与 SDK 记录根 ScrollPattern、SetScrollPercent/Scroll 映射、view size/百分比、NoScroll 与无滚动条 NodeId | 通过 | ADR-0012「Root ScrollPattern」；[SDK 窗口与任务](../SDK/窗口与任务.md)；[滚动验收](stage-three-辅助技术滚动-验收.md) |
| GLOSSARY 增加 accessibility-tree 术语；SDK 窗口/任务与样式/输入记录 UIA 行为与证据限制 | 通过 | `GLOSSARY.md` 的 Accessibility tree/provider/action/ScrollPattern；两个 SDK 页 |
| README/路线图不再把辅助技术描述为未开始，并链接验收页 | 通过 | `README.md` 的 UIA 条目与文档列表；`docs/产品/路线图.md` 的阶段三条目 |
| 可见页记录 primary、Details、关闭确认在 100%/150% 下的 Inspect 树/属性/边界与 Narrator 观察，并明确隐藏/CI 为部分证据 | 通过 | 本页六张 Inspect 截图、三张 Recap 截图与「隐藏测试与 CI」 |
| 不扩大声明；无多显示器或混合 DPI；fmt、clippy、workspace tests 通过 | 通过 | 本页「环境」与「边界」；提交门禁 |

## 边界

- **单屏：** 只在单屏 2560×1600 上运行；100% 是临时切换，结束时恢复 150%。不声称多显示器、混合 DPI 或跨屏拖动。
- **Narrator：** 本轮 WASAPI 回环没有声音，逐字短语来自 Speech Recap；不声称朗读音频、次数或所有元素的朗读顺序。讲述人以 UIAccess 运行，注入的 Recap 窗口关闭只能通过系统设置中的开关完成，因此每次观察都重启一次讲述人。
- **Inspect：** 树面板只含桌面、任务栏和系统设置等系统行与三个 ZenUI 窗口根，无第三方应用窗口标题；数据面板只含应用自己的属性与 provider 描述。Watch Cursor/Focus 在选中后关闭，避免数据面板跟随指针或系统焦点。
- **未覆盖：** MSAA/`IAccessible` 桥、Selection/SelectionItem、Grid/Table、虚拟化 item pattern、水平滚动、滚动条 NodeId、菜单/托盘原生 chrome 与读屏软件专属脚本；这些在 [ADR-0012](../adr/0012-uia-derived-tree-and-action-queue.md) 的 Deferrals 中列出。
- **滚动读数：** 本页的窗口内容不超一屏，ScrollPattern 报告 false；可滚动窗口的 0→50→100、分页/行步长与拒绝路径以 [滚动验收](stage-three-辅助技术滚动-验收.md) 为准。

## 后续状态

本页关闭 #192 的文档与可见验收缺口；ADR-0010/0011 的滚动契约、各 pattern 专题页与 #63 的多显示器/设备丢失边界不变。
