---
doc-id: SPEC-PLAT-WIN
layer: spec
status: normative
date: 2026-11-18
owner: Windows 平台工程师（10 年+）
sources:
  - archive/review/FINAL.md §7.2/§9
  - archive/review/VALIDATION.md
  - archive/research/runtime-and-rendering.md
  - archive/research/stack-decisions.md
  - archive/research/validation-and-roadmap.md
  - spec/06-render.md（责任边界表）
replaces: []
---

# SPEC-PLAT-WIN Windows 平台后端

> 本文件定义 Windows 后端**必须满足什么**，不声明已实现。事实状态只归 `docs/evidence/` 的 `CLAIM-*` / `EVID-*`；本文件出现的任何探针或源码路径都只是待归并线索，不是证据。
>
> 版本 pin 的唯一出处是 [SPEC-ROUTE](../01-route.md)。**本文件不写任何版本号**；受支持的 Windows 版本范围 MUST 由 `product/roadmap.md` 与 `SPEC-SCOPE-002` 单点定义，当前为 `OPEN`（`OPEN-PLAT-WIN-01`）。凡依赖 OS 版本的能力差异，一律登记为 `OPEN` 并给验证路径。
>
> 状态词只用 `SPEC / PROBE / VERIFIED / WITHDRAWN / OPEN`；**spec 层不使用 `VERIFIED`**。
>
> 本文件只拥有“Windows 平台事实如何被采集、归一化并交给抽象层”。以下事实**不在本文件重复定义**：`GeometrySnapshot` 的形状与发布顺序（`SPEC-RENDER-001/002`）、surface 与设备生命周期（`SPEC-RENDER-012/013`）、caret/selection 的坐标空间与公开单位（`SPEC-TEXT-015`）、IME composition 与 Enter 的消费顺序（`SPEC-TEXT-014`）、焦点结算（`SPEC-RT-026`）。出现重叠时以上述条目为准，本文件只写 Windows 侧采集方式与失败症状。

## 1. 后端职责与边界

Windows 后端是**采集与归一化层**，不是第二套 UI 权威。它 MUST 把 OS 事实转成 `zenui-platform` 的 typed 事件与能力描述，再交给 runtime / 布局 / 渲染 / 文本层；MUST NOT 自行计算几何、维护第二份焦点、缓存可见矩形或直接改 View 树。

| 职责 | 归谁（唯一拥有者） | 通过什么抽象 | 失败时的可见症状 |
|---|---|---|---|
| 窗口创建 / 销毁 | `zenui-platform`（Windows backend） | `WindowShell` trait；winit 的 `Window` / `WindowId` | 窗口无内容、首帧黑屏；关闭后仍被 surface 引用而 panic |
| 事件循环嵌入 | `zenui-platform` | winit `EventLoop` + `ControlFlow` + `EventLoopProxy` | 空闲时 CPU 100%；状态变化后不重绘；worker 唤醒丢失 |
| 输入与 IME | `zenui-platform` 采集 → `SPEC-TEXT` / 输入路由 | winit `WindowEvent`、`Ime` 事件、`set_ime_cursor_area`；自有 `InputRouter` | 中文丢字或双提交；候选窗错位；键盘落在错误控件 |
| DPI / 缩放 | `zenui-platform` | winit `ScaleFactorChanged`；每窗口 scale；DPI epoch | 125%/150% 下文字模糊；跨显示器后命中偏移 |
| 剪贴板 | `zenui-platform` | 独立 clipboard adapter，输入输出 typed `ClipboardData` | 粘贴为空或旧内容；剪贴板被占用导致卡死 |
| 拖放 | `zenui-platform` | winit `HoveredFile` / `DroppedFile` → typed `DropEvent` | 拖入文件无反应；拖出窗口丢失；自定义格式被静默忽略 |
| 原生菜单 / 托盘 | **可选模块，不是核心** | typed `Menu` / `Tray` trait；命令走同一 reducer | 菜单命令绕过事务；Explorer 重启后托盘图标消失 |
| 无障碍适配 | `zenui-platform` + AccessKit adapter | AccessKit tree/action → UIA provider | NVDA 不播报或读旧名；焦点顺序错乱；action 命中旧节点 |
| 打包与签名 | 发布流水线 + `zenui-platform` 配置 | MSIX / MSI / portable + Authenticode + WER | SmartScreen 拦截；崩溃无 dump；缺字体/ICU 资产 |

### SPEC-PLAT-WIN-001 平台层不是 UI 权威

- **状态**：`SPEC`
- **强度**：MUST NOT
- **要求**：Windows 后端 MUST NOT 保存布局矩形、z-order、clip chain、focus 目标或语义树节点的第二份副本；这些只能来自 runtime 发布并提交的 `GeometrySnapshot`（`SPEC-RENDER-001`）与 retained View 树。平台层只 MAY 保存 OS 句柄与“较上次已发布快照”的 epoch。
- **失败模式**：后端缓存了一份 `GetClientRect` 推导的矩形用于命中，窗口滚动后它与 `GeometrySnapshot` 不一致，点击命中到相邻控件；或关闭窗口时后端仍持有焦点目标，向已删除节点派发 action。
- **来源**：FINAL §7.2；`06-render.md` §0 责任边界表。

### SPEC-PLAT-WIN-002 单一事件泵，且只在主线程

- **状态**：`SPEC`
- **强度**：MUST / MUST NOT
- **要求**：整个进程 MUST 只有一个 UI 事件循环，由 `zenui-platform` 在主线程创建并运行；MUST NOT 额外调用 `GetMessage` / `PeekMessage` 自建第二个消息泵，MUST NOT 在 render/commit 阶段或事务进行中启动嵌套泵（`MessageBox`、`DoDragDrop`、私有 loop、阻塞式 `WaitForSingleObject`）。若 OS 交互必须使用 `DoDragDrop`，只能在事件回调最外层、事务之外发起，并当作一次外部 I/O。worker 线程 MUST 经 `EventLoopProxy` 唤醒 UI 线程（`SPEC-RT-037/050`）。
- **失败模式**：第二个消息泵抢走 `WM_*`，winit 收不到输入；或在事件回调里阻塞等待 GPU，消息泵停止，窗口显示“未响应”；worker 直接碰窗口句柄 → 随机崩溃。
- **来源**：FINAL §3/§7.2；`SPEC-RT-050`。

### SPEC-PLAT-WIN-003 窗口创建前完成 DPI 感知声明

- **状态**：`SPEC`
- **强度**：MUST / MUST NOT
- **要求**：进程 MUST 在创建任何 HWND 之前声明 per-monitor DPI 感知（应用清单优先；运行时 API 只能作为清单缺失时的兜底，且同样必须在首个 HWND 之前）。MUST NOT 在窗口已创建后再声明或切换感知模式；MUST NOT 依赖“系统会替我们缩放”的 DPI-unaware 路径。
- **失败模式**：窗口在 150% 显示器上被位图拉伸，文字发虚；或 `ScaleFactorChanged` 不触发，布局按错误 scale 计算；在混合 DPI 的多显示器间拖动时窗口尺寸跳变。
- **来源**：FINAL §7.2；`06-render.md` `SPEC-RENDER-012`。

### SPEC-PLAT-WIN-004 输入坐标必须按窗口 scale 归一化

- **状态**：`SPEC`
- **强度**：MUST / MUST NOT
- **要求**：后端 MUST 把 winit 交付的物理像素坐标，按**该窗口当前** scale factor 转成 logical 坐标，并标记该坐标所属的 `GeometrySnapshot` epoch；转换函数 MUST 与 `SPEC-STYLE-050` 的 logical px 定义一致。MUST NOT 在同一路径混用 physical 与 logical，MUST NOT 使用全局 scale。
- **失败模式**：125% 缩放下点击位置整体偏移约 1/5 控件宽度；多显示器各自 scale 不同时，副屏上的输入全部错位。
- **来源**：FINAL §7.2；`SPEC-STYLE-050`；`SPEC-RENDER-001`。

### SPEC-PLAT-WIN-005 键盘语义分离 physical / logical / text

- **状态**：`SPEC`
- **强度**：MUST / MUST NOT
- **要求**：后端 MUST 同时保留 physical key、logical key 与 text 输入三路信息：文本编辑类快捷键按 logical key 匹配，位置型快捷键与游戏式控件按 physical key 匹配，文本输入按 text 处理，三者 MUST 可区分；对 dead key（`text` 为空、后续才合成字符）MUST NOT 当作可输入字符，也 MUST NOT 丢弃后续合成事件。键盘布局在会话中切换时，MUST 重新解释 logical key 而不重启事件循环。
- **失败模式**：AZERTY/Dvorak 用户按 Ctrl+Z 触发错误命令；德语用户输入 `^` 后字母丢失；快捷键在布局切换后失效。
- **来源**：FINAL §4/§7.2；Windows `WM_KEYDOWN`/`WM_CHAR` 与 winit `KeyEvent` 语义。

### SPEC-PLAT-WIN-006 IME：采集 composition，提交只走文本 reducer

- **状态**：`SPEC`
- **强度**：MUST / MUST NOT
- **要求**：后端 MUST 将 OS IME 事件（IMM32 / TSF 路径由所接入 winit 版本决定）归一化为 composition start / update / commit / cancel 四类，并交给 `SPEC-TEXT-012/014` 的单一 reducer。preedit 光标范围 MUST 经 `SPEC-TEXT-011` 的 adapter 转换，MUST NOT 假定它与 UTF-16、grapheme 或 glyph 索引相同。候选窗定位 MUST 用最新 `GeometrySnapshot` 的 caret 矩形调用 `set_ime_cursor_area`。MUST NOT 把 preedit 文本当作已提交值写 Store；MUST NOT 在 composition 期间让 Enter 触发表单提交（`SPEC-TEXT-014`）。
- **失败模式**：中文输入“买牛奶”过程中每敲一个字母就提交一次，列表出现重复项；候选确认 Enter 同时新增一条任务；候选窗出现在窗口左上角而不是 caret 下方；切到别的窗口后 composition 残留，回来继续输入时插到错误字段。
- **来源**：FINAL §5/§7.2；`SPEC-TEXT-011/012/014`；`SPEC-GATE-A1-07`。

### SPEC-PLAT-WIN-007 DPI 变化是资源重建事件，不是一次 resize

- **状态**：`SPEC`
- **强度**：MUST / MUST NOT
- **要求**：收到 scale 变化时，后端 MUST 依次：更新该窗口 scale → 用 OS 建议矩形调整窗口 → 重配 surface（物理尺寸）→ 推进字体/图集/文本缓存 epoch（`SPEC-TEXT-004`、`SPEC-RENDER-004`）→ 请求重新布局与重绘。MUST NOT 假定 scale 为整数或只取 `1.0`/`2.0`；MUST NOT 复用按旧 scale 生成的字形位图或 atlas。
- **失败模式**：把窗口从 100% 屏拖到 150% 屏后文字仍是旧位图，发虚且行高与命中矩形不一致；在 125% 下 1px 边框消失或变成 2px。
- **来源**：FINAL §7.2；`SPEC-TEXT-004`；`SPEC-RENDER-004/012`。

### SPEC-PLAT-WIN-008 剪贴板访问必须有界、可失败

- **状态**：`SPEC`
- **强度**：MUST / MUST NOT
- **要求**：剪贴板读写 MUST 通过独立 adapter，失败 MUST 返回 typed `PlatformError`；MUST 使用有上限的重试/退避，MUST NOT 无限阻塞 UI 线程等待 `OpenClipboard`。文本 MUST 按 `CF_UNICODETEXT`（UTF-16LE）转换；自定义格式 MUST 显式声明支持或返回 `Unsupported`，MUST NOT 静默丢弃。变更通知 MAY 用 `GetClipboardSequenceNumber` 轮询；若使用格式监听，MUST 由附加的 message-only window 承担，MUST NOT 替换主窗口的 WndProc。
- **失败模式**：另一个进程长期占用剪贴板时应用卡死；复制中文后粘贴为乱码；粘贴菜单状态长期停留在旧内容；粘贴图片/HTML 时静默无反应却提示成功。
- **来源**：FINAL §7.2；Windows `OpenClipboard`/`GetClipboardData`/`WM_CLIPBOARDUPDATE` 语义。

### SPEC-PLAT-WIN-009 拖放只承诺已声明能力

- **状态**：`SPEC`
- **强度**：MUST / MUST NOT
- **要求**：核心 MUST 只承诺 **文件路径拖入**（`HoveredFile` / `DroppedFile`）为 typed `DropEvent`，并携带 window、坐标 epoch 与路径列表。MUST NOT 承诺任意 MIME/文本/图片 payload、drop 位置细节或从应用内拖出（OLE drag source）——这些 MUST 登记为 `OPEN` 并返回 `Unsupported`，不得静默接受。若需要 OLE 交互，MUST 只在事件回调内发起，MUST NOT 在 render/commit 阶段调用。
- **失败模式**：用户拖入一段文本或图片，界面高亮表示可放置但松手后无数据，且没有任何错误提示；从列表拖出任务到资源管理器时应用卡住。
- **来源**：FINAL §7.2；winit 文件拖放事件；Windows OLE drag-drop 模型。

### SPEC-PLAT-WIN-010 原生菜单 / 托盘是可选模块，不是核心

- **状态**：`SPEC`
- **强度**：MUST NOT
- **要求**：核心框架 MUST NOT 依赖原生菜单栏或系统托盘；若提供，MUST 是独立可选模块，命令 MUST 走与键盘/指针相同的 command/reducer 路径（`SPEC-RT-050`、`SPEC-MACRO-005`），MUST NOT 直接改 View 树。托盘实现 MUST 处理 Explorer 重启后的重新注册；MUST NOT 依赖未文档化的暗色菜单栏行为。
- **失败模式**：菜单项绕过事务直接改状态，出现第二条刷新路径；Explorer 崩溃重启后托盘图标永久消失；暗色主题下菜单栏仍是亮色且无降级说明。
- **来源**：FINAL §7.2；Windows `Shell_NotifyIcon` / `WM_TASKBARCREATED` / HMENU 语义。

### SPEC-PLAT-WIN-011 无障碍适配：UIA 由 AccessKit 承载，几何与 action 同源

- **状态**：`SPEC`
- **强度**：MUST / MUST NOT
- **要求**：Windows 无障碍 MUST 经 AccessKit 的 winit 适配层暴露 UIA（旧 MSAA/IAccessible 只作为客户端桥接，不是我们的主路径）；语义节点 MUST 由 retained View 树生成，矩形与 action 目标 MUST 来自同一 `GeometrySnapshot`（`SPEC-RENDER-017`）。每个 action MUST 先校验 `ViewId + generation` 仍存活且可见。后端 MUST NOT 在无 UIA provider 的情况下假定“写了 Name 就会被读屏读出”；真实播报只在 C Gate 成立（`SPEC-RENDER-018`）。
- **失败模式**：NVDA 焦点移动后仍读旧名称；在筛选后读出已删除行；`Tab` 焦点顺序与语义树顺序不一致；窗口激活了但没有 UIA provider，读屏完全静默。
- **来源**：FINAL §7.2/§9 C；`SPEC-RENDER-017/018`；`SPEC-GATE-C-02`。

### SPEC-PLAT-WIN-012 打包、签名与崩溃上报是一等交付物

- **状态**：`SPEC`
- **强度**：MUST
- **要求**：发布物 MUST 声明其分发形态（MSIX / MSI / portable）并给出对应清单、架构与依赖；MUST 使用 Authenticode 签名，签名失败 MUST 阻断发布。崩溃上报 MUST 产出可符号化的 minidump（含模块列表与 commit），并把 Rust panic 与原生异常分开记录。字体、ICU 词典与纹理资产 MUST 随包存在（`SPEC-TEXT-008`），MUST NOT 依赖目标机已安装的系统字体来满足 golden。
- **失败模式**：用户首次运行被 SmartScreen 拦截；崩溃只留下“程序已停止工作”而无 dump；中文分词在干净机器上失效；MSIX 安装因架构或依赖缺失失败。
- **来源**：FINAL §9 C；`SPEC-GATE-C-04/05`；`SPEC-TEXT-008`。

### SPEC-PLAT-WIN-013 窗口关闭与技术资源释放顺序

- **状态**：`SPEC`
- **强度**：MUST / MUST NOT
- **要求**：关闭 MUST 先停止该窗口的输入/IME 与重绘请求，再销毁 surface，最后释放与窗口绑定的 View 子树与 owner；MUST NOT 在 surface 已销毁后仍提交绘制，MUST NOT 在 owner 释放后接受迟到事件。多窗口时，关闭一个窗口 MUST NOT 影响其他窗口的 device 与 scale。
- **失败模式**：关闭窗口时偶发 `use-after-free`/panic；关闭一个窗口后另一个窗口变黑；IME composition 在关闭瞬间提交到已销毁 editor。
- **来源**：FINAL §7.2；`SPEC-RENDER-012`；`SPEC-RT-021`。

### SPEC-PLAT-WIN-014 平台错误映射为 typed Result，不用 panic 探测

- **状态**：`SPEC`
- **强度**：MUST NOT
- **要求**：后端所有可预期失败（无可用 adapter、surface 不支持、剪贴板被占、权限不足、无 UIA、拖放格式不支持）MUST 映射为 `Result::Unsupported` / typed `PlatformError`，并进入统一 sink（`SPEC-RT-057`）。MUST NOT 用 `unwrap`、`expect`、wgpu validation panic 或 try-panic 作为能力检测（`SPEC-ROUTE-013`、`SPEC-RENDER-008`）。
- **失败模式**：虚拟机/远程桌面无 GPU 时应用直接崩溃而不是显式降级；不支持的能力在用户操作时才 panic。
- **来源**：`SPEC-ROUTE-013`；`SPEC-RENDER-008`；FINAL §7.2。

### SPEC-PLAT-WIN-015 系统偏好（高对比度 / 减少动态 / 主题 / 文本缩放）必须可观测

- **状态**：`SPEC`
- **强度**：MUST / MUST NOT
- **要求**：后端 MUST 把高对比度、减少动态、应用主题（浅色/深色）与系统文本缩放等偏好暴露为带 epoch 的平台事件或能力查询；MUST NOT 把它们实现为一次性启动读取后永不更新。是否把“文本缩放”并入布局 rem、以及高对比度下如何重映射 token，MUST 由 `SPEC-STYLE` 的统一规则决定，后端 MUST NOT 自创一套颜色替换。
- **失败模式**：用户在系统里开启高对比度后界面文字与背景同色；运行中切换深浅色主题无反应；用户把“放大文本”调到 150% 后应用文字纹丝不动。
- **来源**：FINAL §6/§7.2；`SPEC-STYLE-024`；Windows `SPI_GETHIGHCONTRAST` / `SPI_GETCLIENTAREAANIMATION` / `UISettings` 语义。

### SPEC-PLAT-WIN-016 多窗口与模态共享一个事件循环

- **状态**：`SPEC`
- **强度**：MUST / MUST NOT
- **要求**：每个顶层窗口 MUST 拥有独立 surface 与独立 scale（`SPEC-RENDER-012`），但 MUST 共享同一个 UI 事件循环与同一 reducer；`Device/Queue` 的共享边界以 `SPEC-RENDER` 为准。模态 MUST 实现为“禁用父窗口 + 同一循环内的子窗口”，MUST NOT 启动第二个事件循环；MUST NOT 依赖 `IsDialogMessage` 与自有 Tab 处理同时生效。
- **失败模式**：模态对话框出现后主窗口仍可点击；打开第二个窗口时输入事件被第一个窗口吞掉；两个窗口共用 scale 导致副屏错位。
- **来源**：FINAL §7.2；`SPEC-RENDER-012`；Windows owner/`EnableWindow` 语义。

## 2. 平台差异矩阵

下表的每一行都是一个**可能改变框架语义**的 Windows 差异。处理列只有两种合法结论：**必须在抽象层吸收**（上层代码看不到差异，看到的是同一 typed 语义），或**允许暴露**（登记为能力/偏好的 typed 描述，业务层只能读取能力，不能按 OS 分支）。

| 差异 | 对框架语义的影响 | 处理 | 关联 |
|---|---|---|---|
| IME composition 事件时机：preedit/commit 分段投递，提交可能与按键同批到达 | 把 preedit 当已提交值 → 重复字符、双提交 | **必须在抽象层吸收**：统一为 composition 状态机，提交只经单一 reducer | 006 |
| 候选窗由 IME/OS 拥有，应用只能提供 caret 区域 | 候选列表内容与逐项高亮不可控 | **允许暴露**：只承诺定位提示，不承诺候选窗内容 | 006 |
| 呈现模型：DWM 合成 + flip-model，最小化/被遮挡/零尺寸时 Present 可能非致命失败 | 把“没画出来”当成功 → 状态推进到坏帧 | **必须在抽象层吸收**：零尺寸跳过绘制，非致命状态保留 damage 重试 | 002/007 |
| 设备丢失：驱动更新、TDR、挂起恢复产生 device removed/reset | 旧 atlas/纹理被新 device 引用 → validation error 或黑屏 | **必须在抽象层吸收**：清理→重建→发布（`SPEC-RENDER-013`） | 013 |
| 字体枚举与 fallback 随系统语言包、版本与安装字体变化 | golden 不可复现；CJK/emoji 缺字 | **必须在抽象层吸收**：golden 捆绑固定字体；系统字体只作能力信息，不参与 golden（`SPEC-TEXT-010`） | 012 |
| 分数 DPI：125%/150%/175%/225%，且每显示器可不同 | 位图/命中/字体 epoch 不一致 | **必须在抽象层吸收**：每窗口 scale + 文档化取整策略 | 004/007 |
| 系统文本缩放（辅助功能“放大文本”）独立于 DPI | 忽略用户偏好，文字过小 | **允许暴露**为偏好 epoch；是否并入 layout rem 待裁决（`OPEN-PLAT-WIN-10`） | 015 |
| 无障碍 API 是 UIA；MSAA 仅旧客户端桥接 | 部分 role/action 能力与 macOS/Linux 不同 | **必须在抽象层吸收**：AccessKit 映射；**允许暴露**能力清单 | 011 |
| 高对比度 / 减少动态 / 深浅主题是系统级偏好 | 强制配色与动画策略不同 | **必须在抽象层吸收**为偏好 epoch；颜色重映射规则归 `SPEC-STYLE` | 015 |
| 剪贴板是全局互斥资源，且可能有历史/云同步 | 读写可能失败或延迟 | **必须在抽象层吸收**：有界重试 + typed 失败；格式能力允许暴露 | 008 |
| 拖放：稳定支持文件路径，自定义 payload 与拖出能力有限 | 业务以为任意数据可拖放 | **允许暴露**：只承诺文件拖入，其余 `Unsupported` | 009 |
| 键盘布局可运行时切换，dead key 与组合输入分段到达 | 快捷键与文本输入语义混淆 | **必须在抽象层吸收**：physical/logical/text 三路分离 | 005 |
| 混合显卡 / 多 adapter：呈现必须选与 surface 兼容的 adapter | 选错 GPU → 黑屏或跨 adapter 失败 | **必须在抽象层吸收**：始终带 compatible surface 选择并验证 | 014 |
| 窗口激活受前台锁限制：后台进程不能随意抢焦点 | 模态/弹窗可能只在任务栏闪烁 | **必须在抽象层吸收**为焦点事件；**允许暴露**“需要用户手势”的能力 | 016 |
| 圆角 / Mica / 透明 / 自定义标题栏依赖未稳定文档化的 DWM 行为 | 承诺后跨 OS 版本失效 | **允许暴露**为能力；核心不承诺 | 013 |

### SPEC-PLAT-WIN-030 差异必须显式登记，不得“碰巧不同”

- **状态**：`SPEC`
- **强度**：MUST / MUST NOT
- **要求**：任何会改变布局、绘制、命中、文本坐标或语义树的平台差异，MUST 在实现前登记到本矩阵，并明确“吸收”或“暴露”；MUST NOT 让同一份上层代码在 Windows 上悄悄走另一条分支。新增差异时 MUST 同时给出可观测症状与验证手段。
- **失败模式**：某个控件在 Windows 上命中区域比 macOS 大 1px，代码里没有任何登记，回归测试只在 macOS 跑，问题直到用户报告才暴露。
- **来源**：本文件 §2；`SPEC-SCOPE-021`。

### SPEC-PLAT-WIN-031 吸收判据：影响语义的必须吸收

- **状态**：`SPEC`
- **强度**：MUST / MUST NOT
- **要求**：若差异会改变 `GeometrySnapshot`、文本坐标、焦点/选区语义或 AT 可观察结果，MUST 在抽象层吸收；只有纯粹保真度、外观或可选能力差异 MAY 暴露。暴露项 MUST 以 typed 能力/偏好出现，MUST NOT 以 `cfg!(windows)` 散落在业务层。
- **失败模式**：业务组件里出现 `#[cfg(target_os = "windows")]` 修正坐标，跨平台行为分叉且无法测试。
- **来源**：`06-render.md` §0；`SPEC-SCOPE-002`。

## 3. 失败模式

每条失败模式都必须落到一个**可观测症状**，并能被 Gate C 的场景或日志捕获。只写“可能出错”不算。

| 失败模式 | 触发条件 | 可观测症状 | 必须的检测 / 恢复 | 关联 |
|---|---|---|---|---|
| device loss | 驱动更新、TDR、远程会话切换、挂起恢复 | 黑屏、花屏，或日志出现 device removed/reset | 捕获设备丢失回调；清理 atlas/binding/scene；重建成功才发布 | 013 |
| resize 竞态 | 拖拽改变窗口大小、最小化、DPI 切换同时发生 | 拉黑一帧、内容拉伸、validation error | 尺寸为 0 时跳过绘制；surface 重配串行化；失败保留旧帧 | 002/007 |
| 活动 resize 卡顿 | 在 `WM_ENTERSIZEMOVE`..`WM_EXITSIZEMOVE` 期间每帧做重活 | 窗口跟手迟滞、标题栏“未响应” | 活动 resize 期间限流或延迟全量重建；结束后补一帧 | 002 |
| IME 事件丢失 / 重复提交 | 焦点切换、窗口停用、composition 中关闭窗口 | 丢字、重复字符、候选确认后多一条任务 | focus 丢失时显式 commit 或 cancel；提交带 editor generation | 006 |
| 焦点丢失 | 弹窗、Alt-Tab、点击非客户区、前台锁 | 键盘输入无响应，或落到错误控件 | 焦点事件与 `GeometrySnapshot` 同 epoch；前台锁失败给可见提示 | 016 |
| AT 不播报 | UIA provider 未激活、focus 事件缺失、语义树 stale | NVDA 静默或读旧名称 | action 前校验 generation；语义树随提交更新；C Gate 实测 | 011 |
| DPI 跳变 | 跨显示器拖动、显示器热插拔、远程会话缩放 | 命中偏移、文字模糊、caret 错位 | 每窗口 scale epoch；scale 变化重建字体/图集/surface | 004/007 |
| 被遮挡或最小化时 Present 失败 | 窗口被完全遮挡、最小化 | 误判为成功，damage 丢失；恢复后停在旧内容 | 识别非致命状态；跳过或保留 damage；恢复时重绘 | 002 |
| 剪贴板占用 | 另一进程持有剪贴板 | 复制/粘贴卡住或返回空 | 有界重试 + typed 失败；不阻塞 UI 线程 | 008 |
| 拖放被忙循环饿死 | 拖放过程中 UI 线程做长任务 | 松手后无 `DropEvent`，或拖影停留 | 事件处理不阻塞；长任务 postcommit/worker | 009 |
| 事件回调重入 | 在事件处理里开模态、`DoDragDrop`、同步等待 | 递归 panic、消息乱序、状态被改两次 | 命令排队到下一事务；禁止在 render/commit 内起嵌套泵 | 002 |
| 跨线程碰窗口 | worker 直接调用窗口/树 API | 随机崩溃或死锁 | 只允许 `EventLoopProxy` 投递；UI 线程独占（`SPEC-RT-050`） | 002 |
| 混合 GPU 黑窗 | 选到与 surface 不兼容的 adapter | 窗口纯黑但进程无错误 | 选择时带 compatible surface；失败返回 typed error | 014 |
| 高对比度不可读 | 用户开启高对比度，token 未重映射 | 文字与背景同色、边框消失 | 订阅偏好 epoch；走 `SPEC-STYLE` 统一重映射 | 015 |
| 布局切换导致快捷键失效 | 用户切换 AZERTY/Dvorak | 同一物理键触发不同命令或无效 | physical/logical 分离；快捷键用 physical，文本用 logical | 005 |
| 托盘图标消失 | Explorer 重启 | 托盘区域图标永久不见 | 监听 `TaskbarCreated` 并重新注册 | 010 |
| 自定义 WndProc 覆盖 winit | 为剪贴板/UIA 安装子类化窗口过程 | 输入事件全部消失，或 UIA 正常但键盘失效 | 只链式 `CallWindowProc`，不得替换 winit 的 WndProc | 002/008 |

## 4. Gate C 最小证据清单

以下每条都必须在**真实 Windows 主机**上执行，产物写入 `artifacts/desktop/windows-<display>-<adapter>/`，包含 OS、显示协议/会话类型、adapter、driver、commit 与 lock hash。**在对应 `EVID-VAL-*` 出现前，状态一律为 `OPEN`；headless 探针、截图或“本机能跑”都不构成证据。**

| ID | 最小证据（必须真实执行） | 关联 | 状态 |
|---|---|---|---|
| `SPEC-PLAT-WIN-C01` | 真实 winit 窗口由 `zenui::run(Application)` 启动；记录窗口创建/关闭、`ControlFlow::Wait`、首帧与一次 resize 的 trace | `SPEC-GATE-A1-01`、`SPEC-GATE-C-01` | `OPEN` |
| `SPEC-PLAT-WIN-C02` | 指针与键盘：click、wheel、Tab/Shift+Tab、Space、clip 外不可 hit；trace 含 logical 坐标与 epoch | `SPEC-GATE-A1-06` | `OPEN` |
| `SPEC-PLAT-WIN-C03` | 中文 IME：preedit→commit、候选 Enter 只确认、下一次 Enter 只提交一次；计数 `ime_consumed/input_commit/submit/insert/clear` 各符合预期 | `SPEC-GATE-A1-07`、`SPEC-TEXT-014` | `OPEN` |
| `SPEC-PLAT-WIN-C04` | NVDA 实机：list/listitem、checkbox 状态、filter pressed、删除项名称、两个空态、焦点转移与公告；附人/机可读记录 | `SPEC-GATE-C-02`、`SPEC-RENDER-018` | `OPEN` |
| `SPEC-PLAT-WIN-C05` | DPI：100%/125%/150%/200% 与跨显示器移动；每档的字体、命中、caret、语义矩形一致；缩窗与最小化后恢复正确 | `SPEC-GATE-C-01`、`SPEC-RENDER-012` | `OPEN` |
| `SPEC-PLAT-WIN-C06` | GPU：DX12 为主，可用时补 Vulkan；同一 Todo 场景与 CPU reference 做像素对比；至少构造一条 `Unsupported` 路径且不 panic | `SPEC-GATE-C-03`、`SPEC-RENDER-008` | `OPEN` |
| `SPEC-PLAT-WIN-C07` | 剪贴板与文件拖放：复制/粘贴中文、格式不支持时的 typed 失败、拖入文件产生 `DropEvent`、拖出/自定义格式返回 `Unsupported` | `SPEC-GATE-C-01`、`SPEC-PLAT-WIN-008/009` | `OPEN` |
| `SPEC-PLAT-WIN-C08` | 空闲：真实窗口 `ControlFlow::Wait` 下 30–60s 无每帧重绘；附 ETW/WPA 或等价采样 | `SPEC-GATE-C-06` | `OPEN` |
| `SPEC-PLAT-WIN-C09` | 设备丢失恢复：构造 device removed/reset，验证清理→重建→发布，旧 device 资源不再被引用 | `SPEC-RENDER-013`、`SPEC-GATE-C-03` | `OPEN` |
| `SPEC-PLAT-WIN-C10` | 打包产物：MSIX/MSI/portable 之一可干净安装、升级、卸载；签名有效；字体/ICU/licence/lock hash 齐备 | `SPEC-GATE-C-04/05` | `OPEN` |
| `SPEC-PLAT-WIN-C11` | 系统偏好：高对比度、减少动态、深浅主题、文本缩放各切换一次，界面按 `SPEC-STYLE` 规则响应 | `SPEC-PLAT-WIN-015`、`SPEC-STYLE-024` | `OPEN` |
| `SPEC-PLAT-WIN-C12` | 崩溃上报：Rust panic 与原生异常各构造一次，产出可符号化 minidump 与模块/commit 信息 | `SPEC-PLAT-WIN-012` | `OPEN` |

**收敛规则**：C01–C12 的任意一条，只有在 evidence 层留下含主机、命令、commit 与产物的 `EVID-VAL-*` 后才能从 `OPEN` 改为 `VERIFIED`；跑在 macOS/Linux 或另一台 Windows 上的旧结果 MUST NOT 复用（`SPEC-GATE-C` 反作弊规则）。

## 5. 不确定项

以下事实我**不敢在 spec 层确认**。每条给验证路径；在验证完成前，实现 MUST 按最保守假设处理并返回 typed `Unsupported`，而不是假装可用。

| ID | 不敢确认的事实 | 为什么重要 | 验证路径 | 状态 |
|---|---|---|---|---|
| `OPEN-PLAT-WIN-01` | 受支持的最低/最高 Windows 版本范围未定 | 决定 manifest、DWM、UIA、输入 API 的可用下限 | `product/roadmap.md` 定版；在 CI 增加对应 runner | `OPEN` |
| `OPEN-PLAT-WIN-02` | 所接入 winit 的 preedit 光标范围精确单位（byte / UTF-16 / char）与空范围语义 | 决定 `SPEC-TEXT-011` adapter 的换算，错了就 caret 偏移 | 读该 pin 的源码 + 中文/日文 IME 实测 trace | `OPEN` |
| `OPEN-PLAT-WIN-03` | winit 在窗口失焦/停用时是否保证发送 composition cancel 或 commit | 不保证就会出现残留 composition 与错位提交 | 构造失焦、Alt-Tab、弹窗三种场景并记录事件序列 | `OPEN` |
| `OPEN-PLAT-WIN-04` | AccessKit 的 winit 适配层在该 pin 下如何激活 UIA provider，是否与 winit 自身 WndProc 共存 | 决定 NVDA 能否看到任何节点 | 最小窗口 + Inspect/NVDA 读取语义树；记录 `WM_GETOBJECT` 处理 | `OPEN` |
| `OPEN-PLAT-WIN-05` | IMM32 路径与现代 TSF IME（如系统拼音/日文 IME）的行为差异是否影响 preedit/候选窗 | 可能只有部分 IME 正常 | 对至少两种 IME 各跑同一 trace，记录差异 | `OPEN` |
| `OPEN-PLAT-WIN-06` | winit 是否在按钮按下期间保持指针捕获，拖拽出窗口是否继续收到 move/up | 滑块/选择框拖出窗口会“粘住”或丢失释放 | 真机拖出窗口再松手，记录事件序列 | `OPEN` |
| `OPEN-PLAT-WIN-07` | 剪贴板变更监听是否需要附加 message-only window；后台/无焦点时访问是否受限 | 决定粘贴菜单与快捷键状态是否正确 | 用 `GetClipboardSequenceNumber` 与监听两种方案各做一次对照 | `OPEN` |
| `OPEN-PLAT-WIN-08` | OLE 拖放的线程初始化（`OleInitialize`）是否已由 winit 完成；未完成时可否安全补做 | 补错会破坏 winit 的消息处理 | 读 winit 源码 + 最小 drag/drop 探针 | `OPEN` |
| `OPEN-PLAT-WIN-09` | flip-model + wgpu surface 下透明、Mica、圆角、自定义标题栏的可行组合 | 承诺后跨版本易碎 | 逐项做最小窗口探针，记录失败为 `Unsupported` | `OPEN` |
| `OPEN-PLAT-WIN-10` | 系统“放大文本”是否、以及如何并入 layout rem | 影响所有尺寸与 golden | 读系统设置 API + 与 `SPEC-STYLE` 负责人裁决 | `OPEN` |
| `OPEN-PLAT-WIN-11` | 高对比度下 Tailwind token 调色板的重映射规则 | 不做会导致不可读 | 与 `SPEC-STYLE` 联合定规则并出 fixture | `OPEN` |
| `OPEN-PLAT-WIN-12` | 混合显卡/多 adapter 下 `compatible_surface` 是否足以避免黑窗 | 决定首帧是否可靠 | 在 Optimus/APU+dGPU 机器上各跑一次 | `OPEN` |
| `OPEN-PLAT-WIN-13` | Rust panic 的 minidump 捕获方式（panic 不是 SEH 异常） | 决定崩溃是否可诊断 | 用 panic hook + `MiniDumpWriteDump` 做最小探针；对照 WER | `OPEN` |
| `OPEN-PLAT-WIN-14` | MSIX/AppContainer 对 IME、剪贴板、文件访问与字体枚举的影响 | 打包形态可能改变运行时行为 | 同一 Todo 分别以 MSIX 与 portable 运行并对比 | `OPEN` |
| `OPEN-PLAT-WIN-15` | Windows on ARM64 的 GPU/IME/AT 行为是否与 x64 等价 | 平台范围若含 ARM64，不能用 x64 结果背书 | 申请 ARM64 runner 跑 C01–C04 | `OPEN` |
| `OPEN-PLAT-WIN-16` | 多窗口模态下前台锁的实际表现与用户手势要求 | 决定弹窗是否只在任务栏闪烁 | 从后台进程弹模态并记录 `SetForegroundWindow` 结果 | `OPEN` |
