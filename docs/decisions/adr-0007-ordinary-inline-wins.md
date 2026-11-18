---
doc-id: ADR-0007
layer: decisions
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - archive/review/FINAL.md §6
  - archive/review/round2-debate.md（D4/D8 历史候选）
  - archive/review/round3-review.md
  - docs/spec/04-style.md
replaces: []
---

# ADR-0007：ordinary inline 恒胜同字段普通 class

## 背景

评审中期曾提出一条"condition 优先"的排序：`condition_rank` 先于 `origin_rank`，因此 hover/active 激活后
class 可以胜过 ordinary inline。该规则与 CSS Cascade 的 origin 规则相反，也在同一份规格里产生了两种
互相矛盾的 golden。必须裁定唯一排序。

## 备选项

1. **condition 优先（旧 D4）**：活跃条件跨 origin 胜出。
2. **ordinary inline 恒胜**（选中）：同字段上，非 important 的 author inline 胜过普通 class/hover。
3. **完整 CSS cascade**：实现全部 origin/layer/specificity。
4. **按书写顺序 later-wins**：属性遍历顺序决定结果。

备选 1 会让 `style:color` 在 hover 时被 class 覆盖，违背"inline 是显式覆盖"的直觉与 CSS 事实；
备选 3 超出原生样式系统的范围；备选 4 会让宏属性顺序影响语义。

## 决定

1. **唯一排序规则**：`origin / importance` 先比较；class 内部再比较 registry 的 canonical variant/utility
   order；只有 origin、importance、canonical 都相同的项才比较 source order（`SPEC-STYLE-010`）。
2. **ordinary inline 在 hover=false 与 true 都胜过**同字段普通 class/hover 声明（`SPEC-STYLE-011`）。
3. **condition 命中只决定某条 class 声明是否 eligible**，不是排序键（`SPEC-STYLE-012`）。
4. **`!important`、layer、transition 状态在固定版本差分前不得启用**（`SPEC-STYLE-013`）。
5. **旧 D4/D8 的 condition 优先规则为 `WITHDRAWN`**（`SPEC-ROUTE-023`）。

## 依据

- `SPEC-STYLE-010`…`-014`：唯一排序规则、inline 恒胜、condition 非排序键、important/layer 冻结前禁用、
  canonical order 不依赖书写顺序。
- `SPEC-ROUTE-023`：condition 跨 origin 胜 inline 为 `WITHDRAWN`。
- `CLAIM-086`：W3C CSS Cascade `#cascade-sort` 明确 origin/importance 在状态匹配之前比较
  （URL: https://www.w3.org/TR/css-cascade-5/#cascade-sort）。
- `CLAIM-085`：固定 commit 的 Tailwind aria 变体只匹配 `"true"`，说明 variant 不是可以凭直觉扩展的排序键。
- `CLAIM-091`、`CLAIM-092`：D4 全序与旧 hover golden 均为 `WITHDRAWN`。
- `CLAIM-084`：现有 3 条 style 测试只覆盖字段映射与 hover 覆盖 base，**没有** inline-vs-class fixture；
  `EVID-VAL-12` 不能证明本决定已被实现。

## 影响与撤销条件

**影响**

- `SPEC-GATE-B-06` 是唯一 golden：同 background 上 inline 与 class+hover 对照，hover=false/true 都取 inline。
- 测试名 `hover_only_supersedes_the_matching_property` 不得被引用为"inline vs class 已通过"
  （见 `CLAIM-084` 与 `evidence/validation.md` §7 的警告）。
- 宏与 typed builder 必须产生同一排序；属性书写顺序不得改变结果。

**撤销条件（可判定）**

只有在固定上游 commit 的 CSS 差分中出现一个**反例 fixture**：同一属性、同一 origin/importance 下，
ordinary inline 与普通 class/hover 的胜出方与 `SPEC-STYLE-011` 相反；并且该 fixture 能复现两次。
实现偏好、用户投票或"inline 写在前面更自然"都不构成撤销依据。
