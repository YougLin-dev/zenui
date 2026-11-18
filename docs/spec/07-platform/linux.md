---
doc-id: SPEC-PLAT-LINUX
layer: spec
status: draft
date: 2026-11-18
owner: Linux 平台工程师（10 年+）
sources:
  - archive/review/FINAL.md §7.2/§9 C Gate
  - archive/review/round1-experts.md（Wayland/X11、IME、DPI 风险条目）
  - archive/review/VALIDATION.md
  - archive/research/validation-and-roadmap.md
replaces: []
---

# SPEC-PLAT · Linux 桌面后端（Wayland 与 X11 分列）

> 本文件是 `SPEC-PLAT` 的 Linux 分册，定义 **Linux 后端必须满足什么**，**不声明任何已验证**。事实状态归 `docs/evidence/`：没有 `EVID-VAL-*` 的能力一律 `SPEC` 或 `OPEN`。
>
> **Wayland 与 X11 是两条独立证据路径**，MUST NOT 互相背书（`SPEC-SCOPE-002`、`SPEC-GATE-C-01`）；XWayland 下运行按 X11 路径记录，不得记作 Wayland。
>
> 文中出现的 `text-input-v3`、`wp_fractional_scale_v1`、`xdg-shell` 等是**协议标识名**，不是依赖 pin；依赖版本唯一出处是 [spec/01-route.md](../01-route.md)（`SPEC-ROUTE-001`），本文不复述任何 crate/工具链版本。
>
> 责任分层：本文只规定 Linux 后端如何**实现**既有合同——surface/DPI/设备生命周期归 [SPEC-RENDER](../06-render.md)（`SPEC-RENDER-012/013/014`），IME 与坐标语义归 [SPEC-TEXT](../05-text.md)（`SPEC-TEXT-011/012/014/015`），焦点结算归 [SPEC-RT](../02-runtime.md)（`SPEC-RT-026`），语义树合同归 `SPEC-RENDER-017/018`。本文不重新定义这些语义，只定义 Linux 侧的**采集、适配与降级**。

## 1. 后端职责与边界

**分层总则**：`zenui-platform` 的 Linux 实现是唯一接触 winit / Wayland / X11 / DBus / portal 的层；应用与 `zenui-render` MUST NOT 持有 `wl_surface`、`xcb_window_t`、`wl_display` 等原生句柄。每一项职责都必须回答三件事：归谁、经什么抽象、失败时长什么样。

### SPEC-PLAT-LINUX-001 窗口创建、装饰与窗口管理器契约

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **归谁**：`zenui-platform`（WindowShell 的 Linux 实现），经自有 `WindowShell` trait 暴露给运行时。
- **要求**：
  - Wayland 路径 MUST 用 `xdg-shell` toplevel 创建窗口，并把 **app_id 设为 `.desktop` 文件标识**；`xdg-decoration` 是可选协议，后端 MUST 同时支持 **CSD 与服务端装饰（SSD）** 两种结果：协商到 CSD 时由框架绘制标题栏与命中区，协商到 SSD 时 MUST NOT 再画一套。
  - X11 路径 MUST 设置 EWMH/ICCCM 必需属性（`WM_CLASS`、`_NET_WM_NAME`、最小/最大尺寸、`_NET_WM_PID`），普通应用窗口 MUST NOT 使用 override-redirect；override-redirect 只允许框架自有的 tooltip/popup 使用。
  - 窗口最小/最大尺寸是**建议值**：平铺式窗口管理器可以忽略。后端 MUST 以实际 `Resized` 事件为准，不得假设请求值已生效。
- **失败模式**：GNOME 下没有关闭按钮或出现双标题栏；任务栏显示通用图标、无法按 app_id 归组；sway 平铺时窗口被强制成与请求不同的尺寸，布局却按请求值计算。
- **来源**：FINAL §7.2；`SPEC-RENDER-012`；round1-experts（Wayland 装饰与坐标差异）。

### SPEC-PLAT-LINUX-002 事件循环嵌入与线程模型

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **归谁**：`zenui-platform` 的 `EventLoopHost`；运行时只提交 deadline 与唤醒请求。
- **要求**：
  - 窗口与事件循环 MUST 在**进程主线程**创建并驱动；同一进程 MUST 只有一个窗口事件循环。
  - 空闲 MUST 使用事件等待（`ControlFlow::Wait` 类语义），动画与外部请求经 deadline / 显式 wake 驱动（与 `SPEC-RENDER-014` 一致）。
  - MUST NOT 在主线程同步等待剪贴板、DnD、portal 或 AT-SPI 的 I/O；这些 MUST 走异步任务或工作线程，结果回到主线程提交。
  - 把框架嵌入既有 GTK/Qt 主循环、或在库内部自建第二套事件循环 **不在支持范围**，MUST 返回 typed `Unsupported`，不得假装可用。
- **失败模式**：读取大段剪贴板时整个 UI 冻结、frame callback 停摆；在非主线程建窗导致 panic 或未定义行为；与宿主工具包争抢主循环造成双方都收不到事件。
- **来源**：FINAL §7.2；`SPEC-RENDER-014`；`SPEC-ROUTE-013`。

### SPEC-PLAT-LINUX-003 输入设备模型、键盘布局与 compose

- **状态**：SPEC（compose/dead key 部分 OPEN）
- **强度**：MUST / MUST NOT
- **归谁**：`zenui-platform` 输入适配层 → 运行时事件分发；输入语义（焦点、命中）仍归 `SPEC-RENDER-015` 与 `SPEC-RT-026`。
- **要求**：
  - 后端 MUST 同时提供**逻辑键 + 物理 scancode + modifiers**，不得只给字符；快捷键绑定 SHOULD 用物理键，文本输入用逻辑键。
  - 键盘布局由合成器/X 服务器提供，运行中切换布局 MUST 生效；后端 MUST 监听 keymap 变化并重建键映射。
  - Wayland 下 **没有全局指针坐标**：后端 MUST 只暴露窗口局部坐标；相对运动、指针锁定/捕获依赖协议与合成器支持，缺失时 MUST 以 typed `Unsupported` 暴露，MUST NOT 用绝对坐标伪造。
  - 触摸板手势（pinch/rotate/swipe）在不同合成器上可用性不一，MUST 以能力位暴露，MUST NOT 假设存在。
  - **compose / dead key 是否由 pinned 输入栈完成组合，当前 `OPEN`**（见 §5 `OPEN-LX-05`）：后端 MUST 以实际 `text` 事件为准，不得在框架内私自拼装 Unicode。
- **失败模式**：切换到第二键盘布局后快捷键仍按旧布局触发；Wayland 下用绝对坐标做拖拽导致指针与内容偏移；按 compose 键输入 `é` 得到两个独立字符。
- **来源**：FINAL §7.2；round1-experts；`SPEC-RENDER-015`。

### SPEC-PLAT-LINUX-004 IME 接入：text-input 协议与候选窗定位

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **归谁**：`zenui-platform` 的 IME adapter 采集协议事件 → 编辑器 reducer（`SPEC-TEXT-012`）。组合态生命周期**归 `SPEC-TEXT`**，本文只规定 Linux 侧协议映射。
- **要求**：
  - Wayland 路径以 `text-input-v3` 为主（fcitx5 / IBus 均经此协议），X11 路径以 XIM 为回退；两条路径 MUST 都映射为同一组编辑器命令：`preedit_update`、`commit`、`delete_surrounding`、`cursor_rect`、`cancel`。
  - 每次 caret 移动、滚动、窗口 resize、scale 变化后 MUST 重新提交 **surface 坐标下的光标矩形**（`set_ime_cursor_area` 类接口），否则候选窗会停在旧位置。
  - 窗口失焦、编辑器 dispose、UI generation 变化时 MUST 结束当前 composition 并丢弃迟到事件；MUST NOT 把 commit 写到新 generation 的编辑器。
  - 候选激活时的 Enter MUST 由 IME/编辑器先消费，不前置于表单提交（`SPEC-TEXT-014`）；后端 MUST NOT 自行改写这一顺序。
  - `text-input-v3` 与 XIM 在 preedit 样式、周围文本、删除语义上的差异 MUST 作为**能力差异**记录；MUST NOT 用模拟手段假装两者等价。
- **失败模式**：候选窗出现在屏幕角落或上一光标位置；切换窗口后组合串黏在新输入框；中文选词回车误建任务；X11 下 preedit 直接上屏无法取消。
- **来源**：FINAL §5/§7.1；`SPEC-TEXT-012/014`；round1-experts（Wayland scroll/IME 风险）。

### SPEC-PLAT-LINUX-005 DPI 与分数缩放

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **归谁**：`zenui-platform` 的 scale adapter；布局单位与 DPR epoch 的语义归 `SPEC-STYLE` / `SPEC-TEXT-005` / `SPEC-RENDER-002`。
- **要求**：
  - Wayland 路径 MUST 处理 `wp_fractional_scale_v1` + `wp_viewporter`：逻辑尺寸与缓冲区尺寸分离，scale 可以是 1.25、1.5 这类非整数；缓冲区的整数 `buffer_scale` 与逻辑坐标之间的换算 MUST 由后端完成。
  - X11 路径没有分数缩放协议：后端 MUST 按整数 scale（Xft/RandR 提示）处理，并把"实际取整"作为能力差异暴露，不得声称与 Wayland 等价。
  - scale MUST 按**窗口/输出**记录，MUST NOT 缓存为进程级全局值；窗口跨输出拖动后 MUST 以新 scale 重新布局、重排文字、重建 atlas，并与 `GeometrySnapshot` 的 epoch 同步推进。
  - 布局单位 MUST 保持逻辑像素；physical target 与文字 epoch 不得混入布局（与 `SPEC-STYLE` 单位规则一致）。
- **失败模式**：1.25 缩放下文字发虚或按钮比命中区小约 20%；拖到另一块显示器后字号不变但窗口与内容比例错乱；caret 与鼠标位置偏移半个字。
- **来源**：FINAL §7.2；`SPEC-RENDER-002`；round1-experts（per-monitor DPI 差异）。

### SPEC-PLAT-LINUX-006 剪贴板与 primary selection

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **归谁**：`zenui-platform` 的 clipboard adapter → 运行时异步命令；内容类型由应用层决定。
- **要求**：
  - Wayland 路径经 `wl_data_device`：数据传输是 **fd 流**，且只在源客户端存活、选中焦点有效时可服务；primary selection 是独立通道，MUST NOT 与剪贴板混为一谈。
  - X11 路径经 selection 所有权模型：后端 MUST 在事件循环内响应 `SelectionRequest`，MUST NOT 让请求方无限等待。
  - 读取/写入 MUST 异步；大对象 MUST NOT 阻塞主线程；超时 MUST 返回 typed 错误。
  - **退出后的持久化 MUST NOT 被承诺**：没有 clipboard manager 时源进程退出即丢失；后端 MAY 在有 `ext-data-control` 类能力时增强，但不得作为默认语义。
- **失败模式**：复制显示成功、粘贴为空（源进程已退出或未服务 fd）；粘贴图片得到乱码字节；读取剪贴板时应用假死数秒。
- **来源**：FINAL §7.2；round1-experts（clipboard/DnD 无实测）。

### SPEC-PLAT-LINUX-007 拖放（DnD）

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **归谁**：`zenui-platform` 的 DnD adapter；命中判定仍归 `GeometrySnapshot`（`SPEC-RENDER-001/015`）。
- **要求**：
  - Wayland 路径 MUST 使用指针事件携带的 serial 启动拖放会话，数据经 offer 的 fd 读取；拖拽图标是独立 surface，不得假定有全局坐标。
  - X11 路径按 XDND 处理；两条路径 MUST 统一为同一组事件：`enter/over/leave/drop` + MIME + 可选 URI 列表。
  - MUST 至少接受 `text/plain`、`text/uri-list`；图片等富类型按能力位暴露。
  - 沙盒（Flatpak/Snap）下文件路径可能**不可直接访问**：后端 MUST 以 portal 授予的 fd/路径为准，MUST NOT 假设拖入的 host 路径可打开。
- **失败模式**：拖放完全无响应（serial 过期或未取用）；沙盒内拿到路径却 `open` 失败；拖拽图标不跟随指针或残留在屏幕上。
- **来源**：FINAL §7.2；`SPEC-GATE-C-01`；round1-experts。

### SPEC-PLAT-LINUX-008 原生菜单与系统托盘

- **状态**：SPEC（系统托盘/全局菜单 `OPEN`，见 §5 `OPEN-LX-12`）
- **强度**：MUST NOT
- **归谁**：框架提供的**窗口内菜单**组件；平台层不承诺系统级集成。
- **要求**：
  - Linux 没有跨桌面可用的原生菜单栏；全局菜单依赖合成器/桌面环境，托盘依赖 StatusNotifierItem 宿主（GNOME 默认不提供）。
  - 框架 MUST 把菜单实现为**窗口内客户端菜单**；MUST NOT 在文档、API 或发布说明中承诺系统托盘、全局菜单或应用指示器。
  - 若未来提供托盘能力，MUST 以能力探测 + typed `Unsupported` 暴露，MUST NOT 静默不显示。
- **失败模式**：文档宣称支持托盘，但 GNOME 下图标根本不出现；sway 下全局菜单消失；用户以为应用最小化到托盘，实际进程仍在任务栏。
- **来源**：FINAL §7.2；round1-experts（平台差异）。

### SPEC-PLAT-LINUX-009 无障碍：AccessKit → AT-SPI over DBus

- **状态**：SPEC（Linux AT 适配器可用性 `OPEN`，见 §5 `OPEN-LX-03`）
- **强度**：MUST / MUST NOT
- **归谁**：`zenui-platform` 的 a11y adapter；语义树内容与几何来源归 `SPEC-RENDER-017`，播报结论归 `SPEC-RENDER-018`。
- **要求**：
  - 语义树 MUST 经 session DBus 上的 AT-SPI 服务暴露；节点矩形与 action 目标 MUST 来自同一 `GeometrySnapshot`，action MUST 先校验 `ViewId + generation`。
  - 角色映射 MUST 覆盖 Todo 场景所需：列表、列表项、切换按钮组、文本框、错误提示（live region 语义）。
  - 文本 selection/char range MUST 经文本层 adapter 转换（`SPEC-TEXT-011/015`），MUST NOT 把字节或 glyph 偏移交给 AT。
  - 节点增删、状态变化、错误文本出现 MUST 产生 AT-SPI 事件；仅在界面上画出文字 MUST NOT 被视为"已播报"。
  - 运行时依赖：AT-SPI 总线服务、session bus 可访问；沙盒内 MUST 声明相应权限。依赖缺失时 MUST 给出明确诊断，MUST NOT 静默变成无 a11y 应用。
- **失败模式**：Orca 读出旧行或停在已删除节点；错误提示可见但没有任何播报；Flatpak 下 AT 树为空；AT action 命中重建后的同位置新行。
- **来源**：FINAL §7.2；`SPEC-RENDER-017/018`；round1-experts（AT 无实测）。

### SPEC-PLAT-LINUX-010 打包、沙盒、资产与签名

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **归谁**：发布与 CI（产物）；平台层提供运行时能力探测。Gate 判据见 `SPEC-GATE-C-04/05`。
- **要求**：
  - 产物至少覆盖：发行版包（deb/rpm 之一）、AppImage、Flatpak；Snap 可选。C 类不再增加未声明的格式。
  - glibc 符号版本 MUST 以**目标发行版中最早的那一个**为基线；MUST NOT 用构建机的新符号让老发行版无法启动。AppImage 仍依赖宿主 glibc，不能当作"自带运行环境"。
  - MUST 随包提供：ICU 断词词典、确定性测试字体、第三方 licenses、lock 文件哈希（`SPEC-TEXT-008`、`SPEC-GATE-C-05`）。
  - `.desktop` 文件的标识 MUST 与 Wayland `app_id` / X11 `WM_CLASS` 一致，否则任务栏归组与通知都会错位。
  - Flatpak/Snap 下：文件选择 MUST 走 `xdg-desktop-portal`；Wayland/X11 socket、GPU 设备、session bus（含 AT 总线）MUST 按最小权限声明；MUST NOT 依赖未声明的宿主路径。
  - 签名与仓库元数据 MUST 与产物一同发布；未签名产物 MUST 在文档中明确标注。
- **失败模式**：新系统能跑、老发行版报 `GLIBC_... not found`；Flatpak 内点"打开文件"无反应；中文断行退化为按字切分（词典未打包）；任务栏出现两个互不相关的图标。
- **来源**：`SPEC-GATE-C-04/05`；`SPEC-TEXT-008`；FINAL §9 C Gate。

### SPEC-PLAT-LINUX-011 WindowShell 的 GPU surface 与呈现后端

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **归谁**：`zenui-platform` 实现 `SPEC-RENDER-012/013/014` 的 WindowShell 合同；渲染层只提交帧，不创建 surface、不调用 present。
- **要求**：
  - 每个真实窗口 MUST 有独立 surface：Wayland 经 Wayland surface 扩展，X11 经 Xlib/XCB surface 扩展；后端 MUST 选择与窗口协议匹配的 surface 类型，MUST NOT 跨协议复用。
  - 图形后端以 Vulkan 为主，GL/EGL 为回退；实际 adapter MUST 被记录并可在诊断中读取。混合显卡（PRIME）下 MUST 允许显式选择 adapter，MUST NOT 让"默认值"在不同机器上给出不同性能而不留痕。
  - 软件光栅器只能作为**显式**回退并给出用户可见提示；MUST NOT 把它静默当成硬件加速。
  - surface 错误（Lost / Outdated / Timeout）MUST 映射为 typed 结果并触发重建；0 尺寸（最小化）MUST 暂停渲染并保留 last-good 帧，MUST NOT 创建 0×0 swapchain。
  - present mode 与可变刷新率是**能力**，不是保证；不支持的组合 MUST 以 `RenderCapabilities` 暴露（`SPEC-RENDER-008`）。
- **失败模式**：双显卡笔记本选到低功耗 adapter，帧率骤降且日志无记录；无 Vulkan 时静默走 llvmpipe 被当作正常；最小化恢复时 validation panic；窗口关闭后仍持 surface 造成崩溃。
- **来源**：FINAL §7.2；`SPEC-RENDER-008/012/013/014`；round1-experts（Intel iGPU 上限）。

### SPEC-PLAT-LINUX-012 会话生命周期：挂起、锁屏与连接断开

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **归谁**：`zenui-platform` 的 session adapter；恢复后的渲染发布仍受 `SPEC-RENDER-013` 约束。
- **要求**：
  - 系统挂起/恢复 MUST 被视为**可能丢失 GPU 资源**：恢复后 MUST 先校验 device/surface，再决定重建；MUST NOT 直接复用挂起前的 atlas/binding。
  - 会话锁屏时合成器可能隐藏 surface，后端 MUST NOT 与锁屏界面争抢可见性或焦点。
  - Wayland 合成器退出/连接断开、X 服务器断开 MUST 被视为**不可恢复**：事件循环结束、给出 typed 致命错误并按应用策略保存状态；MUST NOT 静默挂起、无限重连或留下僵尸窗口。
  - 应用未保存数据的状态提示由应用层负责；平台层 MUST 只负责事件与错误上报，MUST NOT 代替应用做数据持久化。
- **失败模式**：合盖唤醒后黑屏或崩溃；注销重登后进程残留；SSH X11 断线后进程不退也不报错；挂起前未保存的内容无任何提示地丢失。
- **来源**：FINAL §7.2/§9 C Gate；`SPEC-RENDER-013`；round1-experts（device-loss 未验）。

## 2. 平台差异矩阵

### SPEC-PLAT-LINUX-013 差异必须显式归类：吸收或暴露

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：每一项 Linux 差异 MUST 被归为两类之一——**吸收**（框架在抽象层归一，业务不可见）或**暴露**（以能力位、typed `Result`、诊断字段呈现）。MUST NOT 用"猜一个合理值"的方式静默归一；被暴露的差异 MUST 能在诊断输出中观察到（会话类型、显示协议、scale、adapter、IME 协议、AT 可用性）。
- **失败模式**：应用在 sway 上正常、GNOME 上点击偏移，日志里既无 session 类型也无 scale/协议记录，无法分诊；同一代码在两台机器表现不同却没有任何能力位可查。
- **来源**：`SPEC-ROUTE-013`；`SPEC-SCOPE-002`；FINAL §7.2。

| # | 差异 | 对框架语义的影响 | 处置 | 关联 |
|---|---|---|---|---|
| 1 | Wayland / X11 / XWayland | 无全局坐标、焦点与抓取模型不同、装饰归属不同 | **吸收**为窗口局部语义；**记录**实际协议，XWayland 记作 X11 | 001/002/003 |
| 2 | 分数缩放（1.25/1.5）vs X11 整数缩放 | logical/physical 换算、文字与 atlas 精度 | Wayland **吸收**（每窗口 scale）；X11 取整**暴露** | 005 |
| 3 | `text-input-v3` vs XIM | preedit 样式、周围文本、删除语义 | 事件**吸收**为统一编辑器命令；能力差异**暴露** | 004 |
| 4 | CSD vs SSD | 标题栏与关闭按钮归属 | **吸收**（双模式，不双画） | 001 |
| 5 | fontconfig family 集合与 fallback | 字形选择、断行宽度 | 确定性**吸收**（固定测试字体）；family 缺失**暴露** | 021 |
| 6 | AT-SPI 服务可用性 | 语义树是否可达 | API **吸收**；运行时依赖**暴露** | 009 |
| 7 | 剪贴板退出后是否保留 | 复制语义 | **暴露**（不承诺持久化） | 006 |
| 8 | DnD + 沙盒路径可达性 | 拖入文件能否打开 | 协议**吸收**；沙盒限制**暴露** | 007 |
| 9 | 系统托盘 / 全局菜单 | 系统集成能力 | **暴露** `Unsupported` | 008 |
| 10 | GPU 后端与驱动能力 | format/alpha/clip/mask/filter | **吸收**为 `RenderCapabilities`；不支持**暴露** | 011 |
| 11 | present mode / VRR / 撕裂 | 帧呈现质量 | **暴露**为能力，不承诺 | 011 |
| 12 | 键盘布局运行中切换、compose | 文本与快捷键结果 | 布局**吸收**；compose 结果 `OPEN` | 003 |
| 13 | 合成器帧回调节奏 | 重绘时机与合并 | **吸收**为 damage 驱动；不承诺固定帧率 | 023 |
| 14 | 锁屏 / 挂起 / 会话断开 | 资源存活与进程生命周期 | 恢复路径**吸收**；不可恢复错误**暴露** | 012 |
| 15 | 平铺 WM 强制几何 | 请求尺寸与实际尺寸不一致 | **吸收**（以实际事件为准） | 001/015 |

## 3. 失败模式

本节把 Linux 特有的坑落到**可观测症状 + 处置合同**。与 §1 重复的部分只引用条目，不重复定义。

### SPEC-PLAT-LINUX-014 GPU 设备丢失、复位与挂起恢复

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：device loss（驱动复位、GPU hang、挂起丢显存、eGPU 热拔）MUST 被识别为 typed 错误；MUST 清理该设备的 atlas/binding/scene 缓存，重建成功后才发布新帧（`SPEC-RENDER-013`）；MUST NOT 无限重试，也 MUST NOT 静默降级到软件光栅而不提示。
- **可观测症状**：唤醒后窗口仍在但全黑；日志出现 device-lost 类错误后无后续帧；恢复后旧花纹/旧字形残留。
- **来源**：FINAL §7.2；`SPEC-RENDER-013`；`OPEN-LX-09`。

### SPEC-PLAT-LINUX-015 resize 与 configure 竞态

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：Wayland 的 configure→ack→commit 顺序 MUST 被遵守；过期的 configure 尺寸 MUST 被丢弃；resize 与 scale 变化 MUST 原子应用，不允许"新尺寸 + 旧 scale"或反之；最小化产生的 0 尺寸 MUST 暂停渲染并保留 last-good；X11 连续 `ConfigureNotify` MUST 合并后再提交。
- **可观测症状**：拖拽边框时闪一帧拉伸内容；最小化恢复后出现永久 1×1 或 validation panic；快速缩放窗口时文字与边框错位一帧。
- **来源**：`SPEC-RENDER-002/006`；round1-experts（resize/DPI 坐标）。

### SPEC-PLAT-LINUX-016 IME 事件丢失、迟到与错序

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：composition MUST 绑定到 `editor_id + editor_generation`；编辑器 dispose、焦点移走、UI generation 变化后到达的 preedit/commit MUST 被丢弃；候选窗矩形 MUST 随 caret/scroll/scale 更新；候选 Enter MUST 不触发提交（`SPEC-TEXT-014`）。
- **可观测症状**：组合串出现在下一个输入框；候选窗停在旧滚动位置；选词回车误提交；X11 下 preedit 上屏后无法取消。
- **来源**：`SPEC-TEXT-012/014`；`OPEN-LX-01/02`。

### SPEC-PLAT-LINUX-017 焦点丢失与抓取释放

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：收到系统焦点丢失 MUST 结束 composition、关闭 popup、释放指针/键盘捕获，并停止把按键送入编辑器；MUST NOT 在失焦后抢回焦点。结构删除引发的焦点转移只走 `FocusRemovalIntent`（`SPEC-RT-026`），与系统焦点变化是两件事。
- **可观测症状**：切到其他应用后快捷键仍被吞；弹出菜单留在最上层不消失；回来后输入框仍显示旧的组合下划线。
- **来源**：`SPEC-RT-026`；`SPEC-TEXT-014`。

### SPEC-PLAT-LINUX-018 AT 不播报或播报过期

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：语义变化 MUST 产生 AT-SPI 事件；节点身份 MUST 在 patch 后保持稳定；action MUST 校验当前 generation；错误提示 MUST 以 live region 语义暴露；MUST NOT 以"界面上能看到"替代播报（`SPEC-RENDER-018`）。
- **可观测症状**：Orca 焦点停在已删除行；错误文本出现但无语音；AT action 作用到同位置的新行；Flatpak 下完全读不到树。
- **来源**：`SPEC-RENDER-017/018`；`SPEC-TEXT-011`；`OPEN-LX-03`。

### SPEC-PLAT-LINUX-019 DPI 跳变与跨输出迁移

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：每次 scale 变化 MUST 同时失效布局、文字 shaping、atlas、caret 矩形，并推进 `GeometrySnapshot` epoch；MUST NOT 只改窗口尺寸而不重排文字。
- **可观测症状**：窗口从 1.0 屏拖到 1.5 屏后字号不变但留白变大；caret 偏移半个字；点击命中偏左/偏上。
- **来源**：`SPEC-RENDER-002`；`SPEC-TEXT-005`；`OPEN-LX-04`。

### SPEC-PLAT-LINUX-020 剪贴板/DnD 停滞与主线程阻塞

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：所有 fd 传输 MUST 异步并带超时；源对象生命周期 MUST 覆盖整个 offer；MUST NOT 在主线程同步读取；失败 MUST 返回 typed 错误并保持 UI 可交互。
- **可观测症状**：粘贴大图时窗口冻结数秒；拖放进行中应用崩溃；复制后在另一个应用粘贴得到空内容。
- **来源**：`SPEC-PLAT-LINUX-002/006/007`。

### SPEC-PLAT-LINUX-021 字体 fallback 漂移

- **状态**：SPEC（确定性部分 `OPEN`）
- **强度**：MUST / MUST NOT
- **要求**：字体枚举 MUST 只经 fontconfig，MUST NOT 硬编码字体路径；缺字 MUST 走显式 fallback 链并在诊断中可见；CJK 词典/字体缺失 MUST 给出明确错误；测试与基准 MUST 使用固定字体工件（`SPEC-TEXT-010`）。
- **可观测症状**：同一段文字在两台机器上换行点不同；中文显示为方块；`fc-match` 与框架实际选用的 family 不一致。
- **来源**：`SPEC-TEXT-008/010`；round1-experts（fallback 资产）。

### SPEC-PLAT-LINUX-022 合成器/显示服务器连接断开

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：Wayland 连接断开与 X11 连接断开 MUST 结束事件循环并上报致命错误；MUST NOT 静默重连、MUST NOT 留下无窗口进程；应用状态保存由应用层决定，平台层只提供事件。
- **可观测症状**：注销后进程仍在后台；SSH 转发 X11 断线后进程挂死；重新登录后出现两个实例。
- **来源**：`SPEC-PLAT-LINUX-012`；`SPEC-RENDER-013`。

### SPEC-PLAT-LINUX-023 空闲与重绘节奏

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：静态界面 MUST 等待事件与 frame callback，MUST NOT 忙轮询；动画/外部渲染请求经 deadline 驱动；合成器的 frame callback 只决定**下一帧时机**，MUST NOT 被当成"每帧必须重绘"的许可（`SPEC-RENDER-014`）。
- **可观测症状**：静止时 CPU 常驻占用、笔记本耗电明显；或相反，动画因过度合并请求而掉帧。
- **来源**：`SPEC-RENDER-014`；`SPEC-GATE-C-06`。

## 4. Gate C 最小证据清单（全部 OPEN）

> 下表是 Linux 侧进入 C Gate 的**最小**证据集。每条都必须记录：commit、lock 哈希、发行版、会话类型（Wayland/X11）、合成器、显示协议、GPU adapter 与驱动标识、命令、退出码、输出摘要。缺任一前置 → 记 `blocked`，不是 `failed`，更不是 `pass`（`SPEC-GATE-C-01`）。
>
> **编号说明**：`EVID-VAL-LX-*` 是本文的占位槽位，正式登记时由 CI 负责人统一分配 `EVID-VAL-*` 序号；槽位内容 MUST NOT 被删减。所有槽位当前 `OPEN`。

| 槽位 | 最小证据内容 | 关键前置 | 判定引用 | 状态 |
|---|---|---|---|---|
| `EVID-VAL-LX-01` | Wayland 真实窗口：xdg-shell toplevel、app_id、CSD/SSD 两种协商、resize、最小化恢复 | Wayland 会话 + 支持 xdg-decoration 的合成器 | `SPEC-GATE-C-01`、`SPEC-PLAT-LINUX-001/015` | `OPEN` |
| `EVID-VAL-LX-02` | X11 真实窗口：EWMH 属性、`ConfigureNotify` 风暴合并、无 override-redirect 正常窗口 | X11 会话 + 窗口管理器 | `SPEC-GATE-C-01`、`SPEC-PLAT-LINUX-001/015` | `OPEN` |
| `EVID-VAL-LX-03` | 输入：键盘布局运行时切换、Tab/Shift+Tab、wheel、clip 外不可命中、焦点 reveal | 真实窗口 + 可切换布局 | `SPEC-GATE-C-02`、`SPEC-PLAT-LINUX-003` | `OPEN` |
| `EVID-VAL-LX-04` | 中文 IME（Wayland）：fcitx5 或 IBus；候选 Enter 只确认、随后 Enter 只提交一次；候选窗随滚动更新 | Wayland + 输入法 + 中文词库 | `SPEC-GATE-C-02`、`SPEC-PLAT-LINUX-004/016` | `OPEN` |
| `EVID-VAL-LX-05` | 中文 IME（X11）：同一场景经 XIM；记录 preedit/周围文本能力差异 | X11 + 输入法 | `SPEC-GATE-C-02`、`SPEC-PLAT-LINUX-004/016` | `OPEN` |
| `EVID-VAL-LX-06` | AT-SPI 实测：Orca（或 accerciser）读取 Tasks/list/listitem/filter 三 toggle；action 命中当前 generation；错误播报 | session bus + AT-SPI 服务 + 屏幕阅读器 | `SPEC-GATE-C-02`、`SPEC-PLAT-LINUX-009/018` | `OPEN` |
| `EVID-VAL-LX-07` | 分数缩放与多显示器：1.0/1.25/1.5/2.0 及跨屏拖动；caret/hit/AT 同 epoch | 多输出或可切换 scale 的环境 | `SPEC-GATE-C-02`、`SPEC-PLAT-LINUX-005/019` | `OPEN` |
| `EVID-VAL-LX-08` | 剪贴板与 DnD：文本、图片、URI 列表；Flatpak 内 portal 文件选择与拖入 | 双应用 + 沙盒运行环境 | `SPEC-GATE-C-01`、`SPEC-PLAT-LINUX-006/007` | `OPEN` |
| `EVID-VAL-LX-09` | 打包产物：deb 或 rpm + AppImage + Flatpak；ICU 词典/字体/licenses/desktop 文件；最老目标发行版启动 | 构建矩阵 + 老发行版容器/机器 | `SPEC-GATE-C-04/05`、`SPEC-PLAT-LINUX-010` | `OPEN` |
| `EVID-VAL-LX-10` | GPU 与恢复：Vulkan 主路径 + GL 回退 + 显式软件回退；挂起恢复与 device-loss 注入；静止 30–60s 无重绘 | 真实 GPU + 可挂起会话 | `SPEC-GATE-C-03/06`、`SPEC-PLAT-LINUX-011/014/023` | `OPEN` |

## 5. 不确定项（OPEN）

以下事实**未验证**，不得在任何文档中被表述为已成立；每条给出可执行的验证路径。

| ID | 不确定项 | 为什么现在不能算通过 | 验证路径 |
|---|---|---|---|
| `OPEN-LX-01` | pinned 窗口栈在 Wayland 上对 `text-input-v3` 的覆盖度（preedit 样式、周围文本、`delete_surrounding`） | 没有真实 IME trace；协议支持不等于全功能 | `WAYLAND_DEBUG=1` 抓取协议往返 + fcitx5 诊断输出，逐事件对照编辑器命令 |
| `OPEN-LX-02` | X11 XIM 在 fcitx5/IBus 下的可用性、preedit 行为与候选窗定位 | XIM 是遗留路径，行为依输入法实现而异 | X11 会话下同一 Todo 场景录屏 + 输入法日志；对比 Wayland 结果并记录差异 |
| `OPEN-LX-03` | AccessKit 的 Linux AT-SPI 适配路径是否与 pinned `accesskit`/`accesskit_winit` 组合可编译、可运行 | `SPEC-ROUTE-003` 的三元组合从未共同编译运行，且 Unix 适配器不在该组合内 | 最小窗口 + 语义树冒烟；编译日志与 Orca 读取记录，登记为 `EVID-VAL-*` |
| `OPEN-LX-04` | 分数缩放下逻辑尺寸、缓冲区整数 scale 与 wgpu surface 尺寸的精确关系 | 只有 1.0 环境经验；1.25/1.5 的取整与 viewport 关系未实测 | 1.25/1.5 输出上量取 surface 物理尺寸、逻辑尺寸与 caret 位置 |
| `OPEN-LX-05` | compose / dead key 是否由 pinned 输入栈完成组合 | 窗口库可能只交付键事件，不代为组合 | 两种会话下输入死键序列与 `é`，抓取实际 `text` 事件 |
| `OPEN-LX-06` | Wayland 剪贴板在源窗口失焦/退出后的行为，以及 clipboard manager 的普遍程度 | 协议本身不保证持久化；桌面环境差异大 | 双应用实验：复制→失焦→退出→粘贴；记录各桌面环境结果 |
| `OPEN-LX-07` | NVIDIA 专有驱动 + Wayland 下的显式同步/撕裂现状 | 驱动与合成器组合差异大，无真机记录 | 真机观察 + 驱动日志 + GPU capture；结论按驱动/合成器分列 |
| `OPEN-LX-08` | XWayland 下实际选择的图形后端与 surface 扩展 | 取决于环境变量与驱动，不能由会话类型推断 | 在 XWayland 会话记录 adapter 与 surface 扩展，不套用原生 X11 结论 |
| `OPEN-LX-09` | device loss 在 amdgpu / NVIDIA 上的可恢复性、挂起恢复后的资源重建 | 未构造设备丢失场景（`OPEN-RENDER-02`） | 挂起恢复 + 驱动复位注入；记录重建前后资源计数与首帧 |
| `OPEN-LX-10` | Flatpak 下 AT-SPI 与 portal 文件对话框的实际权限组合 | 沙盒权限与桌面环境版本相关，未冒烟 | 沙盒内跑 Todo：文件选择、拖入、Orca 读取；记录权限清单 |
| `OPEN-LX-11` | mutter / KWin / sway 的 frame callback 节奏与重绘合并差异 | 三合成器均未实测 | 三环境同一场景采样帧间隔与 CPU 占用 |
| `OPEN-LX-12` | 系统托盘/全局菜单是否存在可接受的跨桌面方案 | GNOME 默认无宿主，方案依桌面环境 | 能力调研 + 最小原型；结论只能是能力位，不得默认承诺 |
| `OPEN-LX-13` | 混合显卡（PRIME）下的 adapter 选择与跨 GPU 呈现成本 | 无双显卡机器记录 | 双显卡机记录 adapter、帧时间与拷贝路径 |
| `OPEN-LX-14` | glibc 基线对应的最老目标发行版，以及 AppImage/Flatpak 的实际兼容边界 | 未做老发行版启动矩阵 | 在候选最老发行版容器/机器启动三种产物，记录符号版本错误 |

**收敛规则**：同 `SPEC-TEXT` §4 与 `SPEC-RENDER` §8——只有 evidence 层出现可复跑的 `EVID-VAL-*`（命令 + 输出 + 主机 + 会话类型）才能把 `OPEN` 改为 `VERIFIED`；仅"代码改了一行"、别的平台是绿的、或某次手工观察，都不构成关闭依据。
