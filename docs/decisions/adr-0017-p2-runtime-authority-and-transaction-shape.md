---
doc-id: ADR-0017
layer: decisions
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - docs/spec/02-runtime.md
  - docs/spec/08-gates.md
  - docs/evidence/validation.md
  - crates/core/src/handle.rs
  - crates/core/src/store.rs
  - crates/core/src/transaction.rs
  - crates/app/src/lib.rs
replaces: []
---

# ADR-0017：P2 的运行时权威与事务形状

## 背景

M0 的 P1（PR #3，merge `90625ae`）打通了 A0-01 的 headless 闭环，`SPEC-GATE-A0-02`…`-11` 仍为 `OPEN`。进入 P2 前，`spec/` 与实现之间存在两处**承重**张力——照原文实现会撞墙，而不是口味分歧：

1. `SPEC-RT-005` 把 `get`/`with` 写在 `FieldHandle<T>` 上，但 handle 只有 `{runtime_id, slot, generation}`，没有回到 arena 的路径；代码里也不存在 `FieldHandle`，只有 row 级 `Handle<T>`（`RowId = Handle<Row>`）。「跨 runtime 传递的 handle MUST 被拒绝」要求两个端点：句柄的来源，以及**解析者是谁**。后者只有解析者自己知道。
2. `SPEC-RT-033` 要求嵌套 batch 只提交一次；`SPEC-RT-051`–`-053` 要求 preflight 在权威树之外演算、失败后与上次提交完全一致。现有 `TransactionState` 只有深度计数（`crates/core/src/transaction.rs`），mutation 直接落到 live store 与 live view tree（`crates/app/src/lib.rs` 的 `render()`）。`EVID-VAL-08` 记录的反例正是发布了**两条** patch、其中一条带中间值 `true`。
3. 评审同时确认两处实现缺陷：读失败后仍执行 `mount_row(..., title_len = 0)`（`crates/app/src/lib.rs:283-302`）；`RuntimeId` 由 `Store::new()` 铸造，且 `Display for InvalidHandle` 会打印其数值（`crates/core/src/handle.rs`），使诊断文本依赖全局分配顺序。

`CLAIM-035` 已记录 VAL-08 的反例；`CLAIM-039` 与 `CLAIM-053` 已记录生产运行时、Store/Query 语义尚未实现。本 ADR 只裁决**形状**，不宣称任何 Gate 通过。

## 备选项

### A. handle 侧解析

1. handle 持 `&'a Arena`：`Copy` 保住，但丢掉 `'static`；`insert` 返回的句柄被 `&mut self` 绑住，所有存储 handle 的结构被生命周期传染；跨 runtime 拒绝不可达（两端恒等）。
2. handle 持 `Arc`/`Rc<Arena>`：直接破坏 `Copy`（MUST）；每次复制走引用计数；`with` 跨用户代码持借用，重入即 `BorrowMutError` 或死锁。
3. 全局 registry 按 `RuntimeId` 查表：`Copy` 保住，但查表必然落到句柄**自己的** arena，`ForeignRuntime` 分支不可达——跨 runtime 读取会成功并读到别的 runtime 的行，正是 `SPEC-RT-004` 的别名失败模式；还需注册/注销生命周期，而 `Store::new()` 从未注册。
4. 线程局部「当前 runtime」：能拒绝，但比较的是「谁被安装」而非「谁是所有者」；postcommit 或回调中的合法读会被误拒；「未安装」不可表示；`Send` 的 handle 跨线程静默失败。

### B. 事务机制

1. 就地修改 + undo log：读路径简单，但回滚恒等性依赖每个副作用都被正确记录逆操作（`mark_dependents`、`observed` revision、`Slots::free` 的 generation、`geometry_epoch`、focus/caret），漏一个就不恒等；与 `SPEC-RT-051` 的「在权威树之外演算」相反。
2. 值暂存 + draft/swap（选中）：回滚是丢弃，恒等是结构性的。
3. 每层 savepoint：内层失败只回滚内层；`SPEC-RT-053` 明确禁止部分回滚。
4. 用 `Cell`/`RefCell` 承载事务状态换取嵌套：会把 `already borrowed` 泄漏给用户（`SPEC-RT-038` 明令禁止），且 `Drop` 语义依赖借用顺序。

### C. `RuntimeId` 归属

1. 每个 runtime 自带计数器：两个 runtime 会铸造相同 id，句柄在对方那里通过比较并解析到同 slot 的行——别名读。
2. 线程局部计数器：同进程多线程仍会碰撞。
3. 由地址派生：不可确定，并把地址带进 trace（仓库明确禁止）。
4. 进程级单调分配器（选中），并让该值**不可渲染**。

### D. `SPEC-GATE-A0-06` 的映射子句

1. 测试替身：比较一张失败从未触碰的空表，没有证明力。
2. 经 ADR 把 Taffy/映射子句划到 Gate B：以「测起来麻烦」收窄判据，违反「判据只能加严或依证据放宽」。
3. core 内零依赖私有 `LayoutMap`，并真实参与 preflight（选中）。

## 决定

1. **handle 侧解析不成立。** 解析权属于权威（`Runtime`/`Store`）；handle 只携带 `{runtime_id, slot, generation}`，保持 `Copy`、`'static`、无条件 `Send + Sync`。跨 runtime 拒绝由接收者在任何 slot 访问之前比较 `runtime_id` 完成。
2. **`FieldHandle<T>` 与 row 级 `Handle<Row>`（`RowId`）共享形状与失败语义。** A0 交付 row 级句柄；field 级句柄归后续阶段。
3. **`read_once` 是权威侧的 `&mut` owned 读**，返回 `Result<String, InvalidHandle>`；失败不增加 `io_reads`；取消 mount 是 mount 执行器的职责（`SPEC-RT-013`），不是该方法的返回值语义。
4. **帧内 read-your-own-writes 合法**：任意嵌套层级的 reader 读到所有未关闭层的合并 overlay（按程序序 last-write-wins）。受限的只有「发布到帧外」：patch、下游通知与提交只允许在最外层 exit 发生一次，按 key 合并。
5. **事务机制是值暂存**：model 用 `ModelJournal` + `StoreDraft`，view 用 `ViewDraft`；提交是一次 swap，回滚是丢弃 stage。
6. **唯一允许的急切权威变更是 slot/generation 预留**：`insert` 返回的句柄必须在提交后有效，而失败事务中铸造的句柄不得别名到之后的行；回滚 MUST 释放预留并推进 generation。
7. **错误分两档。** 被拒绝的 handle/store 写是**值错误**：事务继续，帧照常提交（`io_writes` 不增）。preflight/commit/phase/cycle 失败是**致命错误**：整个事务丢弃、`commits` 不增。两者是不同类型，且不存在从值错误到致命错误的 `From`，因此 `?` 不会把值错误静默升级为中止。
8. **嵌套错误使整个事务中毒**，不做 savepoint；内层 `Err` 之后外层即使返回 `Ok` 也不能提交。
9. **`Drop` 永不提交**：不调用用户代码、不 panic、复位 phase 与 depth；提交换入之后 MUST NOT 再存在失败路径。
10. **`RuntimeId` 由进程级单调分配器铸造，铸造点在 `Runtime::new`。** `Store` 由 `Runtime` 构造，`Store::new()` 收成 `pub(crate)`，删除 `impl Default for Store`，不提供公开的 `Store::with_runtime(RuntimeId)`。
11. **`RuntimeId` 不可渲染**：`raw()` 删除或降为 `pub(crate)`；`Debug` 打印 `RuntimeId(..)`；`Display for InvalidHandle` 不打印数值；任何 trace 或诊断文本不得携带该值。
12. **A0-06 的 `ViewId → LayoutNodeId` 映射由 core 内零依赖私有 `LayoutMap` 承担**，并真实参与 preflight；`taffy::NodeId` 适配器归 Gate B。
13. **明确不做**：`RuntimeId` registry；用 `Cell` 替代 `read_once` 的 `&mut`；commit 期间执行用户回调；改变 `geometry_epoch` 的推进粒度（golden step 02 为 5）。

## 依据

- `SPEC-RT-005`：handle 形状是 `{runtime_id, slot, generation}` 的 `Copy` 结构；lookup MUST 返回 `Result`；「跨 runtime 传递的 handle MUST 被拒绝」需要两个端点，接收者必须能是「错误的那一方」。
- `SPEC-RT-033`：只有最外层 exit 提交一次、内层不得泄漏中间值；`EVID-VAL-08`（`CLAIM-035`）记录了两条 patch 且含中间值 `true`——被违反的是发布次数，不是帧内可见性。
- `SPEC-RT-050`：Topo/Binding 在帧内运行，因此帧内读必须能看见本帧的写。
- `SPEC-RT-051`、`SPEC-RT-052`、`SPEC-RT-053`：journal 在权威树之外演算、一次推进 model/view scratch、失败后与上次提交完全一致。
- `SPEC-RT-038`：`RefCell` 借用文本 MUST NOT 泄漏给用户，MUST NOT panic。
- `SPEC-RT-021.5`：cleanup 只入 journal，不直接改树。
- `SPEC-GATE-A0-03`、`SPEC-GATE-A0-04`、`SPEC-GATE-A0-06`：拒绝、单次提交、preflight + swap 的判据。
- `CLAIM-039`、`CLAIM-053`：生产运行时与 Store/Query 语义尚未实现，P1 只是 A0-01 骨架。
- 源码核对（SRC）：`crates/core/src/handle.rs`（`RuntimeId`、`Handle`、`Display for InvalidHandle`）、`crates/core/src/store.rs`（`Store::new`、`record_write`）、`crates/core/src/transaction.rs`（仅深度计数）、`crates/app/src/lib.rs`（`frame`、`render`、读失败仍挂载）。

## 影响与撤销条件

影响：

- P2 的所有写路径收敛到 `Tx`；`Store` 的公开 mutator 变为 `pub(crate)` 的 apply 路径，否则 journal 可被绕过、`SPEC-RT-053` 无法成立。
- 「提交点推进」与「尝试期遥测」必须分开：`epoch`/`io_writes`/`commits` 只随提交推进；`io_reads`/`string_clones`/`query_evals`/`id_materializations`/`memo_recomputes` 是尝试期遥测。
- `SPEC-RT-053` 的比较集显式排除单调计数器；A0-06 的夹具按权威状态逐项比对。
- 视图与 owner 的 id 高水位必须随 draft 暂存，否则失败帧会留下可观测的 id 空洞（trace 会打印 `focus=view#N`）。
- golden trace 仅在语义真变化时重录，并同步 `EVID-VAL-*` 与摘要哈希。

撤销条件（可判定）：

- **handle**：若出现一个**必须**在 handle 侧解析的调用点，且它能在不引入 ambient state 的前提下提供「当前权威」，并有 fixture 证明 `handle.get()` 在两个同时存活的 runtime 之间仍被拒绝、handle 保持 `Copy + 'static`，则新开 ADR 取代决定 1–2。
- **事务**：若出现一个 fixture，其中值暂存无法在单次 swap 内提交（例如 `insert` 的句柄在 commit 前必须被 live 读路径解析，而 overlay 无法表达），且该场景不能靠 slot 预留覆盖，则新开 ADR 改为增量 draft + 预验证。
- **`RuntimeId`**：若出现跨进程或持久化句柄需求（当前 `SPEC-RT-005` 只覆盖内存内），需要进程 nonce，属于新 ADR。
- **`LayoutMap`**：若在 core 内实现后 A0-06 仍无法逐项比对（例如 preflight 在不引入 Taffy 的情况下产生不了非空映射），带证据新开 ADR 修改判据。
