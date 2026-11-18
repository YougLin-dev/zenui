---
doc-id: SPEC-SCOPE
layer: spec
status: normative
date: 2026-11-18
owner: 框架产品经理（技术型，10 年+）
sources:
  - archive/research/README.md
  - archive/research/tailwind-coverage.md
  - archive/review/FINAL.md
  - archive/review/VALIDATION.md
replaces: []
---

# 00 · 能力边界与非目标

> 本文件回答「建什么、为谁建、明确不建什么」。目标用户的完整画像与稳定性承诺见 `../product/positioning.md`，版本排序见 `../product/roadmap.md`，技术选型与版本 pin 见 `01-route.md`（`SPEC-ROUTE-*`）。
> 状态词只用 `SPEC / PROBE / VERIFIED / WITHDRAWN / OPEN`。凡「已实现」的表述必须指向 `EVID-*`；本文件不含任何 `VERIFIED` 声明。

## 1. 产品形态

### SPEC-SCOPE-001 原生桌面 UI 框架，不是浏览器

Zenui **MUST** 是纯 Rust 的桌面 GUI 框架：一个 library + proc macro，直接创建 OS 原生窗口并在 GPU 上合成界面。

- **MUST NOT** 在运行时嵌入 WebView、加载 HTML/CSS/JS，或以系统浏览器作为渲染后端。
- 失败模式：任何能力只能通过 WebView/CSS 引擎实现时，必须返回 typed `Unsupported` 并登记为 `OPEN`；不得静默降级成 web 渲染，也不得在文档里描述为「支持」。
- 状态：`SPEC`（框架本体不存在，规划中的 workspace 尚无 crate）。

### SPEC-SCOPE-002 平台范围＝三桌面平台，不含 web / mobile

支持目标平台是 **Windows、macOS、Linux 桌面**；Linux 的 Wayland 与 X11 **MUST** 分开记录，不得互相背书。

- **MUST NOT** 宣称 web、iOS、Android、WASM 目标。
- 失败模式：Cargo feature、CI 矩阵或对外文档中出现未支持平台的可编译承诺，视为文档缺陷，按 Gate 门禁拒绝合入。

## 2. 目标用户与首个垂直场景

目标用户三层（独立开发者 / 企业内部工具 / 商业桌面应用）的 80% 场景与「谁先被服务」的定义，**唯一出处是** [`../product/positioning.md` §2](../product/positioning.md)。本文件只约束交付纪律：

### SPEC-SCOPE-003 每个里程碑只服务一个首要用户层

- 每个版本 **MUST** 在 roadmap 中写明当期首要用户层；其余两层的需求只能进入 backlog。
- 失败模式：一个里程碑同时把三层的关键场景列为验收项 → 范围膨胀、Gate 无法闭合，roadmap 评审应直接驳回。

### SPEC-SCOPE-004 单一纵向场景作为统一验收载体

A1 与 C Gate **MUST** 使用同一个 locked Todo 应用（列表 / filter / TextInput / IME / 焦点 / 无障碍）作为验收载体，目标语法见 `examples/todolist/main.rs`（该文件是**目标预览，不可运行**）。

- 失败模式：各 Gate 用不同 demo 各自举证 → 结果不可比，C Gate 的三平台结论失去意义。

## 3. 明确不做（非目标）

以下条目均为 **MUST NOT**。「不做」指不进入框架核心、也不承诺兼容；试图支持时**必须在编译期给出诊断**，而不是运行时 no-op 或空图。

### SPEC-SCOPE-010 不做 Web 与 CSS 引擎
- 不做任意 CSS selector / pseudo-class 引擎、`@plugin`/`@config`/JS 插件、HTML `content::before`、SSR/hydration、浏览器 preflight 的盲目复制。
- 失败模式：未知 class 或浏览器独占能力被静默忽略 → 必须编译期报错并给出 span（见 `04-style.md`）。

### SPEC-SCOPE-011 不做 DOM 独占的布局与语义
- 初版不做 `<table>`/float 布局、subgrid、vertical writing mode、任意 `has()`/`peer` DOM 选择器。
- 失败模式：`.styles` 整表覆盖或「假装几何相同」——必须返回 `Unsupported` 或编译诊断。

### SPEC-SCOPE-012 不做运行时通用字符串样式
- 动态 class 只接受**编译期完整候选的 enum 或静态条件表达式**，不接受任意运行时 class 字符串。
- 失败模式：运行时拼接的 class 被接受但无样式 → 必须拒绝；`ClassId` manifest（若启用）只能引用已生成的静态 StyleProgram。

### SPEC-SCOPE-013 不做第二套 UI 权威
- **MUST NOT** 引入 VDOM/Elm 全量重建、Flutter 三树 reconciler、或把 Vello Scene / inspection Node 当成可独立提交的 UI 树。
- 失败模式：同一状态存在两条刷新路径 → 违反单一权威，架构评审拒绝；详见 `SPEC-ROUTE-020/021`。

### SPEC-SCOPE-014 不把游戏/异步运行时塞进核心
- 核心 **MUST NOT** 默认依赖 Bevy、Tokio、QuickJS 或完整 DOM/CSS 实现；3D 不是游戏引擎。
- 失败模式：核心 crate 的默认依赖树出现上述运行时 → 依赖审计拒绝。

### SPEC-SCOPE-015 3D 是布局叶节点，不是引擎
- 3D **MUST** 以「独立离屏 color/depth + 普通 `TextureView`」接入 2D 合成；glTF 只承诺合规子集；**MUST NOT** 复用 Vello 内部 depth，也**MUST NOT** 承诺任意 2D/3D depth 交插、AR 或游戏场景。
- 失败模式：把占位绿纹理说成「已加载模型」；或在同一 pass 既采样又写纹理——两者都按虚假证据处理。
- 状态：`SPEC`（真实 glTF 资产管线未实现）。

## 4. 兼容性与宣称纪律

### SPEC-SCOPE-020 Tailwind 只承诺 pinned registry 生成的支持清单
- 支持等级 `N/D/R/X` 的语义定义在 `04-style.md`；本文件只约束宣称方式：**MUST NOT** 发布没有固定官方版本 manifest 与差异测试支撑的覆盖率百分率。
- 失败模式：文档出现「支持 80% Tailwind」这类数字而无 `EVID-MATRIX` 来源 → 视为虚假宣称，删除或降级为 `SPEC`。

### SPEC-SCOPE-021 能力宣称必须能落到证据
- 任何对外「已支持 / 已验证」的表述 **MUST** 指向 `evidence/` 中的 `EVID-*` 或 `CLAIM-*` 条目。
- 失败模式：`PROBE` 级探针被写成 `VERIFIED` → 违反治理契约 §1 铁律 1，按文档缺陷处理。

### SPEC-SCOPE-022 无障碍不过度宣称
- A1 的 Windows semantics dump 与 a11y action 只是**间接**验证；**MUST NOT** 在 C Gate 的真实 NVDA/VoiceOver/AT-SPI 执行前宣称「屏幕阅读器已通过」。
- 失败模式：UI 上有可见错误文本就宣称「AT 已播报」——错误展示 ≠ announcement。

### SPEC-SCOPE-023 性能数字有门槛
- **MUST** 只报告固定 workload + 完整环境（CPU/GPU/driver/OS/电源）+ warmup + p50/p95/p99；未实测 **MUST** 写 `N/A`/`blocked`。
- **MUST NOT** 命名未经测量的帧预算，也**MUST NOT** 把单次 readback 或单像素断言当作端到端性能上限。
- 失败模式：文档出现「60fps」「<16ms」而无对应 `EVID-VAL-*` 采集记录。

### SPEC-SCOPE-024 稳定性承诺的规范约束
- 对外的 semver 承诺、破坏性变更节奏、experimental 与弃用策略，**唯一出处是** [`../product/positioning.md` §4](../product/positioning.md)；本节不重复内容，只约束：任何 release note 或文档章节的稳定性表述 **MUST** 与该节一致。
- 失败模式：发布说明承诺 semver 但 crate 处于 0.x 且无 ADR 记录破坏性变更 → 发布阻断。

## 5. 当前状态（本文件自身的证据边界）

- 本文件全部条目为 `SPEC`（要求），不含 `VERIFIED`。
- 现有可运行资产仅限探针级（`PROBE`），见 `../evidence/probes.md`（规划）与 `../evidence/validation.md`（规划）；其范围**不构成本文件任何条目的满足**。
- 生产 workspace、`crates/core`、launcher 均不存在；`archive/review/VALIDATION.md` §7 与 `archive/review/FINAL.md` §10 的记录仍然是唯一现状依据。
