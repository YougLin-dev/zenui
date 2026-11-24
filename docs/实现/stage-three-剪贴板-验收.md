# stage-three 剪贴板验收

本页记录 Windows 宿主在聚焦文本输入上的 Ctrl+C / Ctrl+X / Ctrl+V：`CF_UNICODETEXT` 读写、Notepad 双向互操作、应用内剪切与跨窗口粘贴、空/非文本剪贴板、组合 no-op、焦点保持，以及 100% 与 150% 可见证据。契约见 [ADR-0015](../adr/0015-clipboard-host-capability.md)。

- 基线：`main` `ac58dcd`（#201 复制/粘贴、#202 剪切）
- 示例：`crates/windows/examples/todo_host.rs`（ZenUI Todo / ZenUI Details），debug 构建
- 平台：Windows 11，单屏 2560×1600
- 外部程序：Notepad 11.2609.11.0（`RichEditD2DPT` 文档区）

## 环境与采集

| 项目 | 100% | 150% |
| --- | --- | --- |
| 显示 | 96 DPI（从 150% 临时切换，采集后恢复 144 DPI） | 144 DPI（本机当前设置） |
| 主窗口 | 916×739 物理像素，屏幕 (48,48)，客户区 900×700 | 1372×1106 物理像素，屏幕 (48,48)，客户区 1350×1050 |
| 详情窗口 | 536×459 物理像素 | 802×686 物理像素 |

- ZenUI 窗口图来自 `PrintWindow(PW_RENDERFULLCONTENT)`，整窗读取；详情窗口单独成图。
- Notepad 文档图来自 `CopyFromScreen`，只取活动标签页文档区的屏幕矩形；不截标签栏，不读取或修改 Notepad 恢复的其它标签页。
- 键盘与鼠标由仓库外的 PowerShell 脚本用 `keybd_event` / `mouse_event` 注入；每次输入前用 `SetForegroundWindow` 置前并确认 `fg` 与目标 HWND 相同。
- 文本与选区来自仓库外的 UIA `TextPattern` 客户端；Notepad 活动标签页同样由 `TextPattern` 读取，偏移是 UTF-16 单元。
- Notepad 自带的恢复标签页包含与本任务无关的本地文件。采集只操作活动标签页；截图与读数不含其它标签页的标题或内容。

## 方法

- `ClipboardProvider` 是宿主 seam：可见宿主安装 `CF_UNICODETEXT` 原生 provider，隐藏宿主默认没有，因此隐藏测试不触碰系统剪贴板。嵌入方用 `Host::set_clipboard_provider` 注入实现；读写失败与 panic 都是静默无操作。
- Ctrl+C 从当前呈现快照读取聚焦输入的已提交值与归一化 selection，按 Unicode 标量写出选中字符；不派发 `on:input`。
- Ctrl+X 复用写入路径，只有 provider 返回成功后才用 `Backspace` 语义删除选区；成功时一次 `on:input`、caret 在原选区起点。失败或没有 provider 时文本、选区与 caret 不变。
- Ctrl+V 读取 `CF_UNICODETEXT`，过滤全部控制字符（含 NUL、CR/LF），再调用一次插入 reducer。有选区时替换，否则插入 caret；成功时一次 `on:input`，caret 在插入文本之后。空文本、非文本、读取失败与 panic 不改模型，也不删除选区。
- 组合优先于这些快捷键；AltGr（Ctrl+Alt）不触发复制、剪切或粘贴。复制/剪切/粘贴都不移动焦点。

## 应用复制到 Notepad

主窗口草稿得到 `ZenUI clipboard`（15 个 UTF-16 码元），Ctrl+A 后 UIA 选区为 `[ZenUI clipboard]`，Ctrl+C；Notepad 活动标签页清空后 Ctrl+V。

| 读数 | 100% | 150% |
| --- | --- | --- |
| 复制前草稿 | `ZenUI clipboard` | `ZenUI clipboard` |
| 复制前选区 | `selected=[ZenUI clipboard]` | `selected=[ZenUI clipboard]` |
| Notepad 文本 | `ZenUI clipboard`，15 码元 | 同左 |
| 草稿 `on:input` | 复制不新增 | 复制不新增 |

证据：[100%](证据/host-clipboard/100-notepad-from-zenui.png)、[150%](证据/host-clipboard/150-notepad-from-zenui.png)。Notepad 文档区显示同一文本，说明 `SetClipboardData` 写入的 `CF_UNICODETEXT` 被另一个进程读取。

## Notepad 复制到应用

Notepad 活动标签页放入 `Notepad to ZenUI 你好 🙂`（22 个 UTF-16 码元；来源是仓库外的系统剪贴板写入，不是 ZenUI），随后在 Notepad 中 Ctrl+A、Ctrl+C，把剪贴板所有权交给 Notepad。

| 读数 | 100% | 150% |
| --- | --- | --- |
| Notepad 文本 | `Notepad to ZenUI 你好 🙂`（`78,111,116,101,112,97,100,32,116,111,32,90,101,110,85,73,32,20320,22909,32,55357,56898`） | 同左 |
| 应用粘贴前草稿 | `ZenUI clipboard`，全选 | `ZenUI clipboard`，全选 |
| 应用粘贴后文本 | 22 码元，与 Notepad 一致 | 22 码元，与 Notepad 一致 |
| 粘贴后选区 | 折叠，caret 在末尾 | 折叠，caret 在末尾 |
| Ctrl+Z | — | 恢复 `ZenUI clipboard` 与全选选区 |

证据：[100% Notepad](证据/host-clipboard/100-notepad-external.png)、[100% 应用](证据/host-clipboard/100-zenui-from-notepad.png)、[150% Notepad](证据/host-clipboard/150-notepad-external.png)、[150% 应用](证据/host-clipboard/150-zenui-from-notepad.png)、[150% 撤销](证据/host-clipboard/150-zenui-paste-undo.png)。空格、CJK 与 emoji 的码元在往返中不变；150% 的一次 Ctrl+Z 恢复粘贴前的值与选区，说明一次粘贴只产生一条历史记录。

## 应用内剪切与主窗口到 Details

主窗口草稿放入 `cut me 你好 🙂`（150%）/ `Notepad to ZenUI 你好 🙂`（100%），Ctrl+A、Ctrl+X 后草稿为空；点击 Open details，在 `Details note` 粘贴。

| 读数 | 100% | 150% |
| --- | --- | --- |
| 剪切后主窗口 | `""`，caret 0 | `""`，caret 0 |
| Details note 粘贴后 | `Notepad to ZenUI 你好 🙂`，22 码元 | `cut me 你好 🙂`，12 码元 |
| 焦点 | Details note `HasKeyboardFocus=True` | 剪切后主窗口 `focus=True`；粘贴后 Details note `focus=True` |
| 另一窗口 | 主窗口保持 `""` | 主窗口保持 `""` |

证据：[100% 剪切后主窗口](证据/host-clipboard/100-zenui-cut-main.png)、[100% Details](证据/host-clipboard/100-zenui-cut-details.png)、[150% 剪切后主窗口](证据/host-clipboard/150-zenui-cut-main.png)、[150% Details](证据/host-clipboard/150-zenui-cut-details.png)。剪切写入与删除之间没有异步回合；可见结果与隐藏的写入失败测试共同证明失败不会删除文本。

## 空剪贴板与非文本剪贴板

主窗口草稿为 `abc`，Ctrl+A 全选后分别把剪贴板置空、置为 16×16 位图（不携带文本格式），再按 Ctrl+V。

| 场景 | 100% | 150% | 结果 |
| --- | --- | --- | --- |
| 空剪贴板 | text=[abc]，selected=[abc] | 同左 | 值、选区、caret 不变 |
| 非文本剪贴板 | text=[abc]，selected=[abc] | 同左 | 值、选区、caret 不变 |

证据：[100% 空](证据/host-clipboard/100-zenui-empty-clipboard.png)、[100% 非文本](证据/host-clipboard/100-zenui-nontext-clipboard.png)、[150% 空](证据/host-clipboard/150-zenui-empty-clipboard.png)、[150% 非文本](证据/host-clipboard/150-zenui-nontext-clipboard.png)。空剪贴板由 `OpenClipboard`/`EmptyClipboard` 清空；位图由 `Windows.Forms.Clipboard.SetImage` 写入。两者都没有 `CF_UNICODETEXT`，粘贴是静默无操作。

## 组合期间的 no-op

150% 下把系统剪贴板设为哨兵 `clipboard sentinel`，切到微软拼音并键入 `nihao`。UIA 已提交文本为空，窗口显示组合串 `nǐhao`；此时依次按 Ctrl+C、Ctrl+X、Ctrl+V。

| 读数 | 组合前 | 组合中按三个快捷键后 |
| --- | --- | --- |
| UIA 已提交文本 | `""` | `""` |
| 组合串 | `nǐhao`（下划线） | `nǐhao`（下划线） |
| 系统剪贴板 | `clipboard sentinel` | `clipboard sentinel` |
| 粘贴结果 | — | 无，未插入哨兵 |

证据：[组合前](证据/host-clipboard/150-zenui-composition-before.png)、[按键后](证据/host-clipboard/150-zenui-composition-after.png)。组合中快捷键归 IME；宿主不写剪贴板、不编辑，也不取消组合。100% 未重复该场景，组合状态机与 DPI 无关。

## 隐藏测试

- `crates/windows/src/clipboard.rs`：`single_line_text` 去掉全部控制字符并保留 CJK/emoji；非法 UTF-16 用替换字符解码。
- `crates/windows/src/host.rs` 新增 13 个宿主测试。复制/粘贴 9 个：精确复制选区且零 `on:input`；折叠/禁用/无焦点不写；代理对；caret 插入、一次 `on:input` 与一步撤销；替换选区、过滤控制字符、空/非文本/读取失败保留选区；组合 no-op；隐藏宿主无 provider 无 I/O；AltGr；跨窗口复制/粘贴。剪切 4 个：写入后删除、失败与缺少 provider 保留文本/选区/caret、折叠/禁用/组合 no-op、主窗口↔Details 焦点保持。
- 删除复用 `Runtime::backspace_node`，选择删除语义由 `tests/text_edit.rs` 的 `backspace_and_delete_forward_respect_caret_and_selection` 固定。
- 内存 provider 不读写系统剪贴板；原生 `OpenClipboard` 重试、`EmptyClipboard`/`GlobalAlloc`/`GlobalLock`/`SetClipboardData` 与每条路径的 `CloseClipboard` 由实现结构与本页可见互操作共同覆盖。193 个 `zenui-windows` 库测试与 workspace 825 个测试通过。

## 边界

- 单屏 2560×1600；100% 是临时切换后恢复 150%，不声明混合 DPI 或多显示器。
- 可见互操作只覆盖 Notepad；其它应用、富文本、HTML、图片粘贴与多格式剪贴板不在本切片。非文本场景只证明“没有 Unicode 文本时不改模型”，不证明图片可以粘贴。
- 可见复制读取的是 UIA 已提交文本与选区；系统剪贴板内容由 Notepad/应用粘贴结果或 `Get-Clipboard` 观察，没有读取剪贴板原始字节。
- 组合 no-op 只在 150% 采集；100% 的复制、剪切、粘贴、空/非文本与跨窗口场景已覆盖。
- Notepad 的其它恢复标签页未被修改；截图不含它们的标题或内容。采集结束时证据文本仍留在系统剪贴板，Notepad 中的临时标签页未保存。
- 隐藏/CI 是部分证据；本页没有逐项人工视觉复核，焦点读数来自 UIA。
