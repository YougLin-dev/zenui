---
doc-id: ADR-0026
layer: decisions
status: normative
date: 2026-11-19
owner: 测试与 CI/发布负责人（10y+）
sources:
  - docs/decisions/adr-0025-a0-not-applicable-handover.md
  - docs/decisions/adr-0024-gate-close-predicate.md
  - docs/spec/08-gates.md
  - docs/evidence/validation.md
  - crates/core/src/view.rs
  - crates/core/tests/lifecycle.rs
replaces: []
---

# ADR-0026：ADR-0025「相邻证据缺口」一节的勘误

## 背景

`ADR-0025` 在「相邻证据缺口（有对象、缺夹具；不得标 `notApplicable`）」一节断言三处子句没有夹具：

1. `SPEC-GATE-A0-09` 的「reorder 保留 generation」——当时只有 ViewId/RowKey/focus/caret/owner 断言；
2. `SPEC-GATE-A0-10` 的 `prev` **结算**分支——当时的删除夹具都删有后继的行；
3. `SPEC-GATE-A0-11` 的「caches 回到基线」——churn 夹具只断言 owner/edge/source/layout/rows。

这三句在 `ADR-0025` 成文时所在分支上是准确的。PR #22（`test/MORT-6-a0-clause-closure`，`1b8f322`）随后补上
了三条夹具，断言与原始缺口逐项对应；PR #22（`ae42be9`）先合入 main，PR #23（`6182928`）再把 `ADR-0025`
并入，因此**在两个 PR 都合入之后，该节已与树不一致**。`EVID-VAL-49` 是这三条夹具第一次进入全量计数的记录。
需要一条可追溯的勘误，而不是让读者对着两处矛盾自行判断。

`docs/decisions/index.md` §5 规定 ADR 正文不就地修订。本条因此是一条独立的勘误记录：它**不改变**
`ADR-0025` 的任何决定（七条 `notApplicable` 移交、撤销条件、边界全部保持），只更正它对「是否存在夹具」
这一事实的陈述。

## 备选项

1. **直接改 `ADR-0025` 正文**：否决。`index.md` §5 禁止就地修订；改了以后历史审阅者无法分辨当时的判断与
   后来的事实。
2. **当作没发生，等读者自己发现**：否决。这一节被 `ADR-0025` 的撤销条件与 gate owner 直接引用；留着一句
   已被证伪的话，会让 `closed` 的映射看起来仍然缺三条。
3. **把三条夹具写进新的 gate 判据**：否决。判据已经要求这三件事，缺的从来不是要求而是夹具；新增判据等于
   重复定义。
4. **单开一条勘误 ADR，逐条给出 old → new 与证据（选中）**：最小改动、可机械核对，且不动任何决定。

## 决定

1. **三处缺口已由夹具关闭，映射如下。**

   | `ADR-0025` 缺口 | 夹具 | 位置 |
   |---|---|---|
   | `SPEC-GATE-A0-09` reorder 保留 generation | `a_reorder_preserves_every_generation` | `crates/core/src/view.rs:329` |
   | `SPEC-GATE-A0-10` `prev` 结算分支 | `a0_10_deleting_the_last_focused_row_falls_back_to_previous` | `crates/core/tests/lifecycle.rs:230` |
   | `SPEC-GATE-A0-11` caches 回到基线 | `a0_11_the_cache_returns_to_the_baseline_after_churn` | `crates/core/tests/lifecycle.rs:624` |

2. **这三项不再构成「未映射义务」。** 在 `EVID-MATRIX.criteria[]` 里，它们分别映射到上表的夹具与
   `EVID-VAL-49`；`SPEC-GATE-A0-09`/`-10`/`-11` 的结构关闭不再因此节而受阻。
3. **`notApplicable` 的范围不变。** 本节从未把这三项标为 `notApplicable`（它们有对象），勘误后同样不标；
   `ADR-0025` 的七条移交与撤销条件全部继续有效。
4. **本条不改任何判据状态、不改 spec 文本、不裁决 M0 出口。** 它与 `ADR-0024` 的分工不变：`closed` 是结构
   谓词，出口是产品判断。
5. **`ADR-0025` 的行号引用因 #22 插入而偏移，引用时以测试名为准。** `view.rs:327` 的
   `focus_intent_reports_both_neighbours` 现在是 `crates/core/src/view.rs:369`（#22 在 `:301` 之后插入了
   `a_reorder_preserves_every_generation`）；`lifecycle.rs:433-534` 所指的 bulk-clear 与 churn 夹具现在是
   `:426`（`a0_10_a_bulk_clear_settles_once_for_the_surviving_sibling`）与 `:487`
   （`a0_11_churn_returns_to_the_baseline`）。行号会继续漂移，ID 与测试名不会。

## 依据

- `ADR-0025`「相邻证据缺口」一节（该文件 `:131-139`）与被其引用的三行判据（`08-gates.md:83-85`）。
- `EVID-VAL-49`：`cargo test --workspace --locked` 在 `6182928` 上 114 passed，三条夹具均在其中。
- `SRC crates/core/src/view.rs:329`、`SRC crates/core/tests/lifecycle.rs:230`、`:624`：夹具定义处；每条
  夹具的文档注释写明「哪一处 mutation 会让它判红」。
- `1b8f322`（PR #22）：提交信息逐条说明三处缺口与对应夹具。
- `docs/decisions/index.md` §5：ADR 正文不就地修订，修改决定须新开 ADR。

## 影响与撤销条件

**影响**

- `ADR-0025` 正文不变；本条的效力只覆盖上表三行。引用「相邻证据缺口」时应同时引用本条。
- `EVID-MATRIX.criteria[]` 的 A0-09/-10/-11 义务已按上表映射；`closed` 的计算因此不再把这三项列为未映射。
- 文档层不需要改动 `08-gates.md`：判据文本本来就要求这三件事，补的是夹具，不是要求。

**撤销条件（可判定）**

1. 若上述任一夹具被删除、改名或不再断言对应子句（例如 `a_reorder_preserves_every_generation` 不再比较
   generation 映射），本条对该项的勘误立即失效，该项回到「有对象、缺夹具」，对应判据必须记 `closed: false`。
2. 若 `EVID-VAL-49` 被后续记录取代（新 run 不再包含该夹具），本条改引新条目；在引用更新前，本条不单独
   构成证据。
3. 若 `ADR-0025` 被 supersede，本条随其目标一起失效，由新 ADR 重新给出 old → new 映射。
