---
doc-id: SPEC-RENDER
layer: spec
status: draft
date: 2026-11-18
owner: 渲染与 GPU 抽象负责人
sources:
  - archive/review/FINAL.md
  - archive/review/VALIDATION.md
  - archive/review/text-layout-review.md
  - archive/research/runtime-and-rendering.md
  - archive/research/stack-decisions.md
replaces: []
---

# SPEC-RENDER 布局投影、绘制、合成与窗口

> 本文件定义**要建什么**，不声明已实现。事实状态归 `docs/evidence/` 的 `CLAIM-*` 与 `EVID-VAL-*`；本文件只给出探针路径，**不复述版本号**。
>
> 版本 pin 的唯一出处是 [spec/01-route.md](01-route.md)（`SPEC-ROUTE`）。本文提到“sparse-strips 生产候选线 / Classic 研究线 / CPU reference”，均不携带版本号。
>
> 状态词只使用 `SPEC / PROBE / VERIFIED / WITHDRAWN / OPEN`；spec 层不使用 `VERIFIED`。

## 0. 责任边界：谁拥有什么，谁不能碰谁

| 层 | 拥有 | 禁止 |
|---|---|---|
| **布局投影（zenui-tree / Taffy adapter）** | 私有 `NodeId ↔ ViewId` 映射、box 几何、scroll/sticky 状态、dirty 传播，并产出 `GeometrySnapshot` | 绘制、持有 GPU 资源、把 `Taffy NodeId` 暴露到业务层 |
| **zenui-render** | 自有 retained paint records（command pages）、page 缓存 key、`SceneAdapter`、`RenderCapabilities` | 拥有 View 树/焦点/编辑器状态；公开 `vello::Scene`、`TextureId`、`wgpu::SurfaceTexture` 等后端类型 |
| **SceneAdapter** | 把自有 paint record 转成具体后端场景并缓存 | 做布局、断行、命中测试；跨设备复用场景对象 |
| **Vello GPU（生产候选线）** | GPU 光栅化、共享 `wgpu::Device/Queue` 上的渲染 | present、创建 window/swapchain、拥有 UI 状态 |
| **Vello CPU（reference/fallback）** | 确定性参考输出、无 GPU 时的显式回退 | 被当作“等价 GPU 能力”而不声明成本与差异 |
| **WindowShell / wgpu surface（zenui-platform）** | surface/swapchain 生命周期、present mode、DPI、设备丢失处理 | 做 shaping、布局、绘制记录编码 |
| **AccessKit adapter** | 语义树、role/action 映射 | 自行计算几何；使用与 paint 不同的矩形源 |
| **3D 子系统（可选）** | 每个 View generation 独立的离屏 color/depth、普通 `TextureView`、glTF 资产 | 复用 Vello 内部 depth；创建第二个 window/swapchain；在同一 pass 既写又采样同一颜色纹理 |

**三条不可越界**：

1. 布局层 MUST NOT 知道任何 GPU 资源；渲染层 MUST NOT 反向驱动布局。
2. 公共 API MUST NOT 泄漏后端 identity（Vello scene、atlas、surface texture）。
3. 平台层 MUST NOT 读 paint record；渲染层 MUST NOT 调用 `surface.present()`。

## 1. 帧提交与几何快照

```text
结构/样式事务 commit
  → layout scratch 计算（Taffy 投影）
  → 发布不可变 GeometrySnapshot(epoch)
  → 组装/复用 retained paint pages（按 key）
  → SceneAdapter → Vello 场景 → GPU 光栅
  → underlay → [3D 离屏纹理 viewport] → overlay/composite
  → present
  → 仅在成功后启用 FrameState / 下一帧 deadline
```

### SPEC-RENDER-001 每次提交只发布一份 GeometrySnapshot

- **状态**：SPEC
- **强度**：MUST
- **要求**：每次结构/布局提交 MUST 发布一份不可变 `GeometrySnapshot`，至少包含：logical rect、累积 transform、scroll/sticky 偏移、完整 ancestor clip chain、paint/hit 一致的 z-order、DPI/epoch。paint、hit、caret、AccessKit action MUST 全部从这一份快照派生。
- **失败模式**：hit 用未裁剪的 Taffy rect 时，点击穿透滚动容器外不可见按钮；AccessKit 用旧几何发 action 命中错误控件。
- **来源**：FINAL §7.2；`text-layout-review.md` P2。

### SPEC-RENDER-002 几何发布与旧命令失效的顺序

- **状态**：SPEC
- **强度**：MUST
- **要求**：结构变化时，MUST 先使指向旧 `ViewId/generation` 的迟到命令失效，再同时替换 focus 状态与 `GeometrySnapshot`；MUST NOT 出现“新树 + 旧矩形”或“旧树 + 新矩形”的中间帧。
- **失败模式**：快速切换筛选时，一次点击命中刚被移除的行；AT action 落到重建后的同位置新行。
- **来源**：FINAL §7.2；`text-layout-review.md` P2 验收。

### SPEC-RENDER-003 retained paint records 由 zenui-render 自有

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：paint record 是 renderer-neutral 的保留命令页；`zenui-render` 不暴露 Vello/glifo 的 owner identity。`SceneAdapter` 可为任一 backend 缓存，但缓存对象与设备/后端 generation 绑定。
- **失败模式**：公共 API 出现 `TextureId` 后，升级 Vello 或替换 backend 会形成 ABI 级破坏；跨设备复用场景对象触发 validation error。
- **来源**：FINAL §7.2；`runtime-and-rendering.md` §5。

### SPEC-RENDER-004 command page 的缓存 key 必须完整

- **状态**：SPEC
- **强度**：MUST
- **要求**：每个 retained subtree page 的 key 至少包含：native size/style revision、继承的 text/primary content epoch、已接受的 Parley 字体/scale 版本与 accepted width、renderer 共享 device 与 backend generation（atlas/shader）、clip/draw layer、opacity/filters 的父级适用属性、viewport size/mount generation（含外部纹理）。
- **失败模式**：只按 size 缓存时，字体替换或设备重建后复用旧字形/旧 atlas，出现花屏或黑块。
- **来源**：`runtime-and-rendering.md` §5“缓存有效性”；FINAL §7.2。

### SPEC-RENDER-005 位移/纯 transform 不重 flatten，但 clip/blend 变化必须更新复合依赖

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：纯位移或 ancestor transform MUST 作为 pass-level composition change 处理，不重新 flatten 几何与命令编码；但 occlusion、clip 相交、父级 clip/blend/opacity mutation MUST 更新 composite dependency。
- **失败模式**：滚动时复用了未更新 clip 的复合页，内容越界绘制到兄弟控件上；或相反，每次滚动都全量重新编码，帧时间抖动。
- **来源**：`runtime-and-rendering.md` §5；FINAL §7.2。

### SPEC-RENDER-006 FrameState 只在成功 present 后提交

- **状态**：SPEC
- **强度**：MUST
- **要求**：只有成功 present 的帧才能成为 published frame state；失败时 MUST 保留旧 damage 供下次重试，MUST NOT 把失败帧当作已提交状态。
- **失败模式**：present 失败后仍推进状态，下一帧只重绘“新 damage”，屏幕上留下半帧旧内容。
- **来源**：`runtime-and-rendering.md` §5（FLUI ADR-0304 参照）。

## 2. 后端、能力与颜色

### SPEC-RENDER-007 后端角色固定

- **状态**：PROBE
- **强度**：MUST / MUST NOT
- **要求**：sparse-strips 的 GPU 线是生产候选；`vello_cpu` 是确定性 reference 与显式 fallback；Classic 研究线只作互操作证明（probe），MUST NOT 成为第二生产后端或公共 ABI。
- **失败模式**：把 Classic 的 alpha/合成约定默认套到 sparse-strips，半透明区域出现预乘/直通错误；或在无 GPU 时静默给空白图。
- **来源**：FINAL §7.2；`VALIDATION.md` §3；`stack-decisions.md` 版本域。

### SPEC-RENDER-008 能力探测必须是 typed Result

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：`RenderCapabilities` MUST 显式枚举 features/limits/surface formats/texture usages/alpha mode/color space/clip/mask/filter/atlas；不支持的输入 MUST 返回 `Result::Unsupported` 类错误。MUST NOT 用 wgpu validation panic、try-panic 或“画出来是空的”充当能力探测。
- **失败模式**：未支持的 mask/filter 触发 GPU validation panic，用户看到崩溃而不是降级；或静默产生错误像素却报告成功。
- **来源**：FINAL §7.2；VALIDATION §3。

### SPEC-RENDER-009 外部纹理绑定必须携带完整元数据

- **状态**：PROBE（单一普通 `TextureView` 已验；完整元数据与生命周期未验）
- **强度**：MUST
- **要求**：外部纹理进入 `TextureBindings` 时 MUST 携带 `format`、`color_space`、`alpha`（premultiplied/straight）、`extents/scale`、`device_epoch`、`committed_serial`、`owner_generation`；paint page 的 key MUST NOT 只有 `TextureId`。陈旧或重复 ID MUST NOT 被当前 surface 采样。
- **失败模式**：viewport 重载后旧纹理仍被采样，显示上一帧模型；设备丢失后复用旧 `TextureId` 造成整帧丢失。
- **来源**：FINAL §7.2；`examples/native_poc/src/gpu.rs:57-65`；`text-layout-review.md`。

### SPEC-RENDER-010 alpha 约定不得跨后端/版本共享默认

- **状态**：PROBE
- **强度**：MUST NOT
- **要求**：Classic 线的 straight-alpha overlay 与 sparse-strips 的 premultiplied 输出是不同 contract；每次绑定外部资源都必须声明 alpha 与颜色空间，MUST NOT 复用另一个后端的默认 shader/混合状态。
- **失败模式**：半透明红色在 GPU 上得到与 CPU reference 不同的 RGBA；叠加两次后偏差累积成可见色带。
- **来源**：VALIDATION §3（`[128,0,0,128]` 与 CPU 一致；Classic 为 straight alpha）；FINAL §7.2。

### SPEC-RENDER-011 颜色空间、P3/HDR 与 format 必须与能力共同验证

- **状态**：OPEN
- **强度**：SHOULD
- **要求**：sRGB/线性空间、P3/HDR、alpha mode 的映射 MUST 与后端实际 format、surface capability 共同验证；没有固定的 token → physical pixel 对照，不得宣称高保真色彩。
- **失败模式**：在 P3 显示器上颜色过饱和；HDR surface 上 SDR 内容亮度错误。
- **来源**：FINAL §6 末段/§7.2；`runtime-and-rendering.md` §5。
- **权威归属（队长裁决）**：颜色 **token / declaration 的声明形态**（sRGB 字面量、alpha 表示、`StyleIR` 字段）由 `SPEC-STYLE` 定义（[04-style.md](04-style.md)）；本条只规定**管线工作空间**（线性化、premultiply、blend、surface format 与 capability 校验）。

## 3. 窗口、surface 与设备生命周期

### SPEC-RENDER-012 WindowShell 拥有 surface，渲染层只提交

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：每个真实窗口 MUST 有独立 surface，DPI 按窗口/显示器记录；多显示器 MUST NOT 共用一个全局 scale。渲染层不创建/销毁 surface，不调用 present。
- **失败模式**：多显示器拖动窗口后内容模糊或错位；窗口关闭时渲染层仍持有 surface，造成 use-after-free/panic。
- **来源**：FINAL §7.2；`runtime-and-rendering.md` §3。

### SPEC-RENDER-013 设备丢失遵循“清理→重建→发布”

- **状态**：SPEC
- **强度**：MUST
- **要求**：device loss 时 MUST 清理该设备的 atlas、bindings 与 scene 缓存；只有重建成功后才发布新帧。旧 device/texture generation MUST NOT 继续被任意绘制路径引用。
- **失败模式**：驱动更新/挂起恢复后黑屏或崩溃；旧 atlas 被新 device 使用触发 validation error。
- **来源**：FINAL §7.2；VALIDATION §3（尚未验证）。

### SPEC-RENDER-014 空闲不渲染

- **状态**：SPEC
- **强度**：MUST
- **要求**：无事件时 MUST 使用 `ControlFlow::Wait`；静态 2D 场景 MUST NOT 每帧重绘；静态 3D 场景 MUST NOT 因存在即每帧 activate。动画/外部请求按 deadline 或显式 `request_redraw` 驱动。
- **失败模式**：空闲时 GPU 占用 100%、笔记本耗电异常；或相反，动画因过度合并请求掉帧。
- **来源**：FINAL §7.2；`runtime-and-rendering.md` §3。

## 4. 2D 语义：clip、滚动与可见性

### SPEC-RENDER-015 clip chain 与 scroll 是几何事实，不是视觉补丁

- **状态**：SPEC
- **强度**：MUST
- **要求**：clip MUST 进入 `GeometrySnapshot` 的 clip chain，paint/hit/AccessKit 共用；`overflow` 的数学效果不等于 scroll widget，ScrollState 负责 wheel/drag/keyboard/scrollbar/focus reveal/nested scroll。MUST NOT 出现“绘制裁了但命中没裁”。
- **失败模式**：滚动区域外的删除按钮仍可被点击；键盘 Tab 进入不可见控件。
- **来源**：`runtime-and-rendering.md` §4；`text-layout-review.md` P2。

### SPEC-RENDER-016 隐藏/禁用/inert 是不同语义，不得用一个 bool

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：`Hidden`（不参与 layout/paint/hit/a11y）、`visibility:hidden`（保留 layout，不绘制/不命中/不可聚焦/从 a11y 子树隐藏）、`Disabled`、`Inert`、clip 必须分别建模，各自决定布局、绘制、命中、焦点与语义树。
- **失败模式**：用 display:none 实现筛选时，输入框丢订阅与 IME；仅视觉隐身的按钮仍能通过键盘/AT 激活。
- **来源**：FINAL §1.3/§7.2；`text-layout-review.md` P2。
- **权威归属（队长裁决）**：五种状态的**生命周期**语义（何时 dispose、何时允许保留）由 `SPEC-RT` 定义（[02-runtime.md](02-runtime.md)）；本条只规定**渲染侧后果**（是否参与 layout、是否绘制、是否命中、是否暴露给语义树），不重复定义生命周期。

## 5. 无障碍

### SPEC-RENDER-017 语义树与几何快照同源

- **状态**：SPEC
- **强度**：MUST
- **要求**：AccessKit 树由 retained View 树生成，节点矩形与 action 目标 MUST 来自同一 `GeometrySnapshot`；每个 action MUST 先校验 `ViewId + generation` 仍存活且可见。selection/char range 必须经文本层 adapter 转换（见 `SPEC-TEXT-011`）。
- **失败模式**：屏幕阅读器读出已删除节点的旧矩形；对重建后的同位置节点执行错误动作。
- **来源**：FINAL §7.2；`text-layout-review.md` P2。

### SPEC-RENDER-018 真实 AT 通过只在 C Gate 成立

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：语义 dump、单元测试或间接验证 MUST NOT 被表述为“NVDA/VoiceOver/AT-SPI 已通过”；真实读屏播报只在 C Gate 的三平台实测后成立。
- **失败模式**：文档/发布说明虚报可访问性支持，实际 AT 无法导航。
- **来源**：FINAL §9 C Gate；VALIDATION §7。

## 6. 可选 3D：只承诺离屏纹理视口

### SPEC-RENDER-019 3D 的最小承诺边界

- **状态**：PROBE（仅有占位纹理/三角，非模型）
- **强度**：MUST
- **要求**：第一阶段只承诺：每个 UI generation 拥有独立离屏 `Rgba` color 与独立 `Depth32Float`；resolve 为普通 `TextureView`，携带完整元数据与 completed serial；经 2D `TextureBindings` 参与 clip/opacity/z-order 合成。
- **失败模式**：把 Vello 内部 depth 当通用 3D depth 复用时，2D 与 3D 深度交错出现不可预测遮挡；同一 pass 既写又采样造成未定义内容。
- **来源**：FINAL §7.3；`runtime-and-rendering.md` §6；`examples/native_poc/src/gpu.rs`（green placeholder）。

### SPEC-RENDER-020 3D 不得创建第二套窗口/呈现

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：3D 子系统 MUST 共享主 `wgpu::Device/Queue`，MUST NOT 创建独立 native window/swapchain/present；重负载渲染器若未来引入，只能经 GPU Resource Boundary 接入。
- **失败模式**：两个 swapchain 争用同一窗口，出现闪烁/撕裂；3D 崩溃拖垮整个 UI 呈现。
- **来源**：`runtime-and-rendering.md` §6。

### SPEC-RENDER-021 3D 资源的 generation 与迟到完成

- **状态**：SPEC
- **强度**：MUST
- **要求**：资源加载/上传 MUST 带 owner 与 generation；owner dispose 后的迟到完成 MUST 丢弃；重载/卸载 MUST 以新 owner/generation 原子替换；worker 失败 MUST 保留 last-good model 并报告错误或 `Unsupported`。
- **失败模式**：快速切换模型时旧 worker 回写覆盖新模型；已卸载 viewport 仍持有纹理造成泄漏。
- **来源**：FINAL §7.3/§9 D Gate。

### SPEC-RENDER-022 glTF 资产管线是 SPEC，不是已实现

- **状态**：SPEC
- **强度**：SHOULD
- **要求**：目标实现为：worker 解析合法 glTF 2.0 子集（buffers、indices、UV、basic material、camera），未知 extension 必须给出明确 diagnostic；PBR/法线/光照/动画逐项增量，未实现项 MUST 走 fallback 而非静默空白。**当前“绿色图片/三角形”不构成 glTF 或 PBR 已完成。**
- **失败模式**：导入成功但网格不显示；未知扩展被静默忽略，材质与源文件不一致。
- **来源**：FINAL §7.3/§9 D Gate；VALIDATION §3/§7。

### SPEC-RENDER-023 3D 拾取是 view-local 射线，不是扁平 hit map

- **状态**：SPEC
- **强度**：MUST
- **要求**：3D 输入 MUST 走 view-local 坐标 → mesh ray intersection，焦点/键盘捕获按控件语义管理；MUST NOT 用 2D flatten hit map 假装 UI 后代自动可命中 3D 内部对象。
- **失败模式**：点击模型内部对象命中到背后的 2D 控件；焦点进入不可见的 3D 内部元素。
- **来源**：`runtime-and-rendering.md` §6 末段。

### SPEC-RENDER-024 3D 的 CPU 回退成本必须显式

- **状态**：SPEC
- **强度**：SHOULD
- **要求**：CPU 路径没有等价的零拷贝外部纹理；3D MUST 提供可选占位，或明确声明昂贵的 readback 与相应帧预算。MUST NOT 把“共享纹理免费”当作跨后端承诺。
- **失败模式**：无 GPU 环境下静默回退到逐帧 readback，帧率从 60 掉到个位数且无提示。
- **来源**：`runtime-and-rendering.md` §6。

## 7. 性能与诊断证据

### SPEC-RENDER-025 渲染性能结论必须可复现

- **状态**：SPEC
- **强度**：MUST
- **要求**：任何帧时间/带宽/draw call 结论 MUST 附：固定 workload、commit、工具链、OS/driver/硬件、warmup 与样本分布（p50/p95/p99），以及 GPU timestamp 或 capture（RenderDoc/PIX/Xcode GPU Capture）。单次 readback 只证明当次输出，MUST NOT 被当作 retained paint cache 或端到端性能证明。
- **失败模式**：报告“使用缓存后 2ms”却无 capture 与 workload，后续优化方向建立在错误基线上。
- **来源**：FINAL §9 B/C 段；VALIDATION §7。

## 8. OPEN 缺陷与未验能力台账

| ID | 未决项 | 当前证据 | 为什么不能算通过 |
|---|---|---|---|
| **OPEN-RENDER-01** | 真实 winit 窗口 / surface / resize / multi-monitor DPI | VALIDATION §3/§7 | 只有 headless RGBA target；没有真实 swapchain |
| **OPEN-RENDER-02** | device loss 恢复与 atlas 重建 | FINAL §7.2；VALIDATION §3 | 未构造设备丢失场景 |
| **OPEN-RENDER-03** | macOS/Metal、Linux X11/Wayland 后端 | FINAL §9 C Gate | 全部未验；原始证据来自 Windows/NVIDIA/Vulkan，且本轮主机不同，不得沿用 |
| **OPEN-RENDER-04** | mask/filter/layer clip 能力与 sparse-strips 已知风险 | VALIDATION §3；FINAL §7.2 | 官方标注存在 mask/complex filter 风险，需 capability gate + 降级测试 |
| **OPEN-RENDER-05** | 字形/纹理 atlas cache 性能 | `examples/native_poc/src/text.rs`（`.atlas_cache(false)`） | 缓存被显式关闭，命中率与帧时间均未测 |
| **OPEN-RENDER-06** | 生产 retained paint cache 与 damage 增量 | VALIDATION §3 | readback 只证明当次像素，不证明跨帧缓存复用 |
| **OPEN-RENDER-07** | 真实 glTF / 独立 3D depth 遮挡 / 多 viewport | VALIDATION §7；FINAL §9 D Gate | 现有绿色占位不是模型，也没有 depth 遮挡断言 |
| **OPEN-RENDER-08** | P3/HDR/颜色空间映射 | FINAL §6 末段 | 无固定 token → physical pixel 对照 |

**收敛规则**：同 `SPEC-TEXT` §4——只有 evidence 层给出可复跑的 `EVID-VAL-*`（含主机与命令）才能把 `OPEN` 改为 `VERIFIED`。跑在其他后端/主机上的旧结果不得冒充本轮证据。

## 9. 与其他规范的关系

- 版本 pin 与后端取舍：只引用 [SPEC-ROUTE](01-route.md)，本文不重复。
- 文本 shaping/缓存/baseline：见 [spec/05-text.md](05-text.md)（`SPEC-TEXT-*`），其中 `OPEN-TEXT-01/02/03` 是本文 `SPEC-RENDER-001/004` 的上游阻塞项。
- 样式到绘制字段的投影：见 `SPEC-STYLE`（[spec/04-style.md](04-style.md)）。
- Gate 归属：B Gate 负责 layout/text/style 基线，C Gate 负责三平台真实窗口与 AT，D Gate 负责 3D；见 [spec/08-gates.md](08-gates.md)（`SPEC-GATE-*`）。
