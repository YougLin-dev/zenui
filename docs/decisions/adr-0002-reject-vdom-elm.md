---
doc-id: ADR-0002
layer: decisions
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - archive/review/FINAL.md §0、§8
  - archive/research/CONCLUSION.md
  - docs/spec/01-route.md
replaces: []
---

# ADR-0002：否决 VDOM / Elm 全量重建

## 背景

SolidJS 风格的核心承诺是"组件执行一次、字段更新只 patch 自己"。Elm/React 风格的 `view()` 全量重建
虽然模型简单，但每次状态变化都会重新生成整棵虚拟树并做 diff；这与"组件 setup 不重跑、局部状态（caret/
focus/scroll）由原生控件持有"的目标直接冲突。

评审时必须回答：是否允许 VDOM 作为默认刷新模型，或作为某些场景的可选路径。

## 备选项

1. **Elm 架构**：单一 `Model → view()`，每次更新全量重建。
2. **React 式 VDOM diff**：保留虚拟树，按 key 做 diff。
3. **Solid 式细粒度图 + retained 树**（选中）。
4. **混合**：默认细粒度，复杂列表退回 VDOM diff。

备选 1/2 会把组件 setup 与事件闭包重新执行变成常态；备选 4 无法给出统一的提交边界——同一棵树会出现
两种 reconcile 语义，错误传播与 generation 校验无法统一。

## 决定

1. **否决 VDOM / Elm 全量重建作为默认刷新路径**（`SPEC-ROUTE-020`，`WITHDRAWN`）。
2. **允许的唯一树 diff 是 `For` 的 keyed reconcile**：只处理增删/重排，不重新执行未变行的 setup
   （`SPEC-RT-024`）。
3. **禁止以"复杂场景"为由引入全局 reconcile**；若某个场景无法用细粒度 patch 表达，先补 spec 与失败模式，
   而不是退回 VDOM。

## 依据

- `SPEC-ROUTE-020`：VDOM / Elm 全量重建已被列为 `WITHDRAWN`，撤销依据是单一权威与组件 run-once。
- `SPEC-ROUTE-010`、`SPEC-RT-010`、`SPEC-RT-024`：唯一路径、组件 mount 一次、`For` keyed reconcile 的边界。
- `CLAIM-089`：`impl View` + 每次更新重建的旧形态已 `WITHDRAWN`。
- `CLAIM-039`：当前没有生产运行时，因此不能把"现有 PoC 能跑"当作 VDOM 可行性的证据。

## 影响与撤销条件

**影响**

- 所有组件 API 必须能表达"mount 一次 + 绑定边 + 结构 patch"；不能用"返回新的 `Vec<Node>`"作为更新接口。
- `For` 的验收必须区分 reorder（保留 identity）与增删（dispose/create），见 `SPEC-RT-024`。
- 文档与示例不得把 `view!` 描述为"每次渲染都重新执行"。

**撤销条件（必须同时满足）**

1. `SPEC-GATE-A0-11` 的 churn 压力下，细粒度路径出现无法修复的 live owner/edge 泄漏，且泄漏可在 VDOM
   路径上消失；
2. 冻结 workload 的 trace 显示 VDOM 全量重建的组件 setup 次数与帧时间都优于细粒度路径（不是只比较代码量）；
3. 新方案仍满足 `SPEC-RT-003` 的 generation 校验与 `SPEC-RT-053` 的失败回滚。

仅"写起来更简单"或"生态更熟"不构成撤销依据。
