---
doc-id: SPEC-PLAT-MACOS
layer: spec
status: draft
date: 2026-11-18
owner: macOS 平台工程师（10 年+）
sources:
  - archive/review/FINAL.md
  - archive/review/VALIDATION.md
  - archive/research/runtime-and-rendering.md
replaces: []
---

# SPEC-PLAT-MACOS macOS 平台后端

> 本文件只定义 macOS 后端**要满足什么**，不声明任何能力已实现或已验证。仓库当前没有 winit / AccessKit / surface 代码（`CLAIM-009`），因此本文所有条目都只能是 `SPEC` 或 `OPEN`；真实窗口、IME、VoiceOver 与打包证据全部欠账（`CLAIM-075`、`CLAIM-079`、`CLAIM-080`、`CLAIM-081`）。
>
> 版本 pin 的唯一出处是 [spec/01-route.md](../01-route.md)（`SPEC-ROUTE`）。本文不写任何依赖版本号，也不写最低支持系统版本（后者仍是未决项，见 §5）。
>
> 阅读前先看 [spec/06-render.md](../06-render.md) §0 的责任边界表：平台层拥有 surface 生命周期与事件翻译，**不读 paint record、不决定绘制内容、不决定焦点与 composition 语义**。

## 1. 后端职责与边界

平台层（`zenui-platform` 的 macOS 后端）只有一个职责：**翻译**。把 AppKit 世界翻译成 zenui 自己的类型化事件与生命周期，把 zenui 的请求翻译成 AppKit / Metal 调用。它 MUST NOT 成为第二个决策中心。

| 职责 | 真相归属 | 通过什么抽象 | 失败时的可见症状 |
|---|---|---|---|
| 窗口创建/销毁 | 平台层（`NSWindow` + `NSView` + `CAMetalLayer`）；surface 生命周期按 `SPEC-RENDER-012` 归 WindowShell | `WindowShell` 的 `create/destroy`；窗口句柄对上层不透明 | 关窗崩溃、surface 泄漏、第二次开窗黑屏 |
| 事件循环 | 平台层拥有 `NSApplication` run loop | 自有 `PlatformEvent` 流；不向上暴露 winit/AppKit 类型 | 嵌套 run loop 导致事务重入、输入重复 |
| 输入 | NSEvent → 平台层；焦点/命中/手势语义 → runtime | `InputEvent`（逻辑坐标 + modifiers + phase） | 点击命中偏移、快捷键失效、滚动手感异常 |
| IME | NSTextInputClient 实现 → 平台层；composition 语义 → `SPEC-TEXT-014` | `ImeEvent::{Preedit, Commit, Cancel, Geometry}` | 候选窗错位、选词回车误提交、组合串丢失 |
| DPI/缩放 | `NSWindow`/`NSScreen` → 平台层；布局与资源重建 → runtime/render | `ScaleChanged { window, scale, physical_size }` | 跨屏后文字模糊、命中偏移、atlas 错配 |
| 剪贴板 | `NSPasteboard` → 平台层；读写时机 → `SPEC-RT-056` | 异步 `ClipboardResult { generation, kind }` | 主线程卡顿、粘贴覆盖新编辑内容 |
| 拖放 | `NSDraggingDestination` → 平台层；drop 目标解析 → runtime | `DragEntered/Updated/Exited/Dropped` 状态机 | 高亮不消失、重复 drop、沙盒拒绝却显示成功 |
| 原生菜单/托盘 | `NSMenu`/`NSStatusItem` → 平台层；命令执行 → runtime | typed `Command` 路由到焦点控件 | Cmd+C/V/Z 无效、一次快捷键执行两次 |
| 无障碍适配 | AccessKit 树 → runtime/render（`SPEC-RENDER-017`）；`NSAccessibility` 映射 → 平台层 | `AccessibilityAdapter`（只做映射与坐标转换） | VoiceOver 只读窗口、焦点不跟随、矩形错位 |
| 打包与签名 | CI + 平台层；判据归 `SPEC-GATE-C-05` | `.app` bundle + codesign + notarize + staple | Gatekeeper 拦截、离线机器无法启动 |

**三条不可越界**：

1. 平台层 MUST NOT 解析 class/样式、决定布局尺寸、决定断行。
2. 平台层 MUST NOT 维护焦点真相；它只报告“AppKit 认为谁是 first responder”，焦点策略由 `SPEC-RT-026` 与 runtime 决定。
3. 平台层 MUST NOT 持有 composition 的权威副本；marked text 的真相在 `SPEC-TEXT-012` 的 reducer 里，平台层只转发。

### SPEC-PLAT-MACOS-001 AppKit 对象只在 UI 主线程创建、修改、销毁

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：`NSApplication`、`NSWindow`、`NSView`、`CAMetalLayer` 及其 delegate MUST 只在 UI 主线程上创建、修改、销毁；MUST NOT 从 worker 线程触碰 AppKit（包括 KVO 注册、`performSelector`、属性读写）。窗口销毁顺序 MUST 是：停止提交 → 等待在途帧完成 → 释放 drawable/layer → 释放 window。
- **失败模式**：启动期 `NSInternalInconsistencyException`/SIGTRAP；关窗后渲染路径仍持有 layer，表现为 use-after-free 崩溃或第二次开窗黑屏。
- **来源**：AppKit 主线程模型；`CLAIM-009`；`SPEC-RENDER-012`。

### SPEC-PLAT-MACOS-002 事件循环只有一个所有者，嵌套 run loop 必须显式建模

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：`NSApplication` 的 run loop MUST 由平台层拥有并驱动。任何嵌套 run loop——`NSOpenPanel`/`NSSavePanel`、sheet、菜单跟踪、窗口拖拽会话——MUST 以 `ReentrancyDepth` 暴露给 runtime。`depth > 1` 时 runtime MUST 只标脏、MUST NOT 开启新的帧事务；回到 `depth == 1` 后合并成一次提交。
- **失败模式**：打开文件面板期间 Effect 触发提交，面板关闭时看到两次 publish 或输入内容重复；或在 sheet 内点击导致死锁。
- **来源**：`SPEC-RT-050`（单主线程与固定阶段顺序）；FINAL §3。

### SPEC-PLAT-MACOS-003 坐标转换只有一个入口

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：AppKit 屏幕坐标（原点左下、points）、窗口逻辑坐标（原点左上）、物理像素三者之间的换算 MUST 由平台层单一 adapter 完成；窗口的 backing scale MUST 随窗口记录。MUST NOT 在文本、渲染、无障碍模块里各自实现一套换算。所有对外几何（命中、paint、caret、AccessKit rect、IME 候选矩形）MUST 来自同一份 `GeometrySnapshot`（`SPEC-RENDER-001`）。
- **失败模式**：IME 候选窗出现在主屏左上角或副屏可视区之外；VoiceOver 读出的控件矩形与可见位置上下颠倒；副屏上点击命中偏移一个屏幕高度。
- **来源**：`SPEC-TEXT-011`（坐标集分离）；`SPEC-RENDER-012`。

### SPEC-PLAT-MACOS-004 IME：平台只转发，不消费

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：平台层 MUST 承载 `NSTextInputClient` 语义（`setMarkedText`、`insertText`、`hasMarkedText`、`markedRange`、`selectedRange`、`firstRect(forCharacterRange:)`），并按原序转发为 `ImeEvent`。MUST NOT 在平台层把 Return/Enter 解释为表单提交；MUST NOT 在 composition 期间用会取消组合串的 `set_string`/`set_text` 类接口重置文本。候选窗矩形 MUST 来自当前 caret 几何，而不是固定的窗口原点。
- **失败模式**：中文选词回车同时提交表单（违反 `SPEC-TEXT-014`）；候选窗固定在屏幕左上；外部更新打断 composition，已输入内容丢失。
- **来源**：`SPEC-TEXT-014`；`CLAIM-079`；FINAL §4/§5。

### SPEC-PLAT-MACOS-005 DPI/缩放按窗口与显示器记录，变化必须触发重排

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：每个窗口 MUST 从 `windowDidChangeBackingProperties` / `ScaleFactorChanged` 得到自己的 scale，MUST NOT 维护全局 scale。scale 变化 MUST 原子地同时触发重排、字形 atlas 重建、paint page 失效与 drawable 重配，MUST NOT 只重绘旧布局。scale MUST 以浮点数承载：macOS 实践中 backing scale 常见为 1x/2x，但“更多空间”等缩放模式改变的是逻辑分辨率，框架 MUST NOT 把 scale 写死成整数枚举。
- **失败模式**：拖到外接显示器后文字永久模糊，或一帧内被拉伸；跨屏拖动时点击命中偏移；旧 scale 的 atlas 被复用出现花屏。
- **来源**：`SPEC-RENDER-012`；`CLAIM-075`；`CLAIM-081`。

### SPEC-PLAT-MACOS-006 剪贴板只在 postcommit 读写，结果带 generation

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：`NSPasteboard` 的读写 MUST 发生在事务提交后（`SPEC-RT-056`），MUST NOT 在布局/绘制回调中同步读大对象。读取 MUST 异步并带回 `(generation, kind)`；过期结果 MUST 丢弃。写入 MUST 声明类型（文本/富文本/文件 URL），MUST NOT 用私有类型名冒充标准类型。
- **失败模式**：复制大量内容时主线程 beachball；粘贴结果覆盖用户已继续输入的新内容；与其他应用互粘时格式丢失。
- **来源**：`SPEC-RT-056`；`CLAIM-081`。

### SPEC-PLAT-MACOS-007 拖放必须闭合会话

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：拖放 MUST 实现完整状态机：注册可接受类型、`draggingEntered`、`draggingUpdated`、`draggingExited`、`prepareForDragOperation`、`performDragOperation`。任何结束路径（drop、取消、拖出、窗口失焦、应用切走）MUST 恰好发出一次 `DragEnded`。MUST NOT 只实现 `performDragOperation`。
- **失败模式**：拖出窗口后 drop 高亮永久残留；被拒绝的 drop 仍显示“已添加”；同一次拖放触发两次插入。
- **来源**：`CLAIM-081`。

### SPEC-PLAT-MACOS-008 原生菜单是快捷键通路，不是可选装饰

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：应用 MUST 安装 `NSApp.mainMenu`，至少包含 App 菜单与 Edit 菜单（`undo:`/`redo:`/`cut:`/`copy:`/`paste:`/`selectAll:`）。菜单命令 MUST 路由到当前焦点 editor 的同一个 reducer，MUST NOT 绕过 `SPEC-TEXT-012` 的提交路径。同一快捷键 MUST 只有一条处理通路：MUST NOT 同时由菜单 key equivalent 与 `keyDown` 各执行一次。
- **失败模式**：自绘文本框里 Cmd+C/V/X/Z 全部无效（因为没有 Edit 菜单与标准 selector）；或一次 Cmd+Z 撤销两步。
- **来源**：AppKit 菜单/响应链模型；FINAL §5（编辑状态机）。

### SPEC-PLAT-MACOS-009 无障碍适配器只做映射，几何来自同一快照

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：AccessKit → `NSAccessibility` 适配器 MUST 从与 paint/hit 相同的 `GeometrySnapshot` 取矩形，并在唯一坐标 adapter 里转换；MUST 同步更新 `NSAccessibilityFocusedUIElement`。不支持的 role/action MUST 返回明确的 unsupported 语义，MUST NOT 编造可访问性。MUST NOT 把语义 dump 或单元测试表述为 VoiceOver 已通过——依据 `SPEC-RENDER-018`，真实播报只在 C Gate 成立。
- **失败模式**：VoiceOver 只读出“窗口”；Tab 移动焦点但读屏焦点不跟随；读出的控件位置与实际可见位置错位。
- **来源**：`SPEC-RENDER-017`/`-018`；`CLAIM-080`。

### SPEC-PLAT-MACOS-010 打包与签名属于发布门，不属于“能跑”

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：发布物 MUST 是 `.app` bundle（Info.plist、图标、正确的 activation policy、菜单栏行为）；MUST 以 Developer ID 签名并公证、staple 到 bundle；MUST 分别产出 arm64 与 x86_64（或 universal）产物；MUST 在干净机器上通过 Gatekeeper 启动。嵌套二进制 MUST 逐个签名，MUST NOT 依赖递归签名开关。MUST NOT 把 `cargo run` 能起窗口当作可发布证据。
- **失败模式**：用户侧提示“已损坏”或“无法验证开发者”；离线机器因票据未 staple 被拦；裸二进制运行导致 Dock 图标、菜单栏或激活行为异常。
- **来源**：`SPEC-GATE-C-05`；FINAL §9。

### SPEC-PLAT-MACOS-011 窗口关闭与应用退出是两件事

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：窗口关闭 MUST NOT 隐式终止应用；应用退出只由显式 quit 命令或平台终止流程触发。Dock 图标被再次激活时 MUST 重建窗口并接回同一 runtime，MUST NOT 重启进程内状态或重置 Store。MUST NOT 在 macOS 上把 Windows 的“关最后一个窗口即退出”语义伪装成跨平台默认；该差异 MUST 在公开 API 文档中显式说明。
- **失败模式**：用户关掉窗口后整个应用消失（违反平台习惯）；或进程残留但 Dock 图标点不出窗口。
- **来源**：`SPEC-SCOPE-021`（能力宣称纪律）；AppKit 应用生命周期。

## 2. 平台差异矩阵

判定列只有两种合法值：**吸收**（抽象层统一，业务层不可见）与**暴露**（允许业务层看到差异，但必须有 typed 能力查询或文档化语义）。任何“既吸收又暴露”的行都视为规格缺陷。

| # | 差异 | 在 macOS 上的具体表现 | 判定 | 边界与理由 |
|---|---|---|---|---|
| 1 | IME 组合事件时机 | marked text 分段更新，commit 可能一次到达；取消路径与 Windows 不同 | 吸收 | 统一为 `ImeEvent::{Preedit, Commit, Cancel}`；消费顺序由 `SPEC-TEXT-014` 唯一持有 |
| 2 | 候选窗定位 | `firstRect(forCharacterRange:)` 使用屏幕坐标、原点左下 | 吸收 | 唯一坐标 adapter 转换（`SPEC-PLAT-MACOS-003`） |
| 3 | 合成/呈现模型 | `CAMetalLayer` drawable、Core Animation 事务、present 时机影响撕裂与延迟 | 吸收 | WindowShell 内部策略；对外只承诺“成功 present 后提交帧状态”（`SPEC-RENDER-006`） |
| 4 | 屏幕坐标原点 | AppKit 左下原点 vs 布局左上原点 | 吸收 | 与 #2 共用同一 adapter |
| 5 | 命令修饰键 | Command 是主修饰键；Control 语义不同 | 吸收 + 文档化 | 逻辑 `Shortcut` 映射到 Cmd；文本编辑快捷键仍走原生菜单 selector（`SPEC-PLAT-MACOS-008`） |
| 6 | 字体枚举与 fallback | 系统字体集与 Windows/Linux 不同，系统字体随 OS 版本漂移 | 暴露 | 公开能力查询“本平台可用字体集”；像素 golden MUST 用随包固定字体，MUST NOT 用系统字体当基线 |
| 7 | 字形栅格细节 | 抗锯齿/子像素/hinting 与其它平台不同 | 暴露 | 只承诺布局度量与字形身份一致，不承诺逐像素跨平台一致 |
| 8 | 无障碍 API 能力 | `NSAccessibility` 是进程内 ObjC 协议，role/action 集与 UIA/AT-SPI 不同 | 吸收 + 能力位 | 不支持的能力返回 typed unsupported；不得静默丢弃 |
| 9 | HiDPI / 缩放模式 | backing scale 与逻辑分辨率解耦；“更多空间”改变逻辑尺寸 | 吸收 | 以浮点 scale 承载；禁止整数枚举假设 |
| 10 | 滚动事件 | precise deltas、momentum phase、rubber-band 越界 | 吸收 | 统一为逻辑像素 + phase；ScrollState 唯一解释（`SPEC-RENDER-015`） |
| 11 | 窗口生命周期 | 关窗不退出、Dock reopen、全屏 Space、window tabbing | 暴露 | 平台语义真实存在，强行统一会伤害用户预期（`SPEC-PLAT-MACOS-011`） |
| 12 | 全局菜单栏 | 菜单在屏幕顶部，不在窗口内 | 吸收 | 业务层只见到 `Command`，不画菜单栏 |
| 13 | 沙盒文件访问 | `NSOpenPanel`/拖放返回的 URL 可能带 security scope | 暴露 | 以 typed 权限结果暴露；不得假装普通路径 IO 可用 |
| 14 | 文本服务 | Services、听写、Emoji 选择器等以输入事件进入 | 吸收 | 统一为 text input / command 事件 |
| 15 | 颜色管理 | ColorSync、P3/EDR、layer colorspace 影响输出 | 吸收 + 能力位 | 管线工作空间归 `SPEC-RENDER-011`；平台只提供 format/colorspace 能力清单 |

## 3. 失败模式

每条都必须能落到一个**用户或测试可观测的症状**；只写“可能出错”不算规范。

| ID | 触发条件 | 可观测症状 | 必须的处理 |
|---|---|---|---|
| `SPEC-PLAT-MACOS-020` | live resize；窗口最小化/隐藏时 drawable size 为 0 | 拖动边框时黑闪、内容拉伸一帧；日志出现 surface 配置错误 | `SPEC`：尺寸为 0 时跳过 surface 配置；resize 合并到一帧；只在尺寸真正变化时重建 |
| `SPEC-PLAT-MACOS-021` | 呈现时序与 Core Animation 事务不一致 | 快速拖动窗口时撕裂，或画面稳定落后一帧 | `SPEC`：呈现策略必须可切换并用 C Gate 证据选定；MUST NOT 用“看起来不闪”当结论（`SPEC-RENDER-025`） |
| `SPEC-PLAT-MACOS-022` | GPU 重启/驱动异常导致 device removed | 窗口变黑且持续报错，或进程崩溃 | `SPEC`：走 `SPEC-RENDER-013` 的清理→重建→发布；不得在旧 device 上继续提交 |
| `SPEC-PLAT-MACOS-023` | composition 期间 Cmd+Tab、点击其它应用、窗口 resign key | 切回后组合串残留、重复提交，或输入法候选窗悬空 | `SPEC`：resignKey 时按 reducer 语义提交或取消，且恰好一次；候选窗必须随焦点关闭 |
| `SPEC-PLAT-MACOS-024` | 窗口被完全遮挡、最小化，或进入 App Nap | 不可见窗口仍持续渲染，耗电与风扇异常 | `SPEC`：响应遮挡/最小化事件停帧；恢复时整帧重绘（`SPEC-RENDER-014`） |
| `SPEC-PLAT-MACOS-025` | 进出全屏 Space、Stage Manager 切换、跨显示器拖动 | 一帧使用旧 scale/旧尺寸；文字模糊或命中偏移 | `SPEC`：`ScaleChanged` 与 `Resize` 必须原子处理，禁止先重绘后重排 |
| `SPEC-PLAT-MACOS-026` | 菜单 key equivalent 与 `keyDown` 同时处理 | 一次 Cmd+Z 撤销两步；一次 Cmd+V 粘贴两份 | `SPEC`：快捷键唯一通路（`SPEC-PLAT-MACOS-008`） |
| `SPEC-PLAT-MACOS-027` | vibrancy/材质层与 `CAMetalLayer` 叠加 | 透明区域出现黑块、内容不透或整块不显示 | `SPEC`：必须能力探测；不支持时显式回退为不透明背景，MUST NOT 静默画黑 |
| `SPEC-PLAT-MACOS-028` | 拖拽中途取消、拖出窗口、拖拽期间窗口失焦 | drop 高亮残留；同一次拖放插入两条 | `SPEC`：拖放状态机必须闭合，`DragEnded` 恰好一次（`SPEC-PLAT-MACOS-007`） |
| `SPEC-PLAT-MACOS-029` | 沙盒下使用 drop 或打开面板返回的 URL | 读取被拒，但 UI 已显示“文件已打开” | `SPEC`：文件访问失败必须是 typed 错误并回滚 UI 状态；不得把权限失败吞成空内容 |
| `SPEC-PLAT-MACOS-030` | 高频 IME + 焦点切换 + 外部替换交错 | 候选窗不消失、文字重复、caret 跳回行首 | `SPEC`：所有文本变更经同一 reducer（`SPEC-TEXT-012`/`-013`）；平台层不得旁路 |

## 4. Gate C 该平台的最小证据清单

下列条目是 macOS 进入 `SPEC-GATE-C` 的**最小**集合。它们当前全部为 `OPEN`；MUST NOT 用 headless readback（`EVID-VAL-15` 只证明 Metal 上离屏 RGBA 一致，`CLAIM-008`）替代真实窗口证据。编号为本文内部的需求编号，落地时由证据负责人按 `docs/README.md` §3 分配 `EVID-VAL-*`，本文不复述其编号规则。

| ID | 要证明什么 | 最小方法 | 关联 Gate | 状态 |
|---|---|---|---|---|
| `SPEC-PLAT-MACOS-E01` | 真实 `NSWindow` + Metal surface + present + 连续 resize + 跨显示器拖动 | 真机运行 + 屏幕录制 + 日志（OS/架构/adapter/commit/lock hash） | `SPEC-GATE-C-01` | `OPEN` |
| `SPEC-PLAT-MACOS-E02` | 键盘、pointer、wheel、Command 快捷键、Edit 菜单通路 | 交互 trace：每次按键只产生一个命令 | `SPEC-GATE-C-01` | `OPEN` |
| `SPEC-PLAT-MACOS-E03` | 中文 IME 组合、选词回车、候选窗位置、取消 | IME trace：preedit/commit/submit/insert/clear 计数；候选窗截图 | `SPEC-GATE-C-02`（场景同 `SPEC-GATE-A1-07`） | `OPEN` |
| `SPEC-PLAT-MACOS-E04` | VoiceOver 实际播报、导航、动作、焦点同步 | 屏幕录制 + 同一 epoch 的语义树快照与几何快照 | `SPEC-GATE-C-02` | `OPEN` |
| `SPEC-PLAT-MACOS-E05` | 静止 30–60 秒不产生每帧重绘 | Instruments / 平台采样 + 帧计数 | `SPEC-GATE-C-06` | `OPEN` |
| `SPEC-PLAT-MACOS-E06` | `.app` 打包、Developer ID 签名、公证、staple、干净机启动、universal 产物与 entitlements 清单 | `codesign -dv`、`codesign -d --entitlements :-`、`spctl -a -vvv`、`xcrun stapler validate`、`lipo -info` 输出 | `SPEC-GATE-C-05` | `OPEN` |
| `SPEC-PLAT-MACOS-E07` | Metal 后端能力清单与 `Unsupported` 路径（不 panic） | 构造不支持的 format/alpha/filter 输入，记录 typed 错误 | `SPEC-GATE-C-03` | `OPEN` |
| `SPEC-PLAT-MACOS-E08` | 沙盒/非沙盒下的剪贴板、拖放、文件访问 | 两种签名配置各跑一遍权限矩阵 | `SPEC-GATE-C-01`/`-05` | `OPEN` |
| `SPEC-PLAT-MACOS-E09` | 长时间 churn 后 owner/GPU 资源回到温缓存基线 | 前后资源计数对照 | `SPEC-GATE-C-07` | `OPEN` |

## 5. 不确定项

以下事实我不敢在 spec 层断言，全部 `OPEN`。每条给出验证路径；没有路径的 `OPEN` 视为不合格。

| ID | 不确定的是什么 | 为什么现在不能定 | 验证路径 |
|---|---|---|---|
| `OPEN-PLAT-MACOS-01` | 当前窗口库在 macOS 上对 `NSTextInputClient` 的覆盖是否完整（marked text、`firstRect` 语义、坐标原点） | 仓库无任何窗口/IME 代码（`CLAIM-009`/`CLAIM-079`），只有版本 pin（`SPEC-ROUTE-003` 说明三元组从未一起编译运行） | 建一个最小 NSView 探针，用拼音输入法记录 `setMarkedText`/`firstRect` 调用序列与截图；结果登记为 `EVID-VAL-*` |
| `OPEN-PLAT-MACOS-02` | `accesskit_winit` 在 macOS 上是否提供完整 `NSAccessibility` 适配，VoiceOver 能否读出编辑器的 selection/range | `CLAIM-080`：语义树、VoiceOver 均未验证；pin 组合未编译（`CLAIM-082`） | 最小窗口 + AccessKit 探针 + VoiceOver 录屏；同时导出语义树快照比对 |
| `OPEN-PLAT-MACOS-03` | 呈现时序策略（是否同步 Core Animation 事务、drawable 数量）对 live resize 撕裂与延迟的实际影响 | 没有真实 surface，只有离屏 readback（`CLAIM-008`） | 同一场景切换策略，用 GPU capture + 帧时间分布对比；按 `SPEC-RENDER-025` 记录 workload 与主机 |
| `OPEN-PLAT-MACOS-04` | 最低支持系统版本与分发渠道（独立分发 vs App Store、是否强制沙盒） | 这是产品/发布决策，不是实现细节；本文被禁止写版本号 | 需要一条 `ADR-*`；决定后由 `SPEC-ROUTE` 记录版本，平台文档只引用 |
| `OPEN-PLAT-MACOS-05` | Hardened Runtime 下运行时编译 Metal shader 是否需要额外 entitlement | 未做签名后真机验证；不同签名配置可能行为不同 | 公证后的产物在干净机器运行图形用例；失败则记录错误码并评估 entitlement |
| `OPEN-PLAT-MACOS-06` | 沙盒下拖放得到的文件访问是否需要 security-scoped bookmark 才能跨会话复用 | 与分发决策（`OPEN-PLAT-MACOS-04`）耦合，尚无签名产物 | 沙盒与非沙盒各跑 E08 矩阵；分别验证当次访问与重启后访问 |
| `OPEN-PLAT-MACOS-07` | GPU device removed 在 macOS 上能否被程序化触发 | 没有公开的故障注入手段，只能靠 sleep/wake 或异常偶发 | 设计可重复的挂起/恢复与压力脚本；若不可控，在 C Gate 记为 `blocked` 而非通过（`SPEC-GATE` §0 规则 2） |
| `OPEN-PLAT-MACOS-08` | 全屏 Space、Stage Manager、多显示器组合下 scale/resize 事件的顺序与次数是否稳定 | 未在真机矩阵上观测过；不同 macOS 大版本可能变化 | 真机组合矩阵录屏 + 事件日志；把观测到的顺序写成 golden，而不是假设 |
| `OPEN-PLAT-MACOS-09` | 系统字体随 OS 更新漂移对布局度量与像素 golden 的影响幅度 | 无固定字体资产与跨版本对照（`OPEN-TEXT-05` 同类问题） | 用随包固定字体建立 golden；另做系统字体差分作为信息性观测，不进门禁 |
