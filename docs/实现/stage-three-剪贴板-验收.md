# stage-three 剪贴板验收（复制、剪切与粘贴）

本页记录 Ctrl+C / Ctrl+X / Ctrl+V 在聚焦文本输入上的实现与可见证据。范围是剪贴板读写、隐藏注入 seam 与 Notepad 互操作；本页的可见场景覆盖复制与粘贴，剪切的可见证据与完整契约、100% 验收在后续文档补充。

- 基线：`feat/201-clipboard-copy-paste`（#201）、`feat/202-cut-after-write`（#202）
- 示例：`crates/windows/examples/todo_host.rs`（ZenUI Todo），debug 构建
- 平台：Windows 11，单屏 2560×1600，150%（144 DPI）
- 外部程序：Notepad 11.2609.11.0（`RichEditD2DPT` 文档区）

## 方法

- `ClipboardProvider` 是宿主的剪贴板 seam：可见宿主安装 `CF_UNICODETEXT` 原生 provider，隐藏宿主默认没有 provider，因此隐藏测试不触碰系统剪贴板。嵌入方可用 `Host::set_clipboard_provider` 注入自己的实现；读写失败与 provider panic 都收敛为静默无操作。
- Ctrl+C 从当前呈现快照读取聚焦输入的已提交选区，按 Unicode 标量切片后写入 provider；折叠 selection、没有选区、没有 provider 或写入失败都不派发 `on:input`。
- Ctrl+X 复用 Ctrl+C 的写入路径，只有 provider 确认写入成功后才用 Backspace 语义删除选区；成功时恰好派发一次 `on:input`，caret 落在原选区起点。写入失败或没有 provider 时文本、选区与 caret 都不变；折叠选区、禁用、无焦点与组合中不写也不删。
- Ctrl+V 读取剪贴板文本，过滤全部控制字符（含 NUL、CR/LF），再调用已有的 `Runtime::insert_text_node`。选区存在时替换选区，否则插入 caret；成功粘贴恰好派发一次 `on:input`，caret 落在插入文本之后。空文本、非文本、读取失败与 panic 不改写模型，也不删除选区。
- AltGr 在 Windows 上表现为 Ctrl+Alt；宿主在进入快捷键策略前排除 Alt，AltGr 组合继续作为字符输入，不会触发复制、剪切或粘贴。
- Notepad 自带的恢复标签页包含与本任务无关的本地文件。采集只读取活动标签页的 UIA 文档并截取文档区域，不截图标签栏，也不打开、修改或关闭其它标签页。

## 应用复制到 Notepad

主窗口草稿键入 `ZenUI clipboard`，Ctrl+A 后 UIA 选区为 `[ZenUI clipboard]`（15 个 UTF-16 码元），Ctrl+C。

| 读数 | 值 |
| --- | --- |
| 复制前草稿 | `ZenUI clipboard`（`90,101,110,85,73,32,99,108,105,112,98,111,97,114,100`） |
| 复制前选区 | `selected=[ZenUI clipboard]` |
| Notepad 粘贴后文本 | `ZenUI clipboard`，长度 15，码元与草稿一致 |
| 草稿 `on:input` | 复制不新增；捕获前的键入由隐藏测试与历史验收覆盖 |

证据：`150-notepad-from-zenui.png`。Notepad 文档区显示同一文本，说明 `SetClipboardData` 写入的 `CF_UNICODETEXT` 被系统里另一个进程读取。

## Notepad 复制到应用

在 Notepad 活动标签页放入外部文本 `Notepad to ZenUI 你好 🙂`（22 个 UTF-16 码元；来源是仓库外的 PowerShell 系统剪贴板写入，不是 ZenUI）。随后在 Notepad 中 Ctrl+A、Ctrl+C，把剪贴板所有权交给 Notepad。

| 读数 | 值 |
| --- | --- |
| Notepad 文本 | `Notepad to ZenUI 你好 🙂`（`78,111,116,101,112,97,100,32,116,111,32,90,101,110,85,73,32,20320,22909,32,55357,56898`） |
| 应用粘贴前草稿 | `ZenUI clipboard`，Ctrl+A 全选 |
| 应用粘贴后文本 | `Notepad to ZenUI 你好 🙂`，长度 22，码元与 Notepad 一致 |
| 粘贴后选区 | 折叠；caret 在插入文本之后 |
| 粘贴后 Ctrl+Z | 恢复 `ZenUI clipboard` 与粘贴前的全选选区 |

证据：`150-notepad-external.png`、`150-zenui-from-notepad.png`、`150-zenui-paste-undo.png`。中文与 emoji 的码元表明 provider 读写的是 UTF-16，运行时按 Unicode 标量编辑；空格与 CJK 在往返中保持不变。Ctrl+Z 一步恢复粘贴前的值与选区，说明一次粘贴只产生一条历史记录。

## 隐藏测试

- `crates/windows/src/clipboard.rs`：`single_line_text` 去掉全部控制字符并保留 CJK/emoji；非法 UTF-16 用替换字符解码。
- `crates/windows/src/host.rs` 新增 13 个宿主测试。复制/粘贴 9 个：精确复制选区且零 `on:input`；折叠选区、禁用输入与无焦点不写剪贴板；代理对复制与粘贴；caret 插入、一次 `on:input` 与一步撤销；替换选区、过滤控制字符、空/非文本/读取失败保留选区；组合中 no-op；隐藏宿主无 provider 无 I/O；AltGr 不触发快捷键；跨窗口共用一个 provider 复制/粘贴且撤销不写另一窗口。
- 剪切 4 个：成功写入后删除选区、恰好一次 `on:input`、caret 在起点且一步撤销恢复选区；写入失败与缺少 provider 都保留文本/选区/caret；折叠选区、禁用与组合中 no-op；主窗口与详情窗口之间剪切/粘贴且焦点不被移动。删除复用 `Runtime::backspace_node`，其选择删除语义由 `tests/text_edit.rs` 的 `backspace_and_delete_forward_respect_caret_and_selection` 固定。
- 这些测试使用内存 provider，不读取或写入系统剪贴板；原生 `OpenClipboard` 重试、`EmptyClipboard`/`GlobalAlloc`/`GlobalLock`/`SetClipboardData` 与每条路径的 `CloseClipboard` 由实现结构与可见互操作共同覆盖。193 个 `zenui-windows` 库测试与 workspace 825 个测试通过。

## 边界

- 只在单屏 150% 运行；100% 与混合 DPI 属于后续剪贴板总验收。
- 本页的可见场景覆盖复制与粘贴；剪切的可见证据与空/非文本剪贴板、组合 no-op、主窗口到详情窗口的完整验收在后续文档补充，当前由上面列出的隐藏测试覆盖。
- 可见互操作只覆盖 Notepad；其它应用、富文本、图片与多格式剪贴板不在本工单范围。
- 可见复制读取的是 UIA 已提交文本与选区；系统剪贴板内容由 Notepad 粘贴结果间接观察，没有直接读取剪贴板原始字节。
- Notepad 的其它恢复标签页未被修改；截图不含它们的标题或内容。采集结束时 Notepad 与其中的本地文件保持原状，证据文本仍留在系统剪贴板。
- 隐藏/CI 是部分证据；本页只记录列出的场景，没有逐项人工视觉复核。
