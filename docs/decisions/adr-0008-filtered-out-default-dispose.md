---
doc-id: ADR-0008
layer: decisions
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - archive/review/FINAL.md §1.3、§8
  - archive/review/round1-experts.md（X1 保留派）
  - archive/review/round2-debate.md（D6）
  - docs/spec/02-runtime.md
replaces: []
---

# ADR-0008：filtered-out 默认 dispose，保留 row model

## 背景

列表筛选会让某些行离开可见集合。两种默认行为各有代价：

- **保留 UI owner / retained-visibility**：编辑状态（draft/caret/IME）不会丢，但隐藏行的节点、订阅、布局
  缓存会无界增长，且隐藏与"真正删除"的语义容易混淆。
- **dispose UI owner**：释放节点与订阅，代价是局部编辑状态丢失；但 row model 仍在 Store 中，重新出现时
  是新 UI generation。

评审中 X1 主张默认保留，理由是反复筛选会丢失草稿并重复 mount/layout。最终裁决为"默认 dispose +
显式 opt-in 恢复"，因为无限保留会把偶发缓存变成所有权合同。必须把这条裁决连同撤销条件记录下来。

## 备选项

1. **默认 retained-visibility / hide-store**：UI owner 保留，隐藏即不参与布局/命中/语义。
2. **默认 dispose UI owner + 保留 row model**（选中）；恢复走显式 `retained_visibility` 或
   `dispose_and_snapshot(key, TTL)`。
3. **filtered 等同数据删除**：同时销毁 row model。
4. **保留整个子树但跳过绘制**：类似 `display:none`。

备选 1/4 无法给出有界容量与"真删除"清理规则；备选 3 会让筛选变成破坏性操作。

## 决定

1. **reorder 保留 identity**：`For` 重排保留 `ViewId`、generation、`Owner`、订阅与局部 caret/focus
   （`SPEC-RT-024`）。
2. **filtered-out 默认 dispose**：key 离开可见集合时销毁 UI owner/订阅/原生资源，但 **row model 不归 For
   所有**，仍在 Store 中（`SPEC-RT-024`、`SPEC-ROUTE-026`）。
3. **再次出现是新 generation**：同 key 返场创建新 UI generation；旧 click/IME/resource ticket 永不复活。
4. **恢复必须显式**：要保留草稿/caret/IME，必须选择有界的 `retained_visibility` 或
   `dispose_and_snapshot`，并定义容量、TTL、焦点与真删除清理规则；第一 Todo 不承诺该能力。
5. **焦点单独结算**：结构 mutation 前捕获 `FocusRemovalIntent`，commit 后按同 kind next→prev→可聚焦
   input→toolbar 继任（`SPEC-RT-026`）。

## 依据

- `SPEC-RT-024`：`For` 的 keyed 生命周期；`SPEC-RT-026`：焦点结算与结构 mutation 分离。
- `SPEC-ROUTE-026`：过滤即 `display:none` / 默认 hide-store 为 `WITHDRAWN`。
- `CLAIM-049`（reorder 保留）、`CLAIM-050`（filtered dispose，SPEC 状态）、
  `CLAIM-051`（`FocusRemovalIntent`，SPEC 状态）。
- `CLAIM-095`：默认 retained-visibility 的主张为 `WITHDRAWN`。
- `CLAIM-053`：Store/Query/For 行为目前无实现或测试记录，本决定是目标而非已验证行为。

## 影响与撤销条件

**影响**

- 默认不承诺"过滤回来仍保留草稿/caret/IME"；产品文案不得暗示自动恢复。
- 任何 retained 方案必须有容量上限与真删除清理测试，不能以"性能优化"为由暗中改变生命周期。
- A0 的 `SPEC-GATE-A0-09`/`-10`、A1 的 `SPEC-GATE-A1-05`/`-06` 是本决定的验收落点。

**撤销条件（必须同时满足）**

1. A1 的真实 Todo 场景证明默认 dispose 丢失了**已提交的业务数据**（不是局部 draft/caret/IME），且无法
   通过事务或错误 sink 修复；
2. 或 C Gate 的 AT trace 显示焦点后继续任在默认 dispose 路径下不可判定，而 retained 方案可判定；
3. 替代方案在同一冻结 workload（`SPEC-GATE-B-10` / `C-07`）下给出有界的 live owner 与资源计数，
   并定义真删除清理规则。

仅"用户可能想保留草稿"或"hide 更省事"不构成撤销依据。
