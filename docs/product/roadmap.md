---
doc-id: PRODUCT-ROADMAP
layer: product
status: draft
date: 2026-11-18
owner: 框架产品经理（技术型，10 年+）
sources:
  - archive/review/FINAL.md
  - archive/review/README.md
  - archive/research/validation-and-roadmap.md
replaces: []
---

# Zenui 产品路线图

> 路线以 **Gate 闭合**驱动，不以日期驱动：**不承诺发布日期**，只承诺顺序与准入条件。
> Gate 的验收判据唯一出处是 [`../spec/08-gates.md`](../spec/08-gates.md)（`SPEC-GATE-*`）；本文只描述**版本切分、对外承诺强度与不承诺项**，不重复判据。
> 状态词只用 `SPEC / PROBE / VERIFIED / WITHDRAWN / OPEN`。

## 0. 排序原则

1. **解锁场景 × 实现成本 × 对 API 稳定性的伤害**，依次比较。
2. 先闭合「最小可证明闭环」（A0），再做「真实窗口垂直切片」（A1），再做基线（B），再做三平台（C），最后 3D 与性能定稿（D）。
3. 任何会迫使公开 API 提前定形的功能，延后到其依赖的 Gate 通过之后；**宁可少发一个能力，不提前冻结签名**。
4. 每个里程碑只服务一个首要用户层（`SPEC-SCOPE-003`）。

## 1. 里程碑总览

| 里程碑 | 对应 Gate | 首要用户层 | 一句话交付 | 对外可依赖程度 |
|---|---|---|---|---|
| **M0 · Core** | A0 | 框架自身（无用户） | 纯 Rust 的 checked core：Owner/generation、field/query、事务、headless Todo reducer trace | 无（不发布 crate） |
| **M1 · First window** | A1 | 独立开发者 | Windows 上真实 winit 窗口跑通 locked Todo | `experimental`，仅 demo 可信 |
| **M2 · Baseline** | B | 独立开发者 | 布局/文本/样式差分基线与能力门 | `experimental`，样式子集可试 |
| **M3 · Truly cross-platform** | C | 企业内部工具 | Windows/macOS/Linux 真实窗口 + IME + 真实 AT | `0.x` 首个可发布候选 |
| **M4 · 3D & performance** | D | 商业桌面应用 | 最小 glTF 视口 + 性能定稿 | `1.0` 候选 |

> 顺序不可交换。M1 的 Windows 成功**不能**替代 M3；M0 未闭合前不得开始 M1 的宏 lowering 扩展。

## 2. 各里程碑的范围与出口

### M0 · Core（A0）

- **交付**：规划中的 `crates/core` + `crates/app`；generational field handle、Store row、Query/RowSet、嵌套事务与 staging journal、FocusRemovalIntent；`todolist_headless` trace。
- **不包含**：proc macro、winit、GPU。
- **出口**：`SPEC-GATE-A0` 全项通过；嵌套 batch 的中间值泄漏反例转绿。
- **对外承诺**：无。不发布、不写 user-facing 文档。
- **风险**：现有 PoC 是平坦 sink 图，`detach` 会遗留空 bucket；重写成本集中在事务与代数（`OPEN`）。

### M1 · First window（A1）

- **交付**：`#[component]` + `view!` 的 golden IR 与 `MountResult<MountPlan>`；typed `For`/`Show`/listeners/`TextInput` epoch adapter；Windows locked Todo 窗口。
- **前置**：`SPEC-ROUTE-003`（winit/accesskit 三元组合冒烟）必须先有 `EVID-*`。
- **出口**：`SPEC-GATE-A1` 全项通过；semantics dump + 最少输入 trace 可重复。
- **对外承诺**：`experimental`。macOS/Linux 一律不宣称；Windows 也只宣称「demo 可运行」，不宣称框架可用。

### M2 · Baseline（B）

- **交付**：修复 baseline 丢失、intrinsic、Font synthesis 转发；StyleIR 固定版本 N/D/R/X manifest；`GeometrySnapshot` 共享；首轮性能工作负载采集。
- **出口**：`SPEC-GATE-B` 全项通过；Tailwind 覆盖率只以 manifest 差异测试的分子/分母形式发布（`SPEC-SCOPE-020`）。
- **对外承诺**：`experimental`；样式子集可被早期用户试用，组件与主题 API 仍可变。

### M3 · Truly cross-platform（C）

- **交付**：同一 locked Todo 在 Windows（中文 IME + NVDA）、macOS（VoiceOver）、Linux（Wayland 与 X11 分开 + AT-SPI）真实执行；`cargo test --workspace --locked` / clippy / package 门禁。
- **出口**：`SPEC-GATE-C` 全项通过，任一平台缺 runner 即 `blocked`，不得由其他平台推导。
- **对外承诺**：首个 **0.x 公开发布**。此时才允许说「三平台可用」，且必须附能力矩阵的已知差异。

### M4 · 3D & performance（D）→ 1.0

- **交付**：最小合法 glTF（mesh + 自有 depth 遮挡 + 2D overlay）→ 渐进 PBR/光照/动画；固定 benchmark 元数据与真实 baseline。
- **出口**：`SPEC-GATE-D` 全项通过。
- **对外承诺**：满足 §5 全部条件后发 **1.0**。

## 3. 对外承诺强度（按阶段）

| 阶段 | 版本号 | 承诺 |
|---|---|---|
| 现在 | 无 crate | 不承诺任何东西；文档只描述目标与探针证据 |
| M1–M2 | 无公开发布 / 内部 tag | `experimental` 语义：可随时改形或删除，不承诺 semver，不承诺平台 |
| M3 | `0.x` | 0.x 破坏性变更规则生效（见 [`positioning.md` §4](positioning.md)）；发布说明必须列出 breaking 条目 |
| M4 之后 | `1.0` | semver；未标 experimental 的公开 API 按 semver 承诺 |

**experimental 标记规则**：
- 进入条件：仅在某个 Gate 已有 `EVID-*` 的**该平台/该能力**上可标 experimental；跨平台未验的部分仍为 `SPEC`。
- 退出条件：对应 Gate 闭合 + 至少一个 minor 的弃用警告 + 迁移说明。
- **MUST NOT** 把 `experimental` 当作「已支持但免责」的措辞——experimental 的默认阅读是「可能随时消失」。

## 4. 明确不承诺

- **不承诺日期**：没有 release date，也没有「Q4 上线」。
- **不承诺性能数字**：在 M4 的固定 benchmark 之前，不出现帧时间、内存上限、启动时间目标（`SPEC-SCOPE-023`）。
- **不承诺平台覆盖**：M3 之前不出现「支持 macOS/Linux」；M3 之后也只承诺已记录的 AT/显示服务器组合。
- **不承诺 Tailwind 百分比覆盖**：只发布 manifest 生成的分子/分母（`SPEC-SCOPE-020`）。
- **不承诺 API 形状**：M1 的 `view!` 语法可以改；改之前必须有 ADR。

## 5. 1.0 准入条件（全部满足）

1. Gate A0、A1、B、C、D 全部闭合，每条有 `EVID-*` 记录与可复跑命令。
2. 三平台真实窗口 + 真实屏幕阅读器通过；能力矩阵的已知差异公开且 typed `Unsupported`。
3. 公开 API 完成一轮无 breaking 的 minor 观察期。
4. MSRV 由固定工具链 CI 宣布（当前 `OPEN`）；edition 与 lock 策略固定。
5. `cargo test --workspace --locked`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo package`、renamed-facade 测试全绿。
6. 文档：每条 `SPEC-*` 有失败模式；示例跑通率 100%；字体/ICU 资产与 licenses 随 artifact 发布。
7. 无「文档说支持、实际 Unsupported」的已知不一致。

## 6. 风险与未决项（OPEN）

| 风险 | 影响 | 当前状态 | 谁需要裁决 |
|---|---|---|---|
| MSRV 从未验证（仅研究期理论下限，见 `SPEC-ROUTE-001`） | 1.0 准入第 4 条 | `OPEN` | 队长 + CI 负责人 |
| winit/accesskit 三元组合从未一起运行 | M1 前置 | `OPEN`（`SPEC-ROUTE-003`） | 平台工程师 |
| ICU 词典数据是否随 artifact 发布 | CJK 分割在用户机器上不可用 | `OPEN` | 发布负责人 |
| Linux 无 Wayland/X11 runner | M3 直接 `blocked` | `OPEN` | CI 负责人 |
| Tailwind 官方 manifest 尚未生成 | 样式覆盖率无法宣称 | `OPEN` | 架构 + 样式实现 |
| 3D 真实资产管线缺失 | M4 无法启动 | `OPEN` | 渲染负责人 |
| 生态为零（组件库/主题） | 企业层用户无法落地 | `OPEN`（1.0 后规划） | 产品 |
