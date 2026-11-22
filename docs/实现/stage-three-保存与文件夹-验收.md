# Stage-three 保存与文件夹对话框验收（实机记录）

本页记录阶段三保存与文件夹对话框的可见 Windows 验收：原生 `IFileSaveDialog`、标题与过滤器、建议文件名与默认扩展名、两过滤器扩展名优先级、覆盖确认与取消、owner 模态禁用、取消、成功保存的精确字节、shell 写入拒绝、长/Unicode 路径、文件夹选择、关闭后的焦点/输入、Details 共享提示与工具栏 containment。契约与取舍见 [ADR-0009](../adr/0009-file-dialog-family.md)；打开文件对话框的早期记录见 [stage-three 文件对话框验收](stage-three-文件对话框-验收.md)。

**状态：可见自动驱动验收完成（150%）。** 运行使用真实可见 HWND、真实 shell `IFileSaveDialog` / 文件夹选择器和宿主默认 presenter，没有注入 presenter。主窗口输入通过投递的原生鼠标/键盘消息完成；shell 对话框的文件名与按钮由 UI Automation / Win32 消息驱动，不是人工鼠标键盘。截图证明可见窗口中的状态与像素，不证明用户注视下的 DWM 时序。隐藏 smoke 与 CI 是部分证据，不构成本页的原生验收。

验收日期：2026-11-23。验收对象为 `ffa81fc`（#164 合并后的 `main`）。文档提交不改变行为。

## 环境

| 项 | 值 |
| --- | --- |
| 操作系统 | Windows 11 专业版 10.0.26300，Build 26300，64 位，交互式会话 |
| 显示缩放 | DPI 144（150%）；主窗口逻辑 900×700，客户区 1350×1050 物理像素 |
| 显示 | 单显示器；本页不声明 100% 缩放结果 |
| 图形 | NVIDIA GeForce RTX 4060 Laptop GPU / 驱动 32.0.15.7283；宿主默认 DX12 路径 |
| 保存对话框 | shell `IFileSaveDialog`，标题 `Save tasks`，class `#32770`，客户区 1258×745 物理像素 |
| 文件夹对话框 | shell 文件夹选择器，标题 `Choose a folder`，class `#32770`，客户区 1259×745 |
| 输入方式 | 主窗口投递 `WM_LBUTTONDOWN` / `WM_LBUTTONUP` / `WM_CHAR`；对话框文件名与按钮由 UI Automation 和 Win32 `SendMessage` 驱动 |

## 方法

- 使用仓库自带的 `cargo run -p zenui-windows --example todo_host`（release 构建）运行 `zenui_demo::TodoApp`，宿主使用 `HostConfig::visible()` 与默认 presenter。保存与文件夹按钮是 demo 的真实实现。
- 每个对话框先导航到 `E:\zenui-acceptance\save-folder\data` 临时目录，再截图和选择；file list 只包含探针文件，不包含用户其它目录。
- 对话框截图用 `PrintWindow(PW_RENDERFULLCONTENT)` 读取客户区，写 PNG。**隐私裁剪：** 左侧导航树用纯黑矩形遮盖，因为它可能显示当前登录账户的 OneDrive 显示名；标题栏、地址栏、file list、file type filter 和按钮未改动。
- 保存对话框的文件名通过 UI Automation `ValuePattern` 设置；保存/取消/覆盖确认通过 UIA 或 Win32 按钮消息驱动。文件夹 title、class、owner、`IsWindowEnabled(owner)`、过滤器与完整路径记录在 [`证据/save-folder/states.txt`](证据/save-folder/states.txt)。
- 两过滤器优先级与默认扩展名使用一个**临时、未提交**的可见探针，验收后删除。两过滤器跑两次：filters `[Markdown (*.md), Text files (*.txt)]` + 默认扩展名 `txt`，以及 `[Text files (*.txt), Markdown (*.md)]` + 默认扩展名 `md`。默认扩展名跑一次：无过滤器、建议名 `probe-ext`、默认扩展名 `dat`。
- 对话框期间 worker 完成本页**未观测**：demo 没有 slow-task 控件，本次没有添加临时 worker 探针。之前的打开文件对话框可见验收（`5f2cd54`，#152/#153）已经记录过慢 worker 在对话框打开期间完成并重绘；本页引用该记录，不重复宣称。

## 可见结果

| # | 步骤 | 预期 | 结果 | 截图 |
| --- | --- | --- | --- | --- |
| 1 | 启动 | 主窗口可见，两条种子任务，工具栏第二行是 `Save tasks` / `Choose folder`，两个按钮都在 900×700 客户区内 | 通过 | [01-main](证据/save-folder/01-main.png) |
| 2 | 点击 `Save tasks` | 真实 `IFileSaveDialog`；标题 `Save tasks`，class `#32770`，类型 `Text files (*.txt)`，文件名 `tasks.txt`，owner 为主窗口且被禁用 | 通过；`owner_enabled=false`，type_count=1 | [02-save-dialog](证据/save-folder/02-save-dialog.png) |
| 3 | 保存到 `data\tasks.txt` | 文件为 UTF-8，每个标题一行；成功提示 `Saved 2 tasks`，错误清空 | 通过；22 字节，sha256 `6335e6b9…05f5` | [03-main-saved](证据/save-folder/03-main-saved.png) |
| 4 | 覆盖已有文件后取消 | 覆盖确认出现；选择否/取消后文件保持旧内容，notice 与 error 不变 | 通过；`old contents` 未被替换 | [04-overwrite-prompt](证据/save-folder/04-overwrite-prompt.png) · [05-main-after-save-cancel](证据/save-folder/05-main-after-save-cancel.png) |
| 5 | 覆盖确认 | 确认后文件被替换为保存内容 | 通过；字节与 sha256 同步骤 3 | — |
| 6 | 点击 `Choose folder` | 文件夹选择器无 file type filter；标题 `Choose a folder`；owner 被禁用 | 通过 | [06-folder-dialog](证据/save-folder/06-folder-dialog.png) |
| 7 | 取消文件夹选择 | notice 与 error 不变 | 通过 | [07-main-after-folder-cancel](证据/save-folder/07-main-after-folder-cancel.png) |
| 8 | 选择文件夹 | notice 显示 `Folder: <path>`，错误清空；Details 也显示同一提示 | 通过；Details 见右图 | [08-main-folder-selected](证据/save-folder/08-main-folder-selected.png) · [09-details-folder-notice](证据/save-folder/09-details-folder-notice.png) |
| 9 | 对话框关闭后在草稿框输入 `After dialog` | 焦点与输入仍可用，notice 不变 | 通过；draft=`After dialog`，有焦点环和 caret | [10-main-typing-after-dialog](证据/save-folder/10-main-typing-after-dialog.png) |
| 10 | 保存到长/Unicode 路径 | 完整路径回传并写入；内容与普通保存一致 | 通过；非 BMP `𠀀` 路径创建成功，22 字节同 sha256 | [11-save-dialog-unicode](证据/save-folder/11-save-dialog-unicode.png) · [12-main-unicode-save](证据/save-folder/12-main-unicode-save.png) |
| 11 | 保存到受保护位置 `C:\Windows\...` | shell 创建测试拒绝，应用不写文件 | 通过；shell 提示无权限并询问是否改存 `文档`；取消后应用状态不变 | [13-write-failure-shell-rejection](证据/save-folder/13-write-failure-shell-rejection.png) |
| 12 | 覆盖只读文件 | shell 拒绝只读替换，应用不写文件 | 通过；提示只读并保持原文件 | [14-readonly-warning](证据/save-folder/14-readonly-warning.png) |
| 13 | 两过滤器扩展名优先级（Markdown 在前） | 选中类型优先于显式默认扩展名 | 通过（临时探针）；默认类型 `Markdown (*.md)`，文件名 `probe.md`，返回 `…\probe.md`，而默认扩展名是 `txt` | [15-two-filter-markdown-default](证据/save-folder/15-two-filter-markdown-default.png) · [16-probe-markdown-result](证据/save-folder/16-probe-markdown-result.png) |
| 14 | 两过滤器扩展名优先级（Text 在前） | 同上，方向相反 | 通过（临时探针）；默认类型 `Text files (*.txt)`，文件名 `probe.txt`，返回 `…\probe.txt`，而默认扩展名是 `md` | [20-two-filter-text-default](证据/save-folder/20-two-filter-text-default.png) · [21-two-filter-text-result](证据/save-folder/21-two-filter-text-result.png) |
| 15 | 默认扩展名 | 建议名不带扩展名时应用默认扩展名 | 通过（临时探针）；无过滤器、建议名 `probe-ext`、默认扩展名 `dat`，对话框文件名 `probe-ext.dat`，返回 `…\probe-ext.dat` | [18-default-extension](证据/save-folder/18-default-extension.png) · [19-default-extension-result](证据/save-folder/19-default-extension-result.png) |
| 16 | 对话框期间 owner 状态 | 主窗口被 shell 禁用 | 通过；主窗口 `visible=true enabled=false`，对话框 `visible=true enabled=true`。`17` 是当时 owner 的客户区截图，不能单独显示对话框 | [17-main-during-dialog](证据/save-folder/17-main-during-dialog.png) |
| 17 | 对话框期间 worker 完成 | 慢 worker 结果可在对话框打开时重绘 | **本次未观测**；见「证据等级与边界」的打开文件验收引用 | — |

关键状态（完整记录见 `states.txt`）：

| 状态 | dialog | owner enabled | notice / error | 文件 |
| --- | --- | --- | --- | --- |
| 启动 | — | true | 空 | — |
| 保存对话框打开 | `Save tasks` / `#32770` / 1258×745 / `Text files (*.txt)` / `tasks.txt` | **false** | 空 | — |
| 保存成功 | — | true | `Saved 2 tasks` / 空 | 22 字节，sha256 `6335e6b9…` |
| 覆盖取消 | — | true | `Saved 2 tasks` / 空 | `old contents` 未变 |
| 文件夹取消 | — | true | `Saved 2 tasks` / 空 | — |
| 文件夹选择 | — | true | `Folder: E:\zenui-acceptance\save-folder\data` / 空 | — |
| Unicode 保存 | — | true | `Saved 2 tasks` / 空 | 22 字节，同 sha256 |
| shell 写入拒绝 | `Save tasks` / 527×230 | true | 未变 | 未写 |
| 只读拒绝 | `Save tasks` / 527×182 | true | 未变 | `protected` 未变 |
| 两过滤器（Markdown 在前） | `ZenUI Two Filters` / 1259×744 / `Markdown (*.md)` / `probe.md` | false（probe owner） | 返回 `…\probe.md` | 探针不写文件 |
| 两过滤器（Text 在前） | `ZenUI Two Filters` / 1259×742 / `Text files (*.txt)` / `probe.txt` | false（probe owner） | 返回 `…\probe.txt` | 探针不写文件 |
| 默认扩展名 | `ZenUI Default Extension` / 1259×743 / 无类型 / `probe-ext.dat` | false（probe owner） | 返回 `…\probe-ext.dat` | 探针不写文件 |

## 证据等级与边界

- **可见自动驱动（本页）：** 真实可见 HWND、真实 shell `IFileSaveDialog` 和文件夹选择器、宿主默认 presenter、真实窗口过程、UIAutomation/Win32 按钮操作与 `PrintWindow` 像素。不是人工操作；不证明用户注视下的 DWM 合成、z-order 或被其它窗口遮挡时的表现。
- **两过滤器探针：** 临时可见探针，验证 filter order 与扩展名优先级；验收后删除，不进入提交。两次运行分别在 filter 顺序和默认扩展名上相反，结果都跟随初始选中的类型，因此本页宣称“初始选中类型覆盖显式默认扩展名”。类型切换本身没有通过 UIAutomation 可靠驱动（控件以 Pane 暴露、不支持 ExpandCollapse，投递的键盘/鼠标消息也没有打开 shell 下拉），因此本页不宣称“用户切换到第二项”的交互。
- **默认扩展名探针：** 同一临时探针的无过滤器分支，建议名不带扩展名、默认扩展名 `dat`；对话框直接显示 `probe-ext.dat`，返回路径也带 `.dat`。这条与两过滤器运行相互独立。
- **写入失败：** 两种不可写目标都被 shell 的创建测试在返回路径之前拒绝，因此应用级 worker 写入错误没有被原生触发。`crates/demo/tests/save.rs` 的 unwritable 用例覆盖应用错误面，是 headless 部分证据；本页的原生证据是 shell 拒绝。
- **对话框期间 worker：** 本次未观测。宿主任务/worker 交付路径不属于本里程碑的改动；[stage-three 文件对话框验收](stage-three-文件对话框-验收.md) 的步骤 3（提交 `5f2cd54`，#152/#153）记录了慢 worker 在打开对话框期间完成（`changed_pixels=2952`）。本页不把该旧记录当作本次运行的结果。
- **隐藏 smoke 与 CI：** `crates/windows/tests/file_dialog.rs` 用注入 presenter 覆盖 save/folder options、arity、取消/失败与 FIFO；`crates/demo/tests/save.rs` 覆盖 headless 保存/文件夹行为；`crates/render/tests/scene.rs` 覆盖工具栏几何 containment，`tests/presentation.rs` 只覆盖元素存在与顺序。它们是路由、协议与布局证据，不是原生验收。
- **未覆盖：** 真实鼠标键盘与人工视觉确认；100% 缩放；程序化取消已打开的 shell 对话框；初始目录/MRU 策略；多个同时打开的对话框；跨显示器 DPI 与实机 device loss（仍属 #63）。

## 判据对照

| 要求 | 状态 | 证据 |
| --- | --- | --- |
| 真实保存对话框：标题、过滤器、建议名 | 通过 | 步骤 2；`states.txt` |
| 默认扩展名 | 通过（临时探针） | 步骤 15；18/19 |
| 两过滤器扩展名优先级 | 通过（临时探针，两种 filter 顺序） | 步骤 13、14；15/16、20/21 |
| 覆盖确认与取消 | 通过 | 步骤 4、5；04/05 |
| owner 模态禁用 | 通过 | 步骤 2、16；`owner_enabled=false` |
| 取消不改变列表/notice/error | 通过 | 步骤 4、7；05/07 |
| 成功保存的精确字节 | 通过 | 步骤 3、5、10；22 字节与 sha256 |
| 写入失败 | shell 拒绝通过；应用级错误见 headless 部分证据 | 步骤 11、12；13/14 |
| 长/Unicode/非 BMP 路径 | 通过 | 步骤 10；11/12 + `states.txt` |
| 文件夹选择 | 通过 | 步骤 6、8；06/08 |
| 关闭后焦点与输入 | 通过 | 步骤 9；10 |
| Details 共享提示 | 通过 | 步骤 8；09 |
| 工具栏 containment | 通过 | 步骤 1；01 + scene 测试 |
| 对话框期间 worker 完成 | 本次未观测（引用打开文件验收） | 步骤 17；见证据边界 |
| 隐藏/CI 不冒充原生验收 | 通过 | 本页「证据等级与边界」 |

## 结论

在 Windows 11、150% 缩放的可见窗口上，`Save tasks` 打开了真实 `IFileSaveDialog`：标题为 `Save tasks`，类型为 `Text files (*.txt)`，文件名为 `tasks.txt`，owner 在对话框期间被 shell 禁用。成功保存写出精确的 22 字节 UTF-8 内容并显示 `Saved 2 tasks`；覆盖确认、取消、长/Unicode 路径与文件夹选择都符合契约；不可写目标由 shell 在返回路径前拒绝。临时探针证明默认扩展名会应用于不带扩展名的建议名，并且两种 filter 顺序下初始选中类型都优先于显式默认扩展名；交互式切换类型没有驱动，本页不作此宣称。证据是自动驱动而非人工试跑；隐藏 smoke 与 CI 结果只覆盖路由与协议，不作为原生验收。
