---
doc-id: ADR-0001
layer: decisions
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - archive/review/FINAL.md §0、§1、§8
  - archive/research/CONCLUSION.md
  - docs/spec/01-route.md
  - docs/spec/02-runtime.md
replaces: []
---

# ADR-0001：单一 View 树 + 单一响应式图 + 一次性 MountPlan

## 背景

项目要同时满足三件事：组件只在 mount 时执行一次、字段更新只影响依赖它的局部、结构更新有独立生命周期。
2026-11-18 的评审把两套并列叙述收敛为一个目标，但当时的探针仍只有平坦 `source → sink` 图与可独立
`fire/evaluate` 的 inspection `Node`。必须先裁定"状态到界面"的默认路径是否只有一条。

已确认的事实：生产 `Signal/Store/Memo/Owner`、事务 journal 与 `MountPlan` 都尚未实现（`CLAIM-039`）；
现有 PoC 只有平坦依赖图（`CLAIM-032`），嵌套 `batch` 仍会泄露中间值（`CLAIM-035`）。因此本 ADR 记录的是
**目标架构的裁决**，不是"已经实现"的声明。

## 备选项

1. **保留树 + 细粒度图 + 一次性 plan**（选中）：一棵 retained native View 树持有身份与控件状态，一张图负责
   脏传播与 patch，组件 mount 一次返回 plan。
2. **VDOM / Elm 全量重建**：每次状态变化重跑 `view()` 并 diff 整棵树（见 ADR-0002）。
3. **Flutter 式 Element / RenderObject 三树**：布局与绘制各持一棵树（见 ADR-0003）。
4. **Store 作为第二刷新权威**：模型变化直接触发整树渲染，图只做辅助。
5. **保留两套路径**：静态树 + 响应式 sink 并存，按场景选择。

备选 2–4 都会产生第二条可独立提交的 UI 权威；备选 5 在评审中被明确拒绝，因为"按场景选择"无法给出
统一的错误边界与 generation 校验。

## 决定

1. **唯一路径**：retained View 树（`ViewId + generation` 与控件状态）+ 单一响应式图（Signal/Memo/field-query）
   + 组件 mount 一次产出的 `MountResult<MountPlan>`。
2. **提交边界**：结构/字段 patch 只在事务 commit 时创建或修改 View；plan 本身不是可运行的 UI 树。
3. **禁止第二权威**：VDOM reconcile、组件重跑、Vello Scene、Store 刷新都不得成为可独立提交的 UI 权威。
4. **inspection Node 不升级**：`macro_poc::Node` 只是探针记录，改名不能变成生产 `MountPlan`。
5. **每个资源域独立身份**：`ViewId` / `OwnerId` / `SignalId` / `Taffy NodeId` / `ResourceId` 不共用一个裸整数。

## 依据

- `SPEC-ROUTE-010`：单一 View 树 + 单一响应式图 + 一次性 MountPlan 是唯一路线。
- `SPEC-RT-001`：单一 UI 权威；`SPEC-RT-010`…`-012`：mount 一次、可失败、plan 的提交边界。
- `SPEC-SCOPE-013`：不做第二套 UI 权威。
- `CLAIM-039`：生产 runtime/plan 尚未实现，因此这是目标约束，不能引用为完成状态。
- `CLAIM-032` / `CLAIM-035`：现 PoC 是平坦图，嵌套 `batch` 有中间值反例，说明现状不满足本决定。
- `CLAIM-089`、`CLAIM-096`：`impl View` 与"改名 inspection Node"两条旧路均为 `WITHDRAWN`。
- `EVID-VAL-07`/`-09`/`-10`：只证明普通 Rust 类型形状；`EVID-VAL-08` 是嵌套 batch 的反例，不是通过证据。

## 影响与撤销条件

**影响**

- 所有实现与 spec 必须引用 `SPEC-ROUTE-010`；任何新增组件/列表机制都要先回答"是否引入第二权威"。
- A0 的 `SPEC-GATE-A0-04`（事务原子性）与 `-06`（staging/swap）是本决定的第一道可执行检查。
- 宏、树、Store、渲染的公开 API 不得暴露可独立 `evaluate/commit` 的中间对象。

**撤销条件（必须同时满足才可重开）**

1. A0 trace 显示同一字段更新确实产生两条独立发布路径，且定位后确认是"单树 + 单图"的结构性限制，而不是单点实现缺陷；
2. `SPEC-GATE-A0-11` 的 1000+ mount/unmount/filter churn 连续两次无法回到 live 基线；
3. 替代方案在冻结 workload 上给出更少的 setup/publish 次数与完整 trace，而不是只给架构描述。

任何一条不成立，本决定保持；性能偏好或代码风格不构成撤销依据。
