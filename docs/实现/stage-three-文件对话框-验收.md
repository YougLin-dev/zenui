# Stage-three 文件选择器与导入验收（实机记录）

本页记录阶段三文件选择器里程碑的可见 Windows 验收：原生打开文件对话框、标题与过滤器、owner 模态禁用、取消、单选、多选、文本导入、错误路径、长/Unicode 路径，以及对话框期间的 worker 完成与关闭后的焦点/输入。设计取舍见 [ADR-0008](../adr/0008-file-picker-host-tasks.md)。

**状态：可见自动驱动验收完成（150%）。** 运行使用真实可见 HWND、真实 shell `IFileOpenDialog` 和宿主默认 presenter，没有注入 presenter。输入通过 UI Automation 与投递的原生鼠标/键盘消息完成，不是人工鼠标键盘；截图证明可见窗口中的状态与像素，不证明用户注视下的 DWM 时序。隐藏 smoke 与 CI WARP 是部分证据，不构成本页的原生验收。

验收日期：2026-11-22。验收对象为 `5f2cd54`（#152 合并后的 `main`；#151 的原生 presenter 已在此之前合并）。文档提交不改变行为。

## 环境

| 项 | 值 |
| --- | --- |
| 操作系统 | Windows 11 专业版 10.0.26300，Build 26300.9974，64 位，交互式会话 |
| 显示缩放 | DPI 144（150%）；主窗口逻辑 900×700，外框 1372×1106 物理像素 |
| 图形 | 可见 HWND 上运行宿主默认 DX12 路径；本探针未重新查询 adapter。同机 stage-three 多窗口验收记录的是 NVIDIA GeForce RTX 4060 Laptop GPU / 驱动 32.0.15.7283 |
| 对话框 | shell 默认尺寸 1281×801 物理像素；class `#32770` |
| 输入方式 | 主窗口投递 `WM_LBUTTONDOWN` / `WM_LBUTTONUP` / `WM_CHAR`；对话框由 UI Automation 设置文件名并调用 Open/Cancel，未使用真实鼠标或键盘 |

150% 是本机当前设置；本里程碑没有临时切换到 100%，因此本页不声明 100% 缩放结果。

## 方法

- 使用一个临时的可见验收探针（未提交）包裹 `zenui_demo::TodoApp`，增加 `Slow task` 按钮与结果文本，以便观察对话框打开期间的 worker 交付；demo 的按钮、任务与共享模型都是实际实现。宿主用 `HostConfig::visible()` 与默认 presenter，没有 `set_file_dialog_presenter`。
- 主窗口的点击和字符输入走与隐藏 smoke 相同的窗口过程；对话框由 UIAutomation 定位：`文件名(N)` 编辑框 AutomationId `1148`，Open 按钮 `1`，Cancel 按钮 `2`。对话框用 class `#32770`、标题 `Import tasks` 与 owner HWND 识别。
- 每个对话框先导航到临时数据目录，再截图与选择，避免文件列表包含用户其它目录。截图用 `PrintWindow(PW_RENDERFULLCONTENT)` 读取完整窗口，先写 BMP，再转 PNG。
- **隐私裁剪：** 对话框 PNG 的左侧导航窗格用纯色矩形遮盖，因为其中显示当前登录账户的 OneDrive 显示名；标题栏、地址栏、文件列表、文件类型过滤器和按钮未改动。主窗口截图未裁剪。
- owner 禁用由 `IsWindowEnabled(main)` 在对话框打开时读取；主窗口截图同时显示非活动标题栏。取消和 Open 都由 UIAutomation 调用 shell 按钮完成，没有程序化关闭对话框。
- 对话框期间的 worker 完成用两次主窗口 `PrintWindow` 比较：慢任务 sleep 1200ms；第一次截图在对话框打开后约 0.4s，第二次在约 2s；`states.txt` 记录 `changed_pixels=2952`。第二张截图在对话框仍然打开时取得，画面出现 `slow task done`。
- 窗口状态、标题、class、owner、过滤器和完整文件名记录在 [`证据/file-picker/states.txt`](证据/file-picker/states.txt)。日志中 `changed_pixels` 是两次 BGRA 截图的逐字节差异数；`owner_enabled=false` 是原生启用位。

## 可见结果

| # | 步骤 | 预期 | 结果 | 截图 |
| --- | --- | --- | --- | --- |
| 1 | 启动 | 主窗口可见，两条种子任务，1 remaining | 通过 | [01-main](证据/file-picker/01-main.png) |
| 2 | 点击 `Import file` | 真实 shell 对话框；标题 `Import tasks`，class `#32770`，过滤器 `Text files (*.txt)`，owner 为主窗口且被禁用 | 通过 | [02-dialog-default](证据/file-picker/02-dialog-default.png) |
| 3 | 对话框保持打开，等待慢任务 | 主窗口在对话框打开时交付并重绘 worker 结果 | 通过；`changed_pixels=2952`，第二张截图出现 `slow task done` | [03 before](证据/file-picker/03-main-during-dialog-before.png) · [04 after](证据/file-picker/04-main-during-dialog-after.png) |
| 4 | 取消 | 列表不变；无导入提示或错误 | 通过；行仍为 Write tests / Read docs | [05-main-after-cancel](证据/file-picker/05-main-after-cancel.png) |
| 5 | 取消后在草稿框输入 `After dialog` | 焦点与输入在对话框关闭后仍可用 | 通过；draft=`After dialog`，focus=`NodeId(1)`，caret x=109.82/y=65 | [06-typing-after-dialog](证据/file-picker/06-typing-after-dialog.png) |
| 6 | 选择 `single.txt` | 一行标题；`Imported 1 task` | 通过；新增 `Single import` | [07-dialog-single](证据/file-picker/07-dialog-single.png) · [08-main-single](证据/file-picker/08-main-single.png) |
| 7 | 选择 `multi-a.txt` 与 `multi-b.txt` | 按顺序提交两行；`Imported 2 tasks` | 通过；新增 `Multi A`、`Multi B` | [09-dialog-multi](证据/file-picker/09-dialog-multi.png) · [10-main-multi](证据/file-picker/10-main-multi.png) |
| 8 | 选择无效 UTF-8 的 `invalid-utf8.txt` | 显示读取错误；列表不变 | 通过；`Import failed: ... stream did not contain valid UTF-8`，行仍为 5 条，成功提示已清空 | [11-dialog-invalid](证据/file-picker/11-dialog-invalid.png) · [12-main-invalid](证据/file-picker/12-main-invalid.png) |
| 9 | 选择长/Unicode 路径 `...\验收-𠀀-跨平台文件选择器-路径\导入标题-𠀀-很长的文件名称-用于路径回传验证.txt` | 路径完整回传并导入 | 通过；新增 `Unicode import`，`Imported 1 task`，旧错误已清空 | [13-dialog-unicode](证据/file-picker/13-dialog-unicode.png) · [14-main-unicode](证据/file-picker/14-main-unicode.png) |
| 10 | 最终状态 | 六行共享模型；草稿、焦点与 caret 保持 | 通过 | [15-main-final](证据/file-picker/15-main-final.png) |

关键状态（完整行见 `states.txt`）：

| 状态 | dialog | owner enabled | rows / notice / error |
| --- | --- | --- | --- |
| 启动 | — | true | 2 行 |
| 对话框打开 | `Import tasks` / `#32770` / 1281×801 / filter `Text files (*.txt)` | **false** | 2 行 |
| 取消后 | — | true | 2 行 |
| 单选后 | — | true | 3 行；`Imported 1 task` |
| 多选后 | — | true | 5 行；`Imported 2 tasks` |
| 无效 UTF-8 后 | — | true | 5 行；成功提示为空；读取错误 |
| Unicode 后 | — | true | 6 行；`Imported 1 task`；错误为空 |

## 证据等级与边界

- **可见自动驱动（本页）：** 真实可见 HWND、真实 shell 文件对话框、宿主默认原生 presenter、真实窗口过程、UIAutomation 操作与 `PrintWindow` 像素。不是人工操作；不证明用户注视下的 DWM 合成、z-order 或被其它窗口遮挡时的表现。
- **隐藏 smoke 与 CI WARP：** `crates/windows/tests/file_dialog.rs` 用注入 presenter 覆盖 anchor/options、嵌套消息、worker 唤醒、FIFO、隐藏宿主 `Unsupported` 与失效锚点；`crates/windows/tests/hidden_smoke.rs` 覆盖宿主关闭；`tests/file_picker.rs` 覆盖运行时请求/所有权/恰好一次完成。它们是路由与协议证据，不是原生验收。
- **headless 导入：** `crates/demo/tests/import.rs` 用 fake host 完成与临时文件覆盖行解析、计数、取消、读取/UTF-8 错误、以及主窗口关闭后的应用所有权导入；这不是原生窗口证据。
- **未覆盖：** 真实鼠标键盘与人工视觉确认；100% 缩放；程序化取消已打开的 shell 对话框；保存/文件夹对话框、拖放与菜单/托盘；初始目录/MRU 策略；多个同时打开的对话框；跨显示器 DPI 与实机 device loss（仍属 #63）。

## 判据对照

| 要求 | 状态 | 证据 |
| --- | --- | --- |
| 原生打开文件对话框锚定到窗口 | 通过 | 步骤 2；`states.txt` 的 owner HWND |
| 标题与过滤器映射 | 通过 | 步骤 2；截图 `Import tasks` 与 `Text files (*.txt)`；UIA 日志 |
| owner 模态禁用 | 通过 | 步骤 2；`owner_enabled=false`；主窗口非活动标题栏 |
| 取消不改变列表 | 通过 | 步骤 4 |
| 单选与多选 | 通过 | 步骤 6、7 |
| 导入计数与列表一致 | 通过 | 步骤 6、7、9 |
| 读取/UTF-8 错误可见且不破坏列表 | 通过 | 步骤 8 |
| 长/Unicode/非 BMP 路径 | 通过 | 步骤 9；`states.txt` 中的完整路径 |
| 关闭后焦点与输入 | 通过 | 步骤 5；focus/caret 与截图 |
| 对话框期间 worker 完成 | 通过 | 步骤 3；两次截图与 `changed_pixels=2952` |
| 应用所有权导入可在 anchor 关闭后完成 | 由 headless 测试覆盖，本页不宣称原生证据 | `crates/demo/tests/import.rs::an_import_can_finish_after_the_main_window_closes` |
| 隐藏/注入证据不冒充原生验收 | 通过 | 本页「证据等级与边界」；ADR-0005、ADR-0008 |

## 结论

在 Windows 11、150% 缩放的可见窗口上，`Import file` 打开了真实 `IFileOpenDialog`：标题为 `Import tasks`，类型过滤器为 `Text files (*.txt)`，owner 在对话框期间被 shell 禁用。取消不改变列表；单选、多选、无效 UTF-8、长/Unicode 路径分别得到正确的导入、计数与错误行为；对话框关闭后草稿仍可聚焦和输入；慢 worker 在对话框打开期间完成并重绘。证据是自动驱动而非人工试跑；隐藏 smoke 与 CI 结果只覆盖路由与协议，不作为原生验收。
