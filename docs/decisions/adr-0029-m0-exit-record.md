---
doc-id: ADR-0029
layer: decisions
status: normative
date: 2026-11-19
owner: 框架产品经理（技术型，10 年+）
sources:
  - docs/decisions/adr-0027-m0-exit-policy.md
  - docs/decisions/adr-0024-gate-close-predicate.md
  - docs/decisions/adr-0025-a0-not-applicable-handover.md
  - docs/decisions/adr-0028-command-generation-handover.md
  - docs/spec/08-gates.md
  - docs/evidence/matrix.json
  - docs/evidence/validation.md
  - docs/product/roadmap.md
replaces: []
---

# ADR-0029：M0 出口记录（候选 `cbf7012a`）

## 背景

`ADR-0027` 定义了 M0 出口的三条条件（E1 结构闭合、E2 逐义务执行型证据、E3 出口时点集成状态），并要求出口成立时另开一条 ADR 作为**出口记录**：写明候选 commit 与记录 commit、被认证输入、`criteriaClosed`、逐义务见证链、E3 运行 ID，以及为空的未映射义务清单。本条就是这份记录，不重新定义条件。

候选在 `#29` 合入后固定为 `cbf7012a`：该修复解决了 `SPEC-RT-024` 要求的「重复 key 在任何 native patch 前被拒」在无序 mount 路径上不成立的问题。候选变更使此前的运行作废，`#32` 在 `cbf7012a` 上重新记录了 E3a、trace 与 E3b。本条只评估该候选。

## 备选项

1. **不出口，等待一个更新的候选**：否决。E1–E3 在冻结候选上已经成立；「等更新」没有可判定的触发条件，只会让出口不可证伪。
2. **出口但不落记录**：否决。`ADR-0027` 决定 6 要求出口裁决是一条独立 ADR；口头或汇总行结论无法审计，也无法承载逐义务见证链。
3. **把 A1 的 `command generation` 或其它后续义务纳入出口**：否决。该子句已按 `ADR-0028` 移交 A1-07，不在 A0 的判据集合内；纳入会改变定义域。
4. **按 `ADR-0027` 的决定 6 记录本次出口（选中）**：条件、输入与运行都可复核，且不引入新的裁决。

## 决定

1. **候选、记录与输入。** 候选 commit 为 `cbf7012a2ee5fb9b527a06f33e73a4f53a7cfec9`；记录 commit 为本 ADR 合入 main 的 merge commit。候选必须是记录 commit 的祖先，且候选到记录之间只允许 `docs/decisions/`、`docs/evidence/`、`docs/README.md`、`docs/product/` 下的变更（`#32` 与本条都满足）。被认证的输入：候选上的 `docs/spec/08-gates.md` blob 为 `207bbbdf876e288f798538b92777e30bb5ca58ba`；记录基线上的 `docs/evidence/matrix.json` blob 为 `7e1de03acfd0537b04ee221d81f58c4b3ecf4403`（本条不改 matrix，记录 commit 上同值）。
2. **E1：结构闭合成立。** `coverage.criteriaClosed = 11`；`gates[]` 中 `SPEC-GATE-A0.closed = true`；A0 的 11 条判据（`SPEC-GATE-A0-01`…`-11`）全部 `closed: true`。四状态词划分仍为 `criteriaTotal = 42`、`criteriaProbe = 12`、`criteriaOpen = 30`、`criteriaSpec = 0`、`criteriaWithdrawn = 0`。状态词未被改写：A0 汇总仍为 `OPEN`，逐行为 `PROBE`。
3. **E2：逐义务执行型证据成立。** A0 共 58 条非 `notApplicable` 义务（54 条测试义务、4 条命令义务），全部在候选 commit 的某次运行中被点名；未映射义务 **0 条**。逐义务见证链见 [validation.md](../evidence/validation.md) 的「M0 见证链输入（候选 `cbf7012a`）」一节（58 行，含判据、义务原文、`ref` 与点名运行）。测试义务由 `EVID-VAL-52` 的 `names`（125 条全名，按最后一个 `::` 段精确匹配）覆盖；命令义务分别由 `EVID-VAL-52`（workspace 命令）与 `EVID-VAL-53`（trace 命令）逐字匹配。
4. **E3：出口时点集成状态成立。** 三条运行都满足 `run.commit = cbf7012a2ee5fb9b527a06f33e73a4f53a7cfec9`：
   - **E3a**：`EVID-VAL-52`，`class: A`、`kind: test`、命令 `cargo test --workspace --locked`、`exit 0`、`tests.passed = 125`、`tests.failed = 0`、`names` 125 条；
   - **trace**：`EVID-VAL-53`，`class: A`、`kind: runtime`、命令 `cargo run -p zenui-app --example todolist_headless --locked`、`exit 0`、`traceSha256 = 57a4a142999537ddcd0fe4778d5f3604a91a77e664e0a926492f5cc002b0857e`；
   - **E3b**：`EVID-VAL-54`，`class: D`、`kind: test`、同一命令、`exit 0`、`tests.passed = 125`、`tests.failed = 0`、run 级 `platforms = [ubuntu-latest, macos-latest, windows-latest]`，CI run `44437050702`。
5. **出口结论与效力。** 在上述候选与输入上，M0 的 E1–E3 全部成立，**M0 出口成立**。出口不改变任何状态词，也不把任何判据升级为证据层状态词；出口结论只表示 `ADR-0027` 的 E1–E3 在该候选上成立。`roadmap.md:45` 的第二句「嵌套 batch 的中间值泄漏反例转绿」由 `SPEC-GATE-A0-04` 的见证运行 `EVID-VAL-52` 满足。出口只解除 `roadmap.md:37` 对 M1 宏 lowering 扩展的阻塞；M1 仍受 `SPEC-ROUTE-003` 前置、`SPEC-GATE-A1` 出口与对外承诺条款约束，M0 本身不发布 crate、不写 user-facing 文档。
6. **冻结的结束。** 记录合入后，`ADR-0027` 的候选冻结结束。此后任何 A0 判据、义务、证据运行或相关 ADR 的变化，按 `ADR-0027` 决定 7 重算出口判定；不自动延续本次结论。

## 依据

- `ADR-0027` 决定 1–7：定义域、E1/E2/E3、`PROBE` 规则、记录形状与撤销条件；本条只执行记录义务。
- `ADR-0024` 决定 1–4：`closed` 由义务与证据重算，`criteriaClosed` 是派生字段，gate 聚合为全称。
- `ADR-0025`：12 条 `notApplicable` 义务均有 `adr` 与 `acceptedBy`，目标文本已落进 A1 行；它们不计入 E2 的 58 条。
- `ADR-0028`：`command generation` 已移交 A1-07，不在 A0 的判据集合内，也不构成本次出口条件。
- `SRC docs/evidence/matrix.json`：`coverage`、`gates[]`、`criteria[]` 与 `runs[]`；上述数字与运行字段直接取自记录基线 blob `7e1de03acfd0537b04ee221d81f58c4b3ecf4403`。
- [validation.md](../evidence/validation.md)「M0 见证链输入（候选 `cbf7012a`）」：58 条义务的逐条点名表；本条的 E2 附件。
- `EVID-VAL-52` / `EVID-VAL-53` / `EVID-VAL-54`：候选上的本机全量、trace 与 CI 三平台运行；字段见 `matrix.json` 的 `runs[]`。
- `docs/spec/08-gates.md`（候选 blob `207bbbdf876e288f798538b92777e30bb5ca58ba`）：A0 判据集合与 §0 反作弊规则；`:48` A0 方法为编译期 + 运行时 trace。
- `roadmap.md:31`、`:37`、`:45-46`：M0 交付与无对外承诺、M1 的阻塞关系与前置。
- `SRC scripts/check_docs.py` 的 `check_matrix_criteria` 与 `check_matrix_coverage`：本次记录所依赖的谓词与计数由 checker 重算，不由手填。

## 影响与撤销条件

**影响**

- M0 出口被记录为已成立；M1 的宏 lowering 扩展可以开始，但 M1 的其余前置与出口不变。
- 不改变任何状态词、判据文本或证据数据；本条只新增决策记录与索引/计数。
- 出口记录不是永久属性：它的输入是可变的证据层。任何后续变化按 `ADR-0027` 决定 7 重算。

**撤销条件（可判定）**

1. 任一 A0 判据的 `closed` 由 `true` 变为 `false`，或 A0 判据集合发生未裁决的变化（`ADR-0027` 决定 7 a/b）。
2. 本条引用的任一运行失效：commit 不再是候选、运行被删除或改记为失败、`names`/命令/摘要字段被撤回，或复核发现运行并未断言所映射义务（`ADR-0027` 决定 7 c）。
3. `EVID-VAL-52` 或 `EVID-VAL-54` 不再复现，或 E3b 未覆盖候选矩阵的三个 OS 家族（`ADR-0027` 决定 7 d）。
4. `ADR-0024`、`ADR-0025` 或 `ADR-0027` 被 supersede、撤销，或 `ADR-0025` 的撤销条件被触发，使对应判据的 `closed` 变假（`ADR-0027` 决定 7 e）。
5. 候选不再是记录 commit 的祖先，或候选到记录之间出现白名单之外的变更：本次记录无效，必须在新候选上重新记录，不得就地修补本条。
6. 撤销以新 ADR 记录，旧记录正文不改；撤销后依赖 M0 的 M1 宏 lowering 扩展停止，直到 E1–E3 重新成立。
