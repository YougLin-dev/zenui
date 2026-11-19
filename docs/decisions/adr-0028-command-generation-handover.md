---
doc-id: ADR-0028
layer: decisions
status: normative
date: 2026-11-19
owner: 框架产品经理（技术型，10 年+）
sources:
  - docs/spec/02-runtime.md
  - docs/spec/08-gates.md
  - docs/decisions/adr-0024-gate-close-predicate.md
  - docs/decisions/adr-0025-a0-not-applicable-handover.md
  - docs/decisions/adr-0027-m0-exit-policy.md
  - crates/core/src/view.rs
replaces: []
---

# ADR-0028：A0 的 command generation 子句移交 A1

## 背景

`SPEC-RT-026` 要求结构 mutation 前捕获的 `FocusRemovalIntent` 包含四项：受影响控件、同 kind 的前后邻居、可用的输入回退、**command generation**。A0 的判定只覆盖了前两部分的现有对象，input/toolbar 已由 `ADR-0025` 移交；**command generation 既不在 A0-10 的判据文本里，也没有任何 ADR 处理过它**。

A0 是 headless core：没有命令派发、没有 `SubmitEvent`，`FocusIntent` 只有 `next` / `previous`（`crates/core/src/view.rs:46-50`）。因此这不是「还没测」，而是 A0 范围里没有可挂 generation 的对象。把它留在无记录状态，会让 `SPEC-RT-026` 的这条 MUST 在 A0/A1 之间凭空消失；按 `08-gates.md` §9 与 `ADR-0024` 的失败模式，这正是必须显式移交的情形。

`docs/spec/08-gates.md` 的 `SPEC-GATE-A1-07` 已经要求 `SubmitEvent` 携带 `origin/command_id/generation/default_prevented`；generation 在 A1 有对象，且提交语义属于 A1-07。

## 备选项

1. **在 A0 引入命令派发与 generation**：否决。A0 的范围是「无宏、无窗口、无 GPU」的 headless core，事件派发已经按 `ADR-0025` 归 A1；为了一个未进入 A0 判据文本的子句扩大范围，会把 M0 变成 A1 的前置实现。
2. **不记录，视为默认忽略**：否决。`SPEC-RT-026` 是 normative MUST；不记录会让该义务在两层都无归属，正是 `ADR-0024` 把「未映射义务」判为未关闭的理由。
3. **记 `notApplicable` 但不指定目标**：否决。等同丢弃要求，A1 落地时无人负责。
4. **记 `notApplicable`，`acceptedBy` 指向 `SPEC-GATE-A1-07` 并在该行落地义务文本（选中）**：对象在 A1 存在，目标行已携带 generation，移交接得最小。

## 决定

1. **A0 不实现 command generation。** A0 没有命令派发与 `SubmitEvent`；`FocusIntent` 只有 `next` / `previous`（`crates/core/src/view.rs:46-50`）。该子句在 A0 没有执行对象。
2. **记录为 `notApplicable` 并移交 A1-07。** 记录形状沿用 `ADR-0025`：保留原文「command generation」，`kind: notApplicable`，`adr: ADR-0028`，`acceptedBy: { criterion: SPEC-GATE-A1-07, text: "`FocusRemovalIntent` 记录 command generation" }`。该子句不在 A0-10 的判据文本内，因此不进入 A0 判据的 `obligations[]`，也不影响任何 A0 判据的 `closed`；本 ADR 正文是这次移交的载体，A1-07 的义务列表只增加上述目标文本。
3. **目标文本必须在 `SPEC-GATE-A1-07` 落地。** 在该判据的 MUST 列新增「`FocusRemovalIntent` 记录 command generation」，与已有的 `SubmitEvent` generation 断言同源；文本未落地前移交不成立（本条与文本同 PR 合入）。
4. **M0 出口不以该子句为前提。** A0 不实现该子句，M0 出口的判据集合与 `closed` 计算不包含它；这不表示该 MUST 被撤销，只表示它在 A1 结算。
5. **A0-10 的既有义务不变。** 本 ADR 不改变 `SPEC-GATE-A0-10` 的文本、义务或 `closed`；A0-10 的 input/toolbar 移交仍以 `ADR-0025` 为准。

## 依据

- `SPEC-RT-026`（`docs/spec/02-runtime.md:271-279`）：MUST 捕获「受影响控件、同 kind 的前后邻居、可用的输入回退、command generation」。
- `docs/spec/08-gates.md` §2：A0 范围是 headless，不使用 proc macro、winit 或 GPU。
- `docs/spec/08-gates.md:101`：`SPEC-GATE-A1-07` 已要求 `SubmitEvent` 携带 `origin/command_id/generation/default_prevented`；IME 与提交结算属于 A1。
- `SRC crates/core/src/view.rs:46-50`：`FocusIntent` 只有 `pub next` / `pub previous`。
- `ADR-0025`：`notApplicable` 须保留原文、指定 `acceptedBy` 目标，并把新增义务文本落进目标行；部分对象不得整体移交。
- `ADR-0024` 决定 1：未映射且未标 `notApplicable` 的义务使判据 `closed: false`。
- `ADR-0027` 决定 4：候选 commit 之后不得再改 `docs/spec/`；本条因此在候选之前落 spec 文本。
- `08-gates.md` §9（`:174-179`）：判据只能加严或依证据放宽；放宽必须留 ADR 说明哪条证据不支持原判据。

## 影响与撤销条件

**影响**

- `docs/spec/08-gates.md` 的 `SPEC-GATE-A1-07` MUST 列新增一条义务；状态词不变，A0-10 不变。`matrix.json` 只在 A1-07 的义务列表补上该目标文本（A0 判据不变），否则义务拼接校验会失败。
- `docs/decisions/index.md` 登记本条；`docs/README.md` §9 的决策文件计数随新增文件更新。
- M0 出口记录不引用本子句；出口的判据集合仍由 `08-gates.md` §2 的 A0 行决定（`ADR-0027` 决定 1）。

**撤销条件（可判定）**

1. A0 引入命令派发或 `SubmitEvent`（或 `FocusIntent` 增加 generation 字段）时，该义务回到 A0，`notApplicable` 必须由新 ADR 撤销；在撤销前不得据本条把 A0 视为已完成该子句。
2. `SPEC-GATE-A1-07` 被改名、拆分或删除该新增文本时，按 `08-gates.md` §9 在本条追加 old → new 映射；找不到目标文本即移交失效，义务回到 A0，须由新 ADR 重新指定目标。
3. 若 `SPEC-RT-026` 本身被 supersede 或删去 command generation 子句，本条须由新 ADR 说明其失效；不得就地修改本条。
