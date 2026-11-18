---
doc-id: ADR-0022
layer: decisions
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - docs/spec/02-runtime.md
  - docs/spec/08-gates.md
  - docs/decisions/adr-0011-docs-four-layers.md
  - docs/decisions/adr-0017-p2-runtime-authority-and-transaction-shape.md
  - crates/core/src/query.rs
  - crates/core/src/txn.rs
  - crates/core/src/store.rs
  - crates/core/src/lib.rs
  - crates/core/src/runtime.rs
replaces: []
---

# ADR-0022：计数器是观测，不是第二权威

## 背景

`SPEC-GATE-A0-07` 与 `SPEC-GATE-A0-08` 的判据本身就是计数：每个 `(epoch, filter)` 的
`query_eval` / `cache_build` / `id_materialize` 各恰 +1；空态下 `row_visits = 0`、`RowSet`
materialization = 0；用 `collect()` 实现 `is_empty` 必须判红。`SPEC-RT-047` 进一步规定：查询合同
MUST 以可观测计数验收，仅凭类型名或布尔返回值 MUST NOT 判定通过。计数因此是判据，不是装饰。

P3 实现（`feat/MORT-6-query-focus-lifecycle`，head `f6ea74e`）的计数存在，但与运行时路径脱节。以下
`crates/` 行号都以 `f6ea74e` 为准（该分支未合并，`main` 上同一函数位置不同）：

- **取行路径不经过缓存。** `Tx::rows`（`txn.rs:746`）先无条件调用 `effective_rows()`（`:747`，全表
  展开成 `Vec<RowKey>`），再交给 `MemoGraph` 计算；`QueryCache::rows` / `rows_at`（`query.rs:139`、
  `:148`）在这条路径上一次都没有被调用。
- **计数靠补记。** `QueryCache::note_eval(rebuilt: bool)`（`query.rs:195`）把「是否发生了构建」变成
  调用方可声明的布尔值，并在 `query.rs:196-199` 自增 `evals` / `builds` / `materializations`；唯一
  调用点在 `txn.rs:775`，位于 `if recomputed` 分支内。计数归属因此来自 MemoGraph 是否重算，而不是
  来自它们命名的缓存工作：一次 `completed` 写会让 `staged_rev` 前进，`Filter::All` 的 memo 随之重算，
  `note_eval(true)` 记一次 build 与 materialization——可 `All` 的 RowSet 内容并没有变化（store 侧
  `set_completed` 只 bump `Active`/`Completed` 的 epoch，`store.rs:220`）；另一方面 `effective_rows()`
  的全表扫描每帧都执行，计数器却可以不动。
- **夹具证明的是平行实现。** `crates/core/tests/queries.rs:30` 自己 `QueryCache::new()`，把
  `runtime.committed()` 当 `&Store` 喂进去，断言的是这个局部缓存的 `counters()`。
  `Runtime::counters()`（`runtime.rs:88-95`）聚合的是 runtime 自己持有的那份 `QueryCache`，看不到
  这些数。
- **旁路在编译期可构造。** `lib.rs:116` 声明 `pub mod query;`，`lib.rs:129` 又 `pub use
  query::{Filter, QueryCache, RowSet}`；外部集成测试可以绕开 `Runtime` 直接造出缓存。
- **接线后还有一处未定义行为。** 缓存键只用 membership epoch（`store.epoch_for(filter)`，
  `store.rs:401`），而 `Store::reorder`（`store.rs:295`）只 `record_write()`、不触碰任何 filter epoch；
  公共的 `Tx::reorder`（`txn.rs:632`）是 golden 路径的一部分（`crates/app/src/lib.rs:389`）。一旦真接线，
  同帧内 reorder 之后会命中旧顺序。

结论：运行时从不走缓存的求值/构建路径，计数器由调用方声明的 `note_eval` 写入，测试夹具断言的是另一个
组件。绿灯证明的是 `QueryCache` 这个平行实现，而不是产品路径。

## 备选项

1. **保留 `note_eval`，在文档里要求调用方如实申报**：否决——「诚实」不是可执行的约束。只要记账入口
   存在，计数就是第二个可写权威，任何调用点漂移都不会被现有测试发现；本轮事故正是这样发生的。
2. **保留公开 `QueryCache`，靠评审或 lint 禁止测试直接构造**：否决——评审规则不是编译期约束。本轮
   连续两次靠人眼抓「测试没测运行时」，说明这类规则没有收敛力。
3. **不保留计数器，改用外部 profiler / 采样式测量**：否决——A0-07 / A0-08 的判据是「恰好 +1」与
   「恰好 0」，采样无法给出可判定的整数；而且 `ADR-0017` 已把尝试期遥测纳入规范（允许在失败事务上
   前进，但必须来自真实工作）。
4. **计数只在做工作的函数里自增；`Tx::rows` 经缓存取行；缓存不可从外部构造**（选中）：可写入口消失，
   旁路在编译期不可构造，判据重新与真实执行同源。

## 决定

1. **删除 `note_eval`，不提供任何记账 API。** 计数的自增点必须与产生该工作的语句在同一函数体内；
   不存在由调用方声明「我做/没做工作」的入口。`Counters` 只有只读快照一种形态。
2. **`Tx::rows` 必须经缓存取行，`RowView` 承载两种来源。** `RowView` 的内部契约固定为五项：
   `rows_for(Filter)`（产出 `(RowKey, bool)`，`bool` 是 `completed`）、`is_empty_for(Filter)`、
   `membership_revision(Filter)`、`order_revision()`、`scan_cost()`。committed store 与 staged overlay
   各实现一份。`rows_at(view, filter)` 自己从 `RowView` 取两个 revision，**不接受调用方传入的
   revision**；现有的 `rows(&Store, …)` 与 `rows_at(&Store, …, revision)` 签名删除。
3. **缓存键 = (membership revision[filter], order revision)，两个 revision 都是新增的显式状态。**
   `Store` 目前只有全局 `epoch` 与 `filter_epoch`，没有 order revision；帧事务只有全局 `staged_rev`。
   因此必须补两件东西：
   - **membership revision（per filter）**：committed 侧取自 `filter_epoch` 的既有语义；staged 侧以
     committed 值为种子单调递增。插入/删除/清空 bump 三个 filter；`completed` 写只 bump
     `Active`/`Completed`。
   - **order revision（一个）**：以 committed 值为种子单调递增，只在顺序真的变化时 bump；必须在公共
     `Tx::reorder` 路径上维护，`Store::reorder`（`#[cfg(test)]`）只是单测入口。
   只用 membership：reorder 后返回旧顺序。帧内只用全局 staged revision：一次 `completed` 写也会让
   `All` 失效（现行 `note_eval` 的重算行为就是这样），A0-07 要求的「反向边只影响读该字段的 filter」
   名存实亡。
4. **`QueryCache` 在类型层面不可从外部构造。** 只删 `lib.rs` 的 `pub use` 不够：`pub mod query;` 仍然
   让 `zenui_core::query::QueryCache` 可命名。实现必须把模块改为 `mod query;`，从根只重导出 `Filter`、
   `RowSet`、`Counters`，并让 `QueryCache` 及其构造函数、`rows_at` / `counters` 收敛为 `pub(crate)`。
   集成测试只能通过 `Runtime` 公共 API 驱动，断言只能读 `Runtime::counters()`。当前无外部消费者且
   crate 未发布（`publish = false`），semver 影响为零；将来若需要外部观测，只导出只读 `Counters`，
   不导出缓存本体。
5. **「不可构造」必须有一条机械自证。** 用 `ADR-0021` 的 UI 机制加一条用例：外部代码写
   `zenui_core::query::QueryCache::new()` 必须编译失败，`.stderr` 固定诊断。否则决定 4 只是意图。
6. **每条计数判据各有一条变异自证，变异必须保持布尔语义、只动计数。** 逐条对应，记录进 `EVID-*`
   （格式沿用 `EVID-VAL-23` 的变异表：变异、命令、哪个夹具变红），不只写注释：

   | 判据 | 变异 | 必须变红 |
   |---|---|---|
   | A0-07 `query_eval` 恰 +1 | 命中分支也自增 `evals` | 第二次消费者 |
   | A0-07 `cache_build` / `id_materialize` 各恰 +1 | 命中分支重建 `RowSet` | 第二次消费者 |
   | A0-07 `completed` 写只影响读它的 filter | `completed` 写 bump 全部 filter 的 membership revision | `All` 不失效的夹具 |
   | A0-08 `row_visits = 0` | `is_empty_for(filter)` 换成 `rows_for(Filter::All).filter(\|(_, c)\| filter.matches(*c)).count() == 0`（布尔不变，只多扫描） | 1000 行空态夹具 |
   | 接线存在 | `Tx::rows` 改回直接展开 `effective_rows()` | A0-07 计数夹具 |

   变异不得改变返回值：把 `is_empty_for` 换成「统计全部行是否为空」会先改变答案，证明不了计数断言在
   起作用。
7. **绿灯不迁移证据，重录由 gate owner 执行。** 接线完成后重录 golden
   （`crates/app/tests/golden/todolist_headless.trace` + `crates/app/tests/golden_trace.rs`）；计数从
   1 变 0 之类必须写清是哪一次行为变化造成的，不得作为「优化成果」。已有 `EVID-VAL-20` / `-25` 不得
   就地改写，必须新增条目取代。
8. **公共路径与测试专用路径分开说。** `Store::reorder` 是 `#[cfg(test)]`，只被 `store.rs` 的单测使用；
   `Tx::reorder` 与 `Tx::reorder_views` 是无条件的公共 API，golden 路径（`todolist_trace`）与 A0-09
   夹具都走它们。两者不可互相外推：store 单测不证明公共提交路径，A0-09 夹具也不证明 `Store::reorder`
   被生产路径使用。决定 3 的 order revision 必须在公共 `Tx::reorder` 上维护。

## 依据

- `SPEC-GATE-A0-07`：「query cache 单接线……`(store_epoch, filter)` 每 epoch query_eval/cache_build/
  id_materialize 各恰 +1；两个消费者 String clones = 0」。
- `SPEC-GATE-A0-08`：「空态由独立 counter/membership 索引派生：`row_visits = 0`、`RowSet
  materialization = 0`……用 `collect()` 实现 `is_empty` 必须判红」。
- `SPEC-RT-043`：query cache MUST 在拓扑稳定后、真实 list 首读时求值并物化一次，第二个消费者 MUST
  共享同一不可变快照；field write MUST NOT 直接 materialize `RowSet`。
- `SPEC-RT-044`：`is_empty(filter)` MUST 来自 query 的标量 empty，MUST NOT 通过 collect / 遍历 /
  `Vec::get` 实现。
- `SPEC-RT-047`：`SPEC-RT-041`–`-046` MUST 以可观测计数验收；仅凭类型名或布尔返回值 MUST NOT 判定。
- `SRC crates/core/src/txn.rs:746-780`（`f6ea74e`）：`Tx::rows` 先无条件展开 `effective_rows()`
  （`:747`）再计算；缓存读取不被调用；`:775` 的 `note_eval(!rows.is_empty())` 是唯一补记点。
- `SRC crates/core/src/query.rs:148-182`（`f6ea74e`）：`rows_at` 是求值路径上的自增点；
  `query.rs:195-200`：`note_eval` 的第二组自增语句，正是要删除的记账入口。
- `SRC crates/core/src/lib.rs:116` 与 `:129`（`f6ea74e`）：`pub mod query;` 与 `QueryCache` 的公开
  `pub use`——两处都公开，只删后者无效。
- `SRC crates/core/src/store.rs:295-313`（`f6ea74e`）：`reorder` 只 `record_write()`，不 bump membership epoch；
  `store.rs:401`：`epoch_for` 只返回 per-filter membership epoch。
- `SRC crates/core/tests/queries.rs:30-31`（`f6ea74e`）：夹具自建 `QueryCache` 并直接读
  `runtime.committed()`。
- `SRC crates/app/src/lib.rs:389`（`f6ea74e`）：`todolist_trace` 在 golden 路径上调用公共 `Tx::reorder`。
- `ADR-0017`：提交点推进与尝试期遥测分离；遥测允许前进，但只能由真实工作推进。
- `ADR-0011`：一个事实只定义一次——计数与执行必须是同一条路径，不能各有一份。

## 影响与撤销条件

**影响**

- A0-07 / A0-08 保持 `OPEN`，直到 `Tx::rows` 经 `rows_at` 取行，且每条判据都有变异自证。
- 公共面变窄：`zenui_core::QueryCache` 与 `zenui_core::query` 路径都消失，`Filter` / `RowSet` 继续
  从根导出；`RowView` 是内部抽象，不进入公共面。这是一次移除；仓库内唯一消费者是集成测试。
- 集成测试不再能构造旁路，断言只能读 `Runtime::counters()`；「某次调用恰好 +1」必须由可观测的公共
  行为触发。
- 决定 3 把缓存键从 spec 现行的「`(store_epoch, filter)`」细化为「(membership revision, order
  revision)」，这是对 `SPEC-RT-043`（`docs/spec/02-runtime.md:388`）与 `SPEC-GATE-A0-07`
  （`docs/spec/08-gates.md:81`）措辞的修正。两者的改写必须由 gate owner 依据本条完成；在那之前，
  spec 与 ADR 对 key 的定义不一致，按 `ADR-0011` 属于待关闭的冲突，不得两头都当权威。
- 其他需要随本条更新的既有文本（本 ADR 不改，全部列为 follow-up）：`docs/evidence/claim-register.md:161`
  的 `CLAIM-045`（PROBE；它同时引用旧的 `(store_epoch, filter)` 与 `second_consumer_shares_the_snapshot`
  夹具，接线后必须重新评估状态）、`docs/spec/08-gates.md:83`（A0-09 的描述不涉及本条，但决定 8 的名字
  边界要与它对读）、`crates/app/tests/golden/todolist_headless.trace` 与对应 `EVID-VAL-20` / `-25`。
- 性能约束不由本 ADR 声称：命中路径不得重新扫描整表，由 A0-08 的 `row_visits = 0` 与 A0-07 的
  `id_materialize` 独立判定。

**撤销条件（可判定）**

1. 若将来出现 runtime 内第二个查询消费者（例如分页查询）而复用 `QueryCache` 是唯一可行路径，必须先
   开 ADR 定义它可被复用的内部形态；不得因此恢复「外部可构造的缓存」，决定 5 的编译失败用例继续保留。
2. 若 `RowView` 被证明无法同时服务 committed 与 staged（例如借用冲突迫使整表复制），需要重审决定 2，
   并以计数证据说明代价；此时 A0-07 保持 `OPEN`。
3. 若 A0-08 的判据从「计数为 0」改为其他可观测形式（需要 spec 变更并新开 ADR），决定 6 的变异表随之
   调整；「计数器没有记账入口」这一条不随之失效。
4. 若出现外部消费者依赖 `zenui_core::QueryCache` 或 `zenui_core::query`（当前为零），公开面移除必须先
   提供迁移说明与 superseding ADR，不得在下游无感知的情况下破坏。
