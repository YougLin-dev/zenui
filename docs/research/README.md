# Zenui 技术研究与架构总纲

日期：2026-11-18。状态：研究、架构候选方案与若干**最小连通验证**；不是生产框架，也不是“Tailwind 已高度兼容”的声明。

## 快速阅读

**当前唯一推荐架构是 [统一规格 FINAL.md](../review/FINAL.md)，真实执行记录是 [VALIDATION.md](../review/VALIDATION.md)；先从 [本轮评审入口](../review/README.md) 阅读。**

本目录保留前期研究、技术依据及过程方案，不与 FINAL 竞争。早期的 `class:if-*`、`class:if-selected`、condition 胜过 ordinary inline、`impl View` mount、某些 Memo&lt;Vec&gt; 解释等均已被撤销或修订。这里的代码形状和支持表不得当成当前 SDK 或完成状态；具体以 FINAL 的 MountResult、事务、StyleIR、Gate A0–D 为准。

先看 **[研究结论（简明入口）](CONCLUSION.md)** 了解初始技术取舍，再看实际命令与阶段候选：[validation-and-roadmap.md](validation-and-roadmap.md)。其中过时语法仅用于追溯决策。

## 一句话方向

做一个原生桌面 UI 框架：**组件执行一次；信号更新绑定的值；结构更新有独立生命周期；节点稳定保留；样式在构建期编译为原生声明；Taffy/Parley/Vello/wgpu 在清晰边界内协作；2D 是主干，3D 是可选布局叶节点。**

## 1. 当前结论

| 子系统 | 推荐 | 状态与边界 |
|---|---|---|
| 响应式 | Scoped Signal / Memo / RenderEffect + `Owner` 生命周期 | 设计选择，需用卸载/循环/错位更新压力测试确认 |
| UI 权威数据 | 一棵原生保留节点树，独立 reactive graph | 不引入 VDOM，不把组件执行当状态更新模型 |
| 语法 | `view!`：HTML 外形、Rust 表达式、PascalCase typed component | 实际 rstml→StyleRule/binding proc macro probe 已通过；完整 widget/component 仍是方向草案 |
| 样式 | 固定 Tailwind v4 子集的 literal class → native StyleIR | 不运行浏览器 CSS；逐项支持表见 `tailwind-coverage.md` |
| 布局 | `taffy = 0.15.1` 与我们的 Adapter | Block/Flex/Grid 数学布局，不是完整 CSS/DOM |
| 文本 | `parley = 0.13.0` / Fontique / HarfRust / Skrifa | shaping layout ≠ IME、交互、可访问性的完整文本框 |
| GPU | `wgpu = 30.0.2` 同 Device/Queue | 类型版本必须统一；一类 GPU/操作系统不能替三平台背书 |
| 2D | 生产候选 `vello_gpu = 0.4.0` + `vello_cpu = 0.4.0`；`glifo = 0.5.0` | Vello Classic 0.12 的 PoC 仅证明连通，不自动替代产品路径 |
| 窗口/无障碍 | `winit = 0.30.13`、`accesskit = 0.25.2`、`accesskit_winit = 0.34.2` | 0.31 beta 不作为默认；需要真机和屏幕阅读器验证 |
| 可选 3D | `gltf = 1.4.1`、`glam`，独立模块、共享 GPU | 首阶段 glTF 模型 viewport，不把 UI 树改造成游戏引擎 |

MSRV 只能经固定工具链/CI 验证后宣布；研究计划可将 Rust 1.89 作为当前最低底层库需求的理论下限，但本机运行版本为 1.99.1，故**目前不声称已经验证 1.89**。

上述所列生态功能来自对应官方文档与源码。推荐阅读顺序：

1. `stack-decisions.md`：参考项目取长补短与版本兼容矩阵。
2. `native-macro.md`：宏、绑定与组件语法。
3. `style-system.md`：Tailwind 不再是 CSS 字符串解释器。
4. `runtime-and-rendering.md`：生命周期、传递数据与 GPU 合成。
5. `tailwind-coverage.md`：明确的支持等级和目标边界。
6. `validation-and-roadmap.md`：验证命令、已通过部分与尚待验证部分。

我刻意选择了“极简、原生的层次分离”，而非完整浏览器或 Flutter 移植。最终落地仍需通过垂直切片验收。

## 2. 为什么不是别的路线

- **不是 WebView / Electron / 浏览器壳**：不支持 web/mobile，不在运行时放 HTML/CSS/JS。
- **不是 elm-esque `view()` 全量重建**：llimphi 的 Elm loop 适合研究交互/合成，但不是我们的更新粒度。
- **不是 Flutter 的 Element/RenderObject 三树叠加**：FLUI 可借其受控布局和保留 patch/damage 经验。
- **不只把 Solid 闭包包装为 `Rc<RefCell>`**：GC 不存在，Owner 与订阅是显式、可释放的内部图。
- **不直接运行 Tailwind CSS**：Taffy 不是 CSS 引擎；utility 的生成不等同于 native 语义已经完成。
- **不把 Vello Scene 当永久 UI 树**：它只是绘制编码；焦点、滚动、命中测试、文本编辑和无障碍需要保留对象。
- **不在 UI 运行时塞入 Tokio/Bevy/QuickJS**：它们是可选宿主/第三方工具决策，不是桌面 UI 必要依赖。

## 3. 从参考项目真正带走的东西

| 参考 | 采用思想 | 不承担/不复制 |
|---|---|---|
| deltaui 0.2.1 | run-once、字段绑定、per-node 缓存、typed styles、Store/For 等实践 | 函数链式 `ui!` 本身；激进版本升级、预发布 fork 的无意识依赖、未验证 macOS |
| FLUI | 精确失效 patch/damage、UI 职责拆解、typed child arity、保留前后帧失败的可追踪状态 | 整套 View→Element→RenderObject 与 View 重建并不是单一响应式权威 |
| llimphi | 同 wgpu 内嵌 3D、独立 GPU pass 与 2D over-layer 的存在证明 | Elm 更新模型；M0 voxel/triangle 功能不冒充完整 glTF 资产管线 |
| SolidJS / Leptos | 静态/动态边分离、Owner disposal、For/Index 区别、薄宏→typed code | DOM clone、SSR/hydration、JS GC 与浏览器调度假设 |
| GPUI | request-layout/prepaint/paint 的复杂节点 escape hatch | 将 Entity/notify 作为第二套默认 UI 数据流 |
| Slint | DSL 编译成 native code、文档与诊断优先 | 其具体许可证下的源代码移植与另一个脚本生态 |
| Dioxus/Blitz | native CSS 研究、TextInput/文本布局方向、typed props | VDOM render/compare 不是我们要的更新模型 |
| Masonry/Floem | 保留树与明确失效 pass，细粒度节点更新 | 另一个第三方 UI runtime 或额外 reconciler 依赖 |

Sources：参考项目 [deltaui](https://docs.rs/deltaui/0.2.1/deltaui/)、[FLUI](https://github.com/vanyastaff/flui)、[llimphi](https://docs.rs/llimphi/0.1.1/llimphi/)、[Solid lifecycle](https://docs.solidjs.com/reference/lifecycle/on-cleanup)、[Leptos architecture](https://github.com/leptos-rs/leptos/blob/main/ARCHITECTURE.md)。各组件/限制链接散见具体文件。

## 4. 三条不可违反的底线

### 4.1 只有一条状态到界面的默认路径

事件/异步产物更新 signal；订阅计算只更新自己的节点/局部样式/结构。不得再引入默认的全量 `view()` reconcile 作为同一状态的第二条刷新通道。应用可以将数据组织成 Entity、Store、模块或状态机，但到 UI 树的落地路径是唯一且可观测的。

### 4.2 细粒度是工作范围，不绑死复杂度为 O(1)

某个字段更新只需局部 patch；但长度改变导致的重新换行、flex/grid 影响兄弟/祖先，仍是规定的真实布局成本。per-node Scene 缓存只节省编码，不意味着 GPU 自动增量光栅化。更新信号不等于跳过依赖链之外的一切工作。

### 4.3 平铺分解不等于伪浏览器

<view>、class、Flex/Grid 带来熟悉感，但不做 `<div>` DOM、CSS 全局 selector 和浏览器脚本。`class="flex bg-zinc-900 hover:bg-zinc-800"` 是普通静态样式描述，运行时为原生数值、枚举及状态条件。具备原生等价物才支持；浏览器专属能力返回有意义的错误而非静默代替。
