---
doc-id: ADR-0003
layer: decisions
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - archive/review/FINAL.md §0、§8
  - archive/research/CONCLUSION.md
  - docs/spec/06-render.md
replaces: []
---

# ADR-0003：否决 Flutter 式三树

## 背景

Flutter 的 View → Element → RenderObject 三树把"声明结构、实例身份、布局/绘制"分成三层，布局与绘制
各持一棵可独立变化的树。它解决了大量框架问题，但也意味着同一份 UI 状态存在三个身份系统与三条更新路径。
本项目已选择"单一 View 树 + 单一响应式图"（ADR-0001），必须明确是否借鉴三树。

FLUI 等参考项目证明了"精确失效 / damage / 保留前后帧失败"的价值，但这些机制不必以三树为前提。

## 备选项

1. **完整三树**：View/Element/RenderObject 各自独立更新与 diff。
2. **单树 + 布局投影 + 单一 GeometrySnapshot**（选中）：布局与绘制从同一 View 树投影，几何只发布一份快照。
3. **两树**：保留 View 树 + 独立布局树，绘制从布局树读。
4. **无保留树**：每帧从声明重新生成布局与绘制。

备选 1/3 都会产生"UI 身份"和"布局/绘制身份"的同步问题；备选 4 无法保留 focus/scroll/caret 等控件状态。

## 决定

1. **否决 Element/RenderObject 三树**（`SPEC-ROUTE-021`，`WITHDRAWN`）。
2. **布局与绘制是同一棵 View 树的投影**：`ViewId → Taffy NodeId` 是私有映射，不形成第二套可提交树。
3. **几何唯一来源**：每次提交发布一份 `GeometrySnapshot`，paint/hit/caret/AccessKit 全部从它派生
   （`SPEC-RENDER-001`、`-017`）。
4. **可借鉴的是失效/damage 机制，不是树的数量**；不得以"借鉴 FLUI"为由复制三树。

## 依据

- `SPEC-ROUTE-021`：三树路线为 `WITHDRAWN`，撤销依据是单一权威与身份域分离。
- `SPEC-ROUTE-012`：Taffy 只做数学布局，绘制记录由自有 retained pages 持有。
- `SPEC-RENDER-001`：一份 `GeometrySnapshot` 供四路消费；`SPEC-RENDER-017`：语义树与几何同源。
- `SPEC-SCOPE-013`：不做第二套 UI 权威。
- `CLAIM-039`：当前没有生产树/布局/绘制实现，本决定是目标约束而非现状描述。

## 影响与撤销条件

**影响**

- 布局层不得持有 GPU 资源，渲染层不得反向驱动布局（`spec/06-render.md` §0 责任边界表）。
- 任何"独立的 render object 树"提案必须先证明 `GeometrySnapshot` 无法满足四路同源，而不是先写实现。
- `ViewId ↔ Taffy NodeId` 映射必须私有，并在结构 mutation 时与 owner/focus 一起事务化。

**撤销条件（必须同时满足）**

1. `SPEC-GATE-B-08`（四路同源几何）连续两次失败，且 trace 证明失败原因是"单树投影无法表达布局/绘制差异"；
2. 替代三树方案给出更少的身份同步点与更低的 live resource 计数（同一冻结 workload）；
3. 新方案仍满足 `SPEC-RENDER-001/002` 的发布顺序与旧命令失效要求。

仅"Flutter 这样做"或"便于写 render object"不构成撤销依据。
