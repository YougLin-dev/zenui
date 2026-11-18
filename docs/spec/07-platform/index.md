---
doc-id: SPEC-PLAT
layer: spec
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - spec/07-platform/windows.md
  - spec/07-platform/macos.md
  - spec/07-platform/linux.md
replaces: []
---

# SPEC-PLAT · 平台层：三平台共有的不变量

> 本文件只定义**三个平台都必须满足、且写三遍会漂移**的不变量。平台特有的行为、差异矩阵、失败模式与 Gate C 证据清单归各分册，本文不复述。
>
> 分册：[windows.md](windows.md) · [macos.md](macos.md) · [linux.md](linux.md)。状态词只用 `SPEC / PROBE / WITHDRAWN / OPEN`；本层不写 `VERIFIED`。
>
> 现有事实状态：三平台**均无真实窗口级证据**。唯一的执行证据是 `evidence/validation.md` 中的 headless GPU readback（Windows/NVIDIA/Vulkan 为上一轮转述，macOS/Metal 为本轮实测），headless 通过 **不等于**窗口、输入、IME、无障碍或打包成立。

## 1. 平台层的定位

### SPEC-PLAT-001 平台层不是 UI 权威

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：平台后端只做四件事——**事件泵**、**输入翻译**、**呈现（present）**、**系统服务适配**。它 MUST NOT 持有 View 树、样式、布局结果或编辑器状态；任何平台回调都只能产生"事件 + 时间戳 + 坐标"这样的中性数据交给核心。
- **失败模式**：在 Win32 消息循环里直接改 UI 数据 → 违反 `SPEC-RT-001`，出现第二条刷新路径与跨线程写入。

### SPEC-PLAT-002 三平台共享同一组语义，不共享同一份代码路径

- **状态**：SPEC
- **强度**：MUST
- **要求**：`WindowShell`、`InputAdapter`、`ImeAdapter`、`A11yAdapter`、`ClipboardAdapter` 是**语义 trait**，允许三个平台实现完全不同；但对外行为（事件顺序、坐标单位、IME 提交时机、焦点语义）MUST 收敛到同一套规范，差异只能出现在"能力是否可用"，不能出现在"语义是什么"。
- **失败模式**：Windows 的 IME 提交走 `WM_IME_ENDCOMPOSITION`、macOS 走 `insertText:`，若各自直接写 Store，同一段中文在不同平台的 reducer 里产生不同结果。

### SPEC-PLAT-003 能力是查询来的，不是假设来的

- **状态**：SPEC
- **强度**：MUST
- **要求**：`RenderCapabilities` / `PlatformCapabilities` MUST 在创建窗口后查询（format、present mode、DPI scale、是否支持 tray、是否支持平滑滚动等），未支持的能力返回明确的 `Unsupported`，MUST NOT 静默降级成另一种语义。
- **失败模式**：在没有 tray 的合成器上"成功"创建了一个永远不可见的托盘图标；或在不支持某 format 的后端上拿到全黑帧但没有错误。

## 2. 三平台共有的硬约束

### SPEC-PLAT-010 单一事件泵，且只在主线程

- **状态**：SPEC
- **强度**：MUST
- **要求**：窗口事件、reactive flush、布局、绘制提交 MUST 在同一 UI 主线程上顺序发生；worker 只能通过带 generation 的桥回传。任何平台都不得出现"平台线程直接 flush"。
- **失败模式**：macOS 上在 `NSEvent` 回调里触发 layout 导致重入；Linux 上 Wayland 事件与 render thread 竞争同一 `present`。
- **分册**：`SPEC-PLAT-WIN-002`、`SPEC-PLAT-MACOS-002`、`SPEC-PLAT-LINUX-002`。

### SPEC-PLAT-011 DPI 是一等公民，不是缩放的补丁

- **状态**：SPEC
- **强度**：MUST
- **要求**：DPI/scale 变化 MUST 作为独立事件触发资源重建（字形、图集、surface 配置），MUST NOT 被折叠成一次普通 resize；输入坐标 MUST 在进入核心前统一到逻辑坐标，绘制坐标 MUST 在提交前统一到物理像素。
- **失败模式**：从 100% 拖到 150% 后文字糊、命中偏移半个字、surface 尺寸与 clip 不一致。
- **分册**：`SPEC-PLAT-WIN-007`、`SPEC-PLAT-MACOS-005`、`SPEC-PLAT-LINUX-005`/`-019`。

### SPEC-PLAT-012 IME 组合态归编辑器，提交态归 reducer

- **状态**：SPEC
- **强度**：MUST
- **要求**：组合中的预编辑文本 MUST 停留在编辑器层（不写入数据模型），只有 commit 才进入文本 reducer；取消组合 MUST 精确回滚到组合前状态。
- **失败模式**：拼音未上屏就写 Store，撤销栈被污染；或组合被取消后残留字符。
- **分册**：`SPEC-PLAT-WIN-006`、`SPEC-PLAT-MACOS-004`、`SPEC-PLAT-LINUX-004`。

### SPEC-PLAT-013 无障碍几何与绘制同源

- **状态**：SPEC
- **强度**：MUST
- **要求**：AT 暴露的 bounding box / 角色 / action MUST 来自与绘制相同的 `GeometrySnapshot` 和同一棵 View 树；任何"a11y 自己算一遍位置"的实现 MUST NOT 被接受。
- **失败模式**：VoiceOver 点击落在视觉位置的旁边；读屏读到的顺序与视觉顺序不一致。
- **分册**：`SPEC-PLAT-WIN-011`、`SPEC-PLAT-MACOS-009`、`SPEC-PLAT-LINUX-009`。

### SPEC-PLAT-014 平台特有服务是可失败的可选模块

- **状态**：SPEC
- **强度**：SHOULD
- **要求**：剪贴板、拖放、菜单、托盘、全局快捷键 MUST 设计成可失败、可缺省的模块，失败时 UI 必须仍可使用并给出可理解的降级，而不是 panic 或静默吞掉。
- **失败模式**：Linux 上无 xdg-desktop-portal 时剪贴板调用阻塞主线程数秒。

### SPEC-PLAT-015 证据按平台、按显示服务器、按后端分别记账

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：MUST NOT 用任一平台的一次通过去替代另一平台；Linux 的 Wayland 与 X11 MUST 分别记录；GPU 后端（DX12/Vulkan/Metal/GL）分别记录。能力矩阵中的每个格子都是独立的 `OPEN`/`PROBE`/`VERIFIED`。
- **依据**：`SPEC-GATE-C`；`CLAIM` 台账中 H1（Windows/NVIDIA/Vulkan 转述）与 H2（macOS/Metal 实测）的分列。
- **失败模式**：在三平台文档里写"跨平台已验证"，实际只有一台机器跑过一次 headless。

### SPEC-PLAT-016 ID 命名空间：跨平台不变量与平台分册分开编号

- **状态**：SPEC
- **强度**：MUST
- **要求**：跨平台不变量用 `SPEC-PLAT-0NN`（本文件）；平台分册用 **子命名空间** `SPEC-PLAT-WIN-NNN` / `SPEC-PLAT-MACOS-NNN` / `SPEC-PLAT-LINUX-NNN`。三者 MUST NOT 共用裸 `SPEC-PLAT-NNN` 编号，避免"哪一条属于哪个平台"靠记忆判断。
  - 平台分册内部的**证据需求槽**统一用 `SPEC-PLAT-<OS>-E NN`（`E01…`），它表达"需要什么证据"，**不是证据**，MUST NOT 占用 `EVID-VAL-*` / `EVID-PROBE-*` 编号空间；真正执行后由 evidence 负责人分配 `EVID-*` 序号回填。
  - 反例（已修正）：Linux 分册曾用 `EVID-VAL-LX-*` 作占位，等于在未执行的情况下占用证据命名空间；Windows 分册曾用 `-C01` 与 macOS 的 `-E01` 并存。现三者统一为 `-E01…`。
- **失败模式**：三份分册各自从 001 起编号且不带平台前缀 → 交叉引用静默指向错误条目。

### SPEC-PLAT-017 平台默认行为可以不同，但必须显式声明且可覆盖

- **状态**：SPEC
- **强度**：MUST
- **要求**：确实属于平台习惯的默认行为差异（当前已知：macOS 关窗不退出、Dock 重新激活接回同一 runtime；Windows 关最后一个窗口即退出）MUST 由平台分册**显式声明为默认值**，并在公开 API 文档中标注；应用层 MUST 能覆盖该默认。框架核心 MUST NOT 把任一平台的语义硬编码成跨平台默认。
- **失败模式**：在 macOS 上照搬 Windows 的"关窗即退出"，用户关窗后应用消失；或反过来在 Windows 上关窗后进程残留且无入口。
- **分册**：`SPEC-PLAT-MACOS-011`、`SPEC-PLAT-WIN-013`。

## 3. Gate C 的共同结构

每个平台分册的第 4 节给出该平台的最小证据清单，共同骨架是：

| 证据类别 | 内容 | 当前状态 |
|---|---|---|
| 真实窗口 | 创建、present、resize、DPI 变化、关闭 | `OPEN`（三平台） |
| 输入 | 键鼠、焦点、滚动、快捷键 | `OPEN`（三平台） |
| IME | 组合、提交、取消、候选窗定位 | `OPEN`（三平台） |
| 无障碍 | 真实 AT 播报与 action（NVDA / VoiceOver / Orca） | `OPEN`（三平台） |
| 打包 | 可分发产物（复现构建） | `OPEN`（三平台） |

> 任何一格转绿都必须有 `EVID-VAL-*` 记录与主机标识，见 [evidence/index.md](../../evidence/index.md)。
