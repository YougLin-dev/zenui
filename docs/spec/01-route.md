---
doc-id: SPEC-ROUTE
layer: spec
status: normative
date: 2026-11-18
owner: 框架产品经理（技术型，10 年+）
sources:
  - archive/research/CONCLUSION.md
  - archive/research/stack-decisions.md
  - archive/research/README.md
  - archive/review/FINAL.md
replaces: []
---

# 01 · 唯一技术路线与版本 pin

> **本文件是全仓库唯一的版本 pin 出处。** 其他任何文档只能写 `[SPEC-ROUTE-001]` 这类引用，不得复述版本号；发现第二处版本号即为缺陷（治理契约 §7 第 4 条）。
> 每条给出：要求强度、失败模式、当前状态。状态词只用 `SPEC / PROBE / VERIFIED / WITHDRAWN / OPEN`。

## 1. 版本 pin（唯一出处）

### SPEC-ROUTE-001 生产候选依赖 pin

| 层 | Pin | 允许替换 | 状态 |
|---|---|---|---|
| 编译工具链 | edition **2024**；观察到的工具链 **1.99.1**；MSRV **OPEN**（研究期理论下限 Rust 1.89，从未在 CI 验证） | MSRV 只能由固定工具链 CI 宣布 | `OPEN` |
| Markup | `rstml 0.13.1` + `syn 3` + `quote` | 仅前端 IR | `PROBE`（parse + 小型 lowering） |
| 布局 | `taffy 0.15.1` | 通过自有 Adapter，`NodeId` 私有 | `PROBE`（测量探针，含已记录缺陷） |
| 文本 | `parley 0.13.0` + `parley_engine 0.13.0`（feature `complex-scripts`） | 不该轻易换 engine | `PROBE`（CJK shaping/ICU） |
| 字形栅格 | `glifo 0.5.0` | 通过 Scene Adapter | `PROBE`（headless） |
| GPU 类型 | `wgpu 30.0.2` | 同代 backend 必须统一 | `PROBE`（headless） |
| 2D 生产 | `vello_gpu 0.4.0`（sparse-strips 首选）+ `vello_cpu 0.4.0`（reference / fallback） | Scene Adapter 可替换实现 | `PROBE`（headless，单机） |
| 窗口 / 无障碍 | `winit 0.30.13` / `accesskit 0.25.2` / `accesskit_winit 0.34.2` | WindowShell 可替换 | `SPEC`（三元组合**从未一起编译/运行**，见 `SPEC-ROUTE-003`） |
| 可选 3D | `gltf 1.4.1` + `glam` + 自有 wgpu mesh | 3D backend 可选 | `SPEC` |
| Tailwind 语义 oracle | 官方 v4 源码 commit **ac7fd4007178123269cc6c8e03c8d957646331fa**；Rust `tailwind 0.4.0` 仅作离线 oracle | oracle 可换，生成物必须可复现 | `OPEN`（无 manifest/差异测试） |

- **MUST** 同一 workspace 内同名 crate 只有一个主版本；升级必须有 ADR。
- 失败模式：`cargo tree -d` 出现两个 `wgpu` 主版本 → 类型不互通（`Device`/`Texture` 不能跨版本共享），构建门禁 `-D warnings` 之外还必须拒绝 dependency duplication；`parley::NormalizedCoord` 与 glifo `i16` 类坐标混用时必须显式 `to_bits()` 转换，否则 silence 出错误字形。
- 可逆性：各 pin 均可替换实现，不可替换的是边界（`SPEC-ROUTE-010` 起的路线条目）。

### SPEC-ROUTE-002 Classic Vello 只是探针

`vello 0.12.0`（Classic）**仅**作为互操作证据存在，**MUST NOT** 作为第二生产后端，也**MUST NOT** 与 sparse-strips 共享默认 alpha/合成假设（Classic 观察到的 straight-alpha 与 sparse-strips 的 premultiplied 不是同一 contract）。

- 失败模式：Classic 的实验结论被直接套用到 `vello_gpu 0.4` 的半透明合成 → 像素回读不匹配；此类复用按虚假证据处理。
- 状态：`PROBE`（生产候选维度为 `WITHDRAWN`）。

### SPEC-ROUTE-003 窗口/无障碍三元组合是待验证基线

`winit` / `accesskit` / `accesskit_winit` 的版本组合**从未在一个真实窗口中共同编译运行**。研究文档中「稳定组合」的措辞描述的是版本共存性判断，不是验证结果。

- **MUST** 在 A1 前先完成一次三元组合的编译 + 最小窗口冒烟，并把结果登记为 `EVID-*`；在此之前任何文档只能写 `SPEC`。
- 失败模式：直接以「已选定」口吻写进对外文档 → 违反 `SPEC-SCOPE-021`。
- 状态：`OPEN`。

### SPEC-ROUTE-004 版本升级纪律

- 升级任一 pin **MUST** 有 ADR（背景 / 候选 / 证据 / 可逆性），并在同一 PR 内更新本表；**MUST NOT** 只改 `Cargo.toml`。
- 失败模式：`Cargo.lock` 漂移但本表未更新 → 复审拒绝。

## 2. 唯一架构路线

### SPEC-ROUTE-010 单一 View 树 + 单一响应式图 + 一次性 MountPlan

框架 **MUST** 只有一条「状态 → 界面」的默认路径：

1. 一棵 retained native View 树持有 `ViewId + generation` 与控件状态（focus/scroll/selection/composition/hit mapping）；
2. 一张细粒度响应式图（Signal/Memo/field-query）承担脏传播与 patch 计算；
3. 组件在实例 mount 时执行**一次**，产出 `MountResult<MountPlan>`，只在事务 commit 时创建 View。

- **MUST NOT** 把 VDOM reconcile、组件重跑、或 Vello Scene 当作第二条可独立提交的权威。
- 失败模式：同一字段更新出现两条刷新路径（例如结构更新绕过事务直接建节点）→ A0 事务验收必挂；trace 中会出现两次 publish 或 stale generation 被接受。
- 交接：具体形制见 `02-runtime.md`（`SPEC-RT-*`）；本文件只锁定「只有一套」这一约束。

### SPEC-ROUTE-011 细粒度是工作范围，不是 O(1) 承诺

局部 patch **不等于**全局 O(1)、局部 GPU 自动增量光栅化或每帧零成本。

- **MUST NOT** 在任何对外材料中把「细粒度更新」写成恒定的复杂度或帧时间承诺（与 `SPEC-SCOPE-023` 一致）。
- 失败模式：benchmark 章节出现未测的「O(1) 更新」表述。

### SPEC-ROUTE-012 布局 / 文本 / 绘制 / 窗口的职责边界

- Taffy 只做数学布局；Parley 只做 shaping/bidi/fallback；绘制记录由 zenui 自有 retained pages 持有；Vello 只是 Scene adapter；wgpu 是唯一 Device/Queue。
- **MUST NOT** 在公开 trait 中暴露 Taffy `NodeId`、Vello 内部 target/scene owner 或 `wgpu::SurfaceTexture` 等未封装资源类型。
- 失败模式：第三方 backend 实现被内部 ID 绑定，替换实现需改公开 API → 违反本条的 review 阻断。
- 细化：`05-text.md`、`06-render.md`。

### SPEC-ROUTE-013 错误是类型，不是 panic

能力缺失、无效 generation、事务失败 **MUST** 以 typed `Result`（含 `Unsupported`）表达；**MUST NOT** 用 wgpu validation panic 或 try-panic 探测能力。

- 失败模式：用户输入/平台差异触发 panic → 违反 A0/C 的 error sink 验收。

## 3. 已撤销路线（WITHDRAWN）

每条保留 ID 与撤销依据，避免旧文档措辞复活。实现细节分别归 `02-runtime.md`、`03-macro.md`、`04-style.md`。

| ID | 曾提出的路线 | 裁决 | 撤销依据 |
|---|---|---|---|
| SPEC-ROUTE-020 | VDOM / Elm 全量 `view()` 重建 | `WITHDRAWN` | FINAL §0/§8：单一 View 树 + 细粒度 patch 为唯一权威；全量重建会引入第二条刷新路径 |
| SPEC-ROUTE-021 | Flutter 式 Element/RenderObject 三树 | `WITHDRAWN` | FINAL §8：可借鉴 FLUI 的失效/damage，不复制三树 |
| SPEC-ROUTE-022 | `class:if-*` / `class:if-selected` 条件样式语法 | `WITHDRAWN` | FINAL §6/§8：改用 typed `aria:pressed` 同源派生；旧措辞出现在 `archive/research/CONCLUSION.md` §Tailwind 段落，归档后只作历史 |
| SPEC-ROUTE-023 | 激活后的 condition 可跨 origin 胜过 ordinary inline | `WITHDRAWN` | FINAL §6：ordinary inline 在 hover=false/true 均胜出；CSS origin 优先于状态激活 |
| SPEC-ROUTE-024 | `impl View` / 隐式 builder 的 mount 形态 | `WITHDRAWN` | FINAL §1.2：统一为 `MountResult<MountPlan>`，宏与 typed builder 产同一 golden IR |
| SPEC-ROUTE-025 | `Copy` 结构体承载 `String` 的模型行 | `WITHDRAWN` | VALIDATION §4.3：实际触发 E0204；改为 stable id + field handle，title 在 mount 时 owned 读取一次 |
| SPEC-ROUTE-026 | 过滤即 `display:none` / 默认 hide-store 保留编辑状态 | `WITHDRAWN` | FINAL §1.3：默认 dispose UI owner；要保留必须显式有界 `retained_visibility` / `dispose_and_snapshot` |
| SPEC-ROUTE-027 | Classic Vello 0.12 作为第二生产后端 | `WITHDRAWN` | 见 `SPEC-ROUTE-002` |
| SPEC-ROUTE-028 | 运行时 CSS selector 解释器 / 任意 class 字符串 | `WITHDRAWN` | FINAL §6：pinned registry 白名单 + 编译期诊断 |
| SPEC-ROUTE-029 | 默认引入 Bevy / Tokio / QuickJS / 完整 DOM+CSS | `WITHDRAWN` | 见 `SPEC-SCOPE-014` |
| SPEC-ROUTE-030 | 每个消费者各自物化 `Memo<Vec>`、或只物化不记字段边 | `WITHDRAWN` | FINAL §2：共享 query cache + 字段反向边；空态必须零 materialization |
| SPEC-ROUTE-031 | 单个裸 `u64` 同时充当 `ViewId`/`OwnerId`/`SignalId`/`NodeId` | `WITHDRAWN` | FINAL §1.1：不同资源域必须 newtype 区分 |

- 引用纪律：旧文档中的原文不改写（归档只读），但任何新文档**MUST NOT** 把上表左列写法描述为现行方案；需要提及时写「已撤销，见 `SPEC-ROUTE-02x`」。
- 失败模式：新规范条目里出现 `class:if-*` 或 `impl View` 作为 API → 直接驳回。
