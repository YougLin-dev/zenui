# stage-three 文件拖放验收

本页记录 Windows 宿主从资源管理器接收 OLE 文件拖放的可见结果：单文件与多文件、Unicode/非 BMP/长路径、文件夹、非文件数据、无处理器窗口、关闭提示、模态选择器打开、关闭重开与拖放后可用性。契约见 [ADR-0016](../adr/0016-ole-file-drop-target.md)，SDK 用法见[窗口与任务](../SDK/窗口与任务.md#文件拖放的事件与所有权)。

- 基线：`main` `3cb3cdb`（#208 合并后的 OLE drop target；多文件图沿用同一实现）
- 示例：`crates/windows/examples/todo_host.rs`（ZenUI Todo / ZenUI Details），debug 构建
- 平台：Windows 11，单屏 2560×1600

## 环境与采集

| 项目 | 100% | 150% |
| --- | --- | --- |
| 显示 | 96 DPI（从 150% 临时切换，采集后恢复 144 DPI） | 144 DPI（本机当前设置） |
| 主窗口 | 916×739 物理像素，屏幕 (1200,80) | 1372×1106 物理像素，屏幕 (980,40) |
| 资源管理器 | 1000×950，屏幕 (40,80) | 900×1100，屏幕 (20,40) |
| 详情窗口 | — | 802×686，屏幕 (1400,360) |
| 关闭提示 | — | 652×386，屏幕 (1500,720) |

- 新采集的样本位于 `E:\code\mine\zenui\target\drop-acceptance`：`single.txt`、`multi-a.txt`、`multi-b.txt`、`unicode-你好-🙂.txt`、`folder-drop/` 与 272 字符的长路径文件。多文件图沿用 #208 的 `drop-two.txt` 与 `drop-unicode-你好-🙂.txt`；文件名与内容出现在图中，源目录未显示。
- 资源管理器窗口每次由脚本打开并置于应用左侧，Details 视图。单文件直接点击；多文件图使用真实点击第一行后 Ctrl+A 选中两行。每次拖动前由 UIA `SelectionItemPattern` 读回选中数量。
- 拖动是仓库外 PowerShell 脚本注入的 Win32 鼠标事件：按下后先移出拖拽阈值，再分 30 步移到目标，停留后释放。拖放中的截图在按钮仍按下时用 `BitBlt` 的 `SRCCOPY|CAPTUREBLT` 抓取；位图不含硬件光标，脚本按当前位置补画。
- 应用窗口图来自 `PrintWindow(PW_RENDERFULLCONTENT)`。资源管理器证据图只裁文件列表，不含导航窗格、地址栏或窗口标题；原生对话框只保留标题条与文件列表；Notepad 只保留文档区。截图不含 OneDrive 登录名或其它个人路径。
- Notepad 复用用户已打开的窗口，仅新增并操作 `non-file.txt` 标签页；未打开或修改其它恢复标签页，也未读取其文档内容。采集后关闭该标签页并把窗口尺寸恢复，Notepad 保持打开。
- 应用任务在落点后约 1.8–2.8 秒读取；截图中的 `Imported N task(s)` 与任务行是最终可见结果。原始场景读数见 `证据/host-drop/states.txt` 的 #209 段。

## 单文件与多文件

`single.txt` 含一行 `Single drop`。100% 与 150% 都显示 `Imported 1 task`，任务行接在两条种子任务之后。

| 显示 | 拖入文件 | 拖后任务顺序 | 证据 |
| --- | --- | --- | --- |
| 100% | `single.txt` | `Write tests`、`Read docs`、`Single drop`；2 remaining | `100-single-01-before.png`、`100-single-02-drag.png`、`100-single-03-after.png` |
| 150% | `single.txt` | 同左；2 remaining | `150-single-01-before.png`、`150-single-02-drag.png`、`150-single-03-after.png` |

多文件由 `drop-two.txt`（`Drop alpha`、`Drop beta`）与 `drop-unicode-你好-🙂.txt`（`Unicode drop`）组成，两行在资源管理器中同时选中。

| 显示 | 拖后任务顺序 | 通知 | 证据 |
| --- | --- | --- | --- |
| 100% | `Write tests`、`Read docs`、`Drop alpha`、`Drop beta`、`Unicode drop` | `Imported 3 tasks`；4 remaining | `100-00-explorer.png`、`100-01-before.png`、`100-02-drag.png`、`100-03-after.png` |
| 150% | 同左 | 同左；4 remaining | `150-00-explorer.png`、`150-01-before.png`、`150-02-drag.png`、`150-03-after.png` |

两个文件在 100% 与 150% 两次运行中都按资源管理器中的顺序进入列表；`drop-unicode-你好-🙂.txt` 的名字同时包含 CJK 与非 BMP 字符，说明 `DragQueryFileW` 到 `DropEvent` 的路径没有经过 ANSI 转换或替换。

## Unicode、非 BMP 与长路径

272 字符的 `long-path.txt` 位于 `target/drop-acceptance` 下 7 层长目录中（超过 `MAX_PATH` 的 260 字符）。从该目录拖动后应用显示 `Imported 1 task`，任务标题为 `Long path drop`，任务数为 3。

| 场景 | 读数 | 证据 |
| --- | --- | --- |
| 272 字符路径 | 标题 `Long path drop`；`Imported 1 task`；2 remaining | `150-longpath-00-explorer.png`、`150-longpath-01-before.png`、`150-longpath-02-drag.png`、`150-longpath-03-after.png` |

路径转换使用 `OsString::from_wide`，因此未配对代理项也不会被替换；隐藏测试覆盖未配对代理项，多文件图覆盖非 BMP 文件名。

## 文件夹与非文件数据

| 场景 | 源 | 结果 | 证据 |
| --- | --- | --- | --- |
| 文件夹 | 资源管理器中的 `folder-drop` 目录 | 处理器运行并显示 `Import failed: Could not read E:\code\mine\zenui\target\drop-acceptance\folder-drop: 拒绝访问。 (os error 5)`；任务仍为 2，1 remaining | `150-folder-00-explorer.png`、`150-folder-01-before.png`、`150-folder-02-drag.png`、`150-folder-03-after.png` |
| 非文件数据 | Notepad 中选中的文档文本 `Not a file drag` | 应用任务、通知与错误都不变 | `150-nonfile-01-before.png`、`150-nonfile-02-drag.png`、`150-nonfile-03-after.png` |

文件夹是合法的 `CF_HDROP` 路径，宿主不替应用决定是否接受；Todo 的导入器只读文本文件，因此把文件夹变成可见错误。Notepad 文本不提供 `CF_HDROP`，目标在 `DragEnter` 返回 NONE，处理器从未运行；截图显示文本仍选中、光标位于应用内，而应用保持两条任务。

## 无处理器窗口与关闭提示

主窗口草稿为 `Unsaved drop draft` 时发送关闭请求，出现 `Discard draft?` 确认窗口。确认窗口没有 `on_drop`，父窗口处于关闭挂起；把 `single.txt` 拖到确认窗口后：

- 任务仍为 `Write tests`、`Read docs`，没有通知或错误；
- 确认窗口保持打开，父窗口保持禁用；
- 通过 UIA 触发 Continue editing 后确认窗口关闭，主窗口恢复，草稿仍为 `Unsaved drop draft`。

证据：`150-close-prompt-01-main.png`、`150-close-prompt-02-confirm.png`、`150-close-prompt-03-drag.png`、`150-close-prompt-04-confirm-after.png`、`150-close-prompt-05-main-after-deny.png`。

## 模态选择器打开

通过 UIA 触发主窗口的 Import file 打开原生 `Import tasks` 对话框，把对话框移到屏幕下方，使主窗口内容区仍可见；从资源管理器拖动 `single.txt` 到被 shell 禁用的主窗口。

| 检查 | 结果 | 证据 |
| --- | --- | --- |
| 对话框 | `Import tasks` 标题与文件列表可见，拖放后仍打开 | `150-picker-00-title.png`、`150-picker-01-dialog.png`、`150-picker-04-dialog-after.png` |
| 主窗口 | 任务仍为 2，无通知或错误 | `150-picker-02-main.png`、`150-picker-03-drag.png` |
| 取消 | 通过 UIA 调用取消按钮后对话框关闭，主窗口恢复 | `150-picker-05-main-after-cancel.png` |

模态期间 owner 由 shell 禁用，目标的 `IsWindowEnabled` 检查返回 false；宿主不需要额外的模态标志，也没有把拖放排入队列。

## 关闭与重开

打开 Details，把 `single.txt` 拖入：主窗口任务变为 3，Details 显示 `Imported 1 task`。关闭 Details 后从主窗口重新打开，再拖入 `multi-a.txt`：新的 activation 得到新 HWND（`0x3DC11A6` → `0x133079A`），主窗口任务变为 4，Details 显示 `Imported 1 task`。旧 Details 的 target 与队列随关闭释放，第一次拖放没有重复进入新窗口。

| 阶段 | 主窗口任务 | Details | 证据 |
| --- | --- | --- | --- |
| 第一次打开 | 3 | `Imported 1 task`，行 `Write tests`、`Read docs`、`Single drop` | `150-details-01-drag.png`、`150-details-02-first-after.png` |
| 关闭后 | 3 | 窗口已释放 | `150-details-03-main-after-close.png` |
| 重开并再次拖入 | 4 | `Imported 1 task`，新增 `Multi A` | `150-details-04-reopen-drag.png`、`150-details-05-reopen-after.png` |

## 拖放后可用性

100% 单文件拖放后，UIA 把草稿设置为 `Post drop task` 并点击 Add：列表追加该任务，remaining 从 2 变为 3，`Imported 1 task` 通知仍在。拖放没有冻结事件循环、焦点或模型更新。

证据：`100-single-04-postdrop.png`。

## 隐藏测试与 CI

- workspace 843 个测试通过；`cargo fmt --all -- --check` 与 workspace clippy `-D warnings` 通过。
- `drag_drop` 模块 7 个测试：Unicode/重复路径按序提取、未配对代理项、缺失或空 `CF_HDROP`、COPY/启用/文件列表判定、注册策略、medium 释放、`RPC_E_CHANGED_MODE` 映射。
- Windows 宿主隐藏测试：带 `on_drop` 的窗口在隐藏宿主中不创建 target，也不初始化 OLE。
- 运行时校验、handler 阶段、任务 scope、关闭/重开与 handler panic 由 #207 的 10 个测试覆盖。
- 模态 shell 对话框、真实资源管理器和非文件源只能由本页的可见运行证明；fake `IDataObject`、隐藏 smoke 与合成 `Runtime::drop_files` 是部分证据。

## 边界

- 只在单屏 2560×1600 运行；100% 临时切换后恢复 150%，不声明混合 DPI 或多显示器。
- 150% 覆盖本页列出的边界场景；100% 覆盖单文件、多文件、拖后可用性。长路径、文件夹、非文件、关闭提示和模态选择器未在 100% 重复采集。
- 只覆盖 Windows 资源管理器文件列表与 Notepad 文本两个 OLE 源；压缩工具、浏览器下载列表、虚拟文件与网络路径没有真机证据。
- 只实现 `CF_HDROP` 的 COPY；文本、URL、图片、MOVE/LINK、拖放位置、drag image、自动滚动与程序化拖动不在本切片。
- 文件夹是路径，接受与否由应用处理器决定；Todo 的读取错误是应用行为，不是宿主拒绝。
- 新采集场景的文件与错误路径来自仓库内的 `target`；多文件图的源目录未显示，文件名与内容为公开测试字符串。导航窗格、地址栏和个人标签页标题被裁掉或未采集。
