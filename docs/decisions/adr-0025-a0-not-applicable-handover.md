---
doc-id: ADR-0025
layer: decisions
status: normative
date: 2026-11-19
owner: Crew 队长（架构）
sources:
  - docs/spec/08-gates.md
  - docs/decisions/adr-0017-p2-runtime-authority-and-transaction-shape.md
  - docs/decisions/adr-0022-counters-are-observation.md
  - docs/decisions/adr-0023-row-set-projection-scope.md
  - docs/decisions/adr-0024-gate-close-predicate.md
  - docs/evidence/validation.md
  - crates/core/src/view.rs
  - crates/core/src/graph.rs
  - crates/core/src/owner.rs
  - crates/core/src/handle.rs
  - crates/core/tests/lifecycle.rs
  - crates/core/tests/gates.rs
replaces: []
---

# ADR-0025：A0 无执行对象的子句移交 A1

## 背景

`ADR-0024` 规定判据 `closed` 需要每条义务映射到夹具/命令，或在 A0 无对象时标 `notApplicable` + ADR +
`acceptedBy`。A0 的七条判据里存在这样的义务：它们的对象在 A0 **不存在**，不是没测，而是没有可测的
东西。`08-gates.md` §9 要求这类放宽逐条说明「哪条证据不支持原判据」并保留 old → new 映射。

写「移交」时有一个必须说清的边界：**除 `SPEC-GATE-A1-03` 已列「For 缺 key」外，A1 现有判据的文本
并不包含这些义务**。因此本条每条都写成「目标行 + 该行必须新增的义务原文」，不声称目标行已经覆盖它。
这些新增属于 §9 允许的**加严**；spec 文本由 gate owner 在 A1 变更中落地，本条提供 old → new 映射与
撤销条件。`acceptedBy.text` 未出现在目标行之前，`ADR-0024` 必须为对应 A0 判据算 `closed: false`。

`notApplicable` 的记录形状（与 `ADR-0024` 一致）：保留义务原文，`kind: notApplicable`，
`adr: ADR-0025`，`acceptedBy: { criterion: <A1 判据>, text: <该行须新增的义务原文> }`。原文不删除，
历史引用不断链。**只移交没有对象的义务**；同一句里已有对象的部分留在 A0，继续映射到现有夹具。

## 备选项

1. **让这些义务继续以 `OPEN` 留在 A0**：否决。它们不是「还没做」，而是 A0 范围内没有对象；留在 OPEN
   会让 A0 永远无法关闭，而且与「A0 是 headless core」的范围定义冲突。
2. **直接删掉义务**：否决。§9 要求保留映射；删掉会让 A1 的义务消失，历史证据也无法对应。
3. **标 `notApplicable` 但不指定 A1 目标**：否决。那等于用一个标记把要求丢掉，A1 落地时没人会想起。
4. **逐条说明、指定 `acceptedBy` 并写明目标行须新增的义务（选中）**：保留原文、给出不支持的证据、
   目标行与新增义务、可判定的撤销条件。

## 决定

1. **A0-03 的 click 派发（证据列义务）。**
   - 原文：通过判据列「旧 handle/旧 generation 只得到一条 `InvalidHandle`，无默认值、无旧值 patch」；
     证据列「slot 复用后派发旧 **click**/读/写」。
   - **留在 A0**：旧 generation 的读/写拒绝有对象也有夹具——
     `a0_03_stale_handle_does_not_alias_the_reused_slot`（`tests/gates.rs`）断言回收槽位后旧 handle
     的读/写返回 `ValueError::Handle`；这部分映射到该夹具与 `EVID-VAL-23`。
   - A0 无对象：A0 没有事件源与派发路径，`ViewKind` 只有 `Root` / `Row`（`view.rs:26-29`）；证据列的
     click 实例化没有夹具。`EVID-VAL-23` 的「不证明什么」关于跨 runtime 读的措辞已过期（
     `a0_03_cross_runtime_access_is_rejected` 同时断言读写），但与本条无关。
   - `acceptedBy`：`SPEC-GATE-A1-06`，text「旧 generation 派发 click 只得到 `InvalidHandle`，不产生
     patch」。
   - 撤销：A0 提供 headless 事件派发 API 时，该义务回到 A0-03，`notApplicable` 由新 ADR 撤销。

2. **A0-05 的多节点环。**
   - 原文：「`WriteDuringCompute`、cycle 是领域错误」。
   - **留在 A0**：`WriteDuringCompute` 与**自读环**由 `EVID-VAL-24` 覆盖
     （`a_write_during_compute_is_a_domain_error`、`a_self_reading_memo_is_a_cycle`、`Tx::probe_cycle`）。
   - A0 无对象：`DepReader` 只读，memo body 不能在计算中发起嵌套 compute（`graph.rs:756-762`）；
     `the_compute_stack_reports_a_read_into_a_cycle` 用 `begin_computing` 手工构造两节点状态、只钉栈
     检查，**不是**不使用钩子的真实 A→B→A。没有这样的夹具。
   - `acceptedBy`：`SPEC-GATE-A1-04`，text「真实 A→B→A 环返回 `GraphCycle`（不使用 `begin_computing`
     手工构造）」。
   - 撤销：A0 图 API 允许嵌套 compute 或显式边插入时，义务回到 A0-05。

3. **A0-06 的 native 树、IME 与 resource。**
   - 原文：「失败后 native 树、id 映射、focus、geometry epoch 与上次提交完全一致」。
   - **留在 A0**：失败注入机制与 fake view / id 映射 / focus / geometry 的回滚比对由 `EVID-VAL-23`
     覆盖；`LayoutMap` 与 preflight 单测由 `EVID-VAL-24` 覆盖。这些不移交。
   - A0 无对象：A0 没有 native 树、没有 IME、没有 GPU resource。三条义务分别标记：
     - native 树 → `acceptedBy` `SPEC-GATE-A1-01`，text「拒绝的 TreeCommit 后 native 树/id 映射/
       focus/geometry 与上次提交逐字段一致」；
     - IME → `acceptedBy` `SPEC-GATE-A1-07`，text「IME preflight 失败后候选与组合状态回滚」；
     - resource → `acceptedBy` `SPEC-GATE-A1-01`，text「surface/resource 预检失败后资源计数回基线」。
   - 撤销：A0 引入 native adapter 或 IME 模型（与 headless 范围冲突，需先改范围）时，三项回到 A0-06。

4. **A0-07 的 `Memo<RowSet>` 与 per-row 反向边粒度。**
   - 原文：「`rows(filter)` 求值登记 membership + 实际读取的 `(row, completed)` 反向边；命中只标脏
     cache；`Memo<RowSet>` 只追踪 cache revision」。
   - **留在 A0**：membership 登记与「命中只标脏 cache」由 `EVID-VAL-42` 的每 filter 计数与变异覆盖；
     反向边按 filter 折叠是 `ADR-0022` 决定 2 的现行形态。
   - A0 无对象：没有 `Memo<RowSet>` 节点（`ADR-0023` 决定 1），也没有 per-row 边对象。两条义务：
     - memo 条款 → `acceptedBy` `SPEC-GATE-A1-04`，text「`Memo<RowSet>` 只随 (membership revision,
       order revision) 失效」；
     - per-row 粒度 → `acceptedBy` `SPEC-GATE-A1-04`，text「per-row 失效」。
   - 撤销：A0 出现行集订阅者（例如 headless `For`）时，两条款回到 A0-07，`ADR-0023` 的边界同时重审。

5. **A0-09 的 `For` 与 subscriptions。**
   - 原文：「For 强制 key；same-owner reorder 保留 ViewId/generation/owner/subscriptions/caret/focus；
     duplicate key 在任何 patch 前拒绝」。
   - **留在 A0**：duplicate key 前置拒绝、reorder 的 ViewId/RowKey/focus/caret/owner 由 `EVID-VAL-34`
     覆盖；generation 见「相邻证据缺口」。
   - A0 无对象：A0 没有 proc macro、没有 `For` DSL（`handle.rs:22` 只有内部 `macro_rules!`，与 `For`
     无关）；没有订阅模型（`owner.rs` 只为 churn 记账）。
     - `For` 强制 key → `SPEC-GATE-A1-03` 的证据列**已列**「trybuild 覆盖 … For 缺 key」，无需新增，
       直接映射；
     - subscriptions → `acceptedBy` `SPEC-GATE-A1-04`，text「same-owner reorder 后订阅不增不减」。
   - 撤销：A0 引入 `For` 或订阅模型时，对应义务回到 A0-09。

6. **A0-10 的 input / toolbar 回退。**
   - 原文：「commit 后仅在目标仍受影响时按 同 kind next → prev → 可聚焦 input → toolbar」。
   - **留在 A0**：row-kind 的 **next** 回退由
     `a0_10_deleting_the_focused_row_settles_one_intent` 断言（`lifecycle.rs` 明写「next sibling, not
     previous」）。`prev` 的 intent 字段由 `view.rs:327` 的 `focus_intent_reports_both_neighbours`
     断言，但结算分支没有夹具——见「相邻证据缺口」，仍留在 A0。
   - A0 无对象：`ViewKind` 只有 `Root` / `Row`，没有可聚焦 input、没有 toolbar。两级：
     - input → `acceptedBy` `SPEC-GATE-A1-07`，text「无同 kind 后继时焦点回退到可聚焦 input 并只结算
       一次」；
     - toolbar → `acceptedBy` `SPEC-GATE-A1-06`，text「无 input 候选时焦点回退到 filter toolbar 并只
       结算一次」。
     A1-06 现有的「filter/Add 可操作」不是回退顺序，不能替代。
   - 撤销：A0 的视图模型长出 input/toolbar kind 时，两级回到 A0-10。

7. **A0-11 的 subscriber bucket。**
   - 原文：「**空 subscriber bucket 不计活跃订阅**」。
   - **留在 A0**：1200 轮 churn 的 owner / edge / source / layout / rows 回基线由 `EVID-VAL-34` 覆盖；
     caches 见「相邻证据缺口」。
   - A0 无对象：subscriber bucket 没有建模（`owner.rs:14-15` 的 `TODO(A0-11)` 明写「nothing to hold of
     yet」）。`acceptedBy`：`SPEC-GATE-A1-04`，text「空 subscriber bucket 不计活跃订阅」。
   - 撤销：A0 建模 subscriber bucket 时，义务回到 A0-11。

**相邻证据缺口（有对象、缺夹具；不得标 `notApplicable`）。** 按 `ADR-0024` 必须补夹具或保持未映射：

- `SPEC-GATE-A0-09` 的「reorder 保留 generation」：`a0_09_a_reorder_preserves_identity_and_moves_no_counter`
  断言 ViewId/RowKey/focus/caret/owner，没有 generation 断言；`ViewNode.generation` 有对象。
- `SPEC-GATE-A0-10` 的 `prev` **结算**分支：intent 的 `previous` 字段由
  `view.rs:327` 的 `focus_intent_reports_both_neighbours` 断言；但所有删除夹具删的都是有后继的行，
  `Tx::commit` 的 previous 分支未被触发。`ViewKind::Row` 有对象。
- `SPEC-GATE-A0-11` 的「caches 回到基线」：churn 夹具断言 owner/edge/source/layout/rows，没有 query
  cache 断言；`QueryCache` 有对象。

## 依据

- `08-gates.md` §9（`:174-179`）：放宽必须留 ADR 说明哪条证据不支持原判据；改名/拆分保留 old → new；
  加严不需要放宽理由。本条的「目标行须新增」按加严处理。
- `08-gates.md` §2 的 A0 范围：headless，不使用 proc macro、winit 或 GPU——这是「无对象」而非
  「未测试」的根据。
- `08-gates.md` §3 的 A1 判据原文：A1-01 真窗口、A1-03 `For` 缺 key 的 trybuild、A1-04
  `For`/binding/订阅、A1-06 键盘与可见域、A1-07 IME、A1-08 Add 事务失败注入。除 A1-03 外，目标行
  现有文本不含本条新增义务，故每条都写明「须新增」。
- `EVID-VAL-23`：stale handle 的读/写拒绝、跨 runtime 拒绝、回滚夹具；不含 click 派发、native 树或
  IME。其关于跨 runtime 读的旧措辞已过期，引用以夹具为准。
- `EVID-VAL-24`：diamond、长链、`WriteDuringCompute`、自读环、`LayoutMap`/preflight；不含不使用
  钩子的多节点环、native/IME。
- `EVID-VAL-34`：duplicate key 前置拒绝、reorder 身份与焦点、焦点 next 结算、churn 回基线；不含
  `For`、订阅、input/toolbar、subscriber bucket、generation 或 caches。
- `EVID-VAL-41`：类型层 E0308 UI 快照；不涉及 click 派发。
- `EVID-VAL-42`：每 filter 的计数与变异判红；不含 per-row 反向边与 `Memo<RowSet>`。
- `SRC crates/core/src/view.rs:26-29`：`ViewKind` 只有 `Root` / `Row`；`:327`
  `focus_intent_reports_both_neighbours`。
- `SRC crates/core/src/graph.rs:756-762`：`DepReader` 只读，A0 不可表达 A→B→A；该测试只钉栈检查。
- `SRC crates/core/src/owner.rs:14-15`：subscriber bucket 未建模。
- `SRC crates/core/src/handle.rs:22`：A0 只有内部 `macro_rules!`，没有 proc macro 或 `For` DSL。
- `SRC crates/core/tests/gates.rs:403`：`a0_03_cross_runtime_access_is_rejected`。
- `SRC crates/core/tests/lifecycle.rs:87-166`、`:171-221`、`:433-534`：reorder 断言不含 generation；
  焦点夹具断言 next、不断言 prev 结算；churn 断言不含 caches。
- `ADR-0023` 决定 1：A0 没有 `Memo<RowSet>` 节点；`ADR-0022` 决定 2：反向边按 filter 折叠。
- `ADR-0024`：`notApplicable` 需要 ADR 引用与 `acceptedBy`，目标文本未落地则 `closed: false`。

## 影响与撤销条件

**影响**

- 本条提供七条 `notApplicable` 的依据与 `acceptedBy` 目标；标记本身仍须由证据层变更写入
  `matrix.json`（`ADR-0024` 的 `criteria[]`），当前矩阵没有 `criteria[]`、没有 `notApplicable`、
  没有 `gates[].closed`。
- 这七条（除 A1-03 已有的 For 缺 key 外）的 `acceptedBy.text` 是目标行的**加严要求**。在 gate owner
  把对应义务写进 A1 的 MUST/证据列之前，`ADR-0024` 必须为对应 A0 判据算 `closed: false`；不得把本条
  当作「已经落到 A1」的证明。
- spec 文本层：`08-gates.md` 的对应行由 gate owner 在 A1 变更中引用本 ADR；本 ADR 不改 spec 文本。
- 历史不断链：原文保留在 `obligations[].text`，A1 落地时以 old → new 映射回填，不改旧 ID。
- 「相邻证据缺口」三处仍需夹具或保持未映射；它们有对象，不适用 `notApplicable`。

**撤销条件（可判定）**

1. 某项的对象在 A0 出现——点击派发、嵌套 compute/显式边、native/IME/resource、行集订阅者、`For`/
   订阅模型、input/toolbar kind、subscriber bucket——对应义务立即回到 A0 判据，`notApplicable`
   必须由新 ADR 撤销；在撤销前 `closed` 不得据此为真。
2. 若 A1 的某条目标判据被改名或拆分，按 §9 在本条追加 old → new 映射；不得只改 A1 而让本条指空。
3. 若某条义务在 A0 出现**部分**对象（例如只支持 click 但不支持旧 generation 派发），不得整体视为
   `notApplicable`；只对没有对象的那部分标记，其余继续映射。
4. 若目标行的 MUST/证据列找不到 `acceptedBy.text`（目标行拒绝、改名或删除了该义务），移交不成立，
   对应 A0 判据记 `closed: false`，并由新 ADR 重新指定目标；本条的标记随之失效。
