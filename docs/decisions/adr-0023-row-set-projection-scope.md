---
doc-id: ADR-0023
layer: decisions
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - docs/spec/03-macro.md
  - docs/spec/02-runtime.md
  - docs/spec/08-gates.md
  - docs/decisions/adr-0022-counters-are-observation.md
  - docs/evidence/validation.md
  - crates/core/src/query.rs
  - crates/core/src/txn.rs
  - crates/core/src/store.rs
  - crates/core/src/runtime.rs
  - crates/core/src/graph.rs
  - crates/app/src/lib.rs
replaces: []
---

# ADR-0023：A0 的行集投影是缓存快照，可订阅的 `Memo<RowSet>` 归 A1

## 背景

`SPEC-GATE-A0-07` 的机制列写着「query cache 单接线……`Memo<RowSet>` 只追踪 cache
revision」，`SPEC-MACRO-011` 又要求 `<For>` 的 `each`「MUST 是共享的 `Memo<RowSet>`（**或 runtime
声明的同构投影**）」。两处措辞都预设了一个图节点携带行集。P3.2 按 `ADR-0022` 决定 2 完成后，代码里
没有这样的节点：

- `QueryCache` 是私有的（`mod query;`，`lib.rs:48`；根只重导出 `Filter` / `RowSet`，`lib.rs:61`），
  槽位键为 `(membership revision[filter], order revision)`（`query.rs:139-142`、`:155`、`:184-214`）。
- `Tx::rows` 只做转调，返回缓存持有的共享 `RowSet`（`txn.rs:805-808`）；两个消费者拿到同一
  `Arc<[RowKey]>`（`shares_with`）。
- P3.2 移除了三只行集 memo，`memo_recomputes` 全量 trace 由 22 降到 14，`edge_count` 不再观察查询。

A0 有 retained view 树，但没有 `For`、没有宏、也没有任何 `Memo<RowSet>` 节点的订阅者：唯一的行集
消费者是 `crates/app::render`，它每帧 `tx.rows(ui.filter)` 拉一次快照再据此 mount/unmount/reorder
（`app/src/lib.rs:180-206`）。这是**拉取**，不是订阅，所以「memo 节点」在 A0 没有消费者，建出来唯一的
可观测后果是让 `edge_count` 与 `memo_recomputes` 动。

`EVID-VAL-33` 把移除三只 memo 记成「行集 memo 就是缓存槽」。这个说法把消费者层节点和缓存槽混成了
一件事：正确的形态是本节与决定 1——A0 的投影是缓存快照，可订阅节点留到 A1。证据措辞的更正见「影响」。

另一件必须落纸的是粗粒度 membership 掩码的前提。生效的 `insert` / `remove` / `clear_completed` 推进
三个 filter 的 membership 并 `note_order()`（`txn.rs:618-619`、`:644-645`、`:672-673`）；`reorder`
只推进 order（`:700`）；`completed` 写只推进 `Active`/`Completed`（`:729`）。提交时
`set_revisions(staged_membership, staged_order)` 把这些值装回 `Store`（`txn.rs:1248`、
`store.rs:164-168`、`:417-418`）。缓存槽键含 order，order 一动三个 slot 必然全部 miss，所以粗掩码
不改变任何缓存行为。但「不改变缓存行为」不等于「不可观测」：`Store::epoch_for` 是公开只读访问器
（`store.rs:422`；`Store` 由根导出，`lib.rs:63`；`Runtime::committed()` 返回 `&Store`），夹具今天就
能断言「插入不推进 `Completed` 的 membership revision」并因此失败。

## 备选项

1. **为每个 filter 建一个 memo 节点，body 读 revision 源再调缓存。** 否决：A0 没有消费者，这个
   节点除了移动计数器不改变任何行为——正是本轮反复清理的「为计数器造行为」；它还与缓存叠成两层
   缓存，和 A0-07 判据里的「单接线」相反；revision 源每帧重算会把 `memo_recomputes` 灌成噪声。
   它也不能替 A0 满足 memo 子句：没有消费者，节点只有声明价值，等于用一个空节点假装满足。
2. **改写 `SPEC-GATE-A0-07` 与 `SPEC-RT-045`，删掉 `Memo<RowSet>` 的措辞。** 否决：那是把 MUST
   降级以迁就实现。A1 的 `For` 确实需要可订阅句柄，删掉要求只是把问题推给下一层。
3. **A0 由缓存快照承担共享投影，可订阅节点随 `For` 在 A1 落地，并在证据里写明 A0 未验证 memo
   条款**（选中）。它使用 `SPEC-MACRO-011` 自己留的口子，不降低任何判据强度，也不制造空节点。
4. **让 `Tx::rows` 返回一个没有节点的 `Memo<RowSet>` 占位句柄。** 否决：句柄没有订阅能力，只是
   改名；它会让「已有 memo」这个误读显得有据可依。

## 决定

1. **A0 的共享行集投影是缓存快照；可订阅的 `Memo<RowSet>` 属于消费者层，随 `For` 在 A1 落地。**
   `Tx::rows(filter)` 返回的 `RowSet` 即 `SPEC-MACRO-011` 所称的「runtime 声明的同构投影」，由缓存槽
   按 `(membership revision[filter], order revision)` 持有，第二个消费者 MUST 共享同一不可变快照。
   A1 的节点受四条约束：
   - `SPEC-MACRO-011` 的括号只覆盖 A0 的快照投影；`SPEC-RT-045` 对 `For` 没有例外——A1 的 `each`
     MUST 是 `Memo<RowSet>`，快照不是可接受的 `each`。
   - 它 MUST 只从 `(membership revision, order revision)` 派生，MUST NOT 读整表、MUST NOT 依赖 store
     节点；这是 `ADR-0022` 决定 3 的键在消费者层的直接投影，不是另一套失效规则。
   - 帧内使用 `StagedView` 的 revision 对（以 committed 值为种子、提交时装回，`txn.rs:362-369`、
     `:1248`），MUST NOT 直接读 `Store::epoch_for` / `order_epoch`——否则同帧插入后它仍读旧快照，
     违反 `SPEC-RT-033` 的「事务内 tracked 读看见自己刚写的值」。
   - 它 MUST NOT 持有独立缓存或自己的失效计数。A1 需要为图新增行集值形态（当前 `Value` 只有
     `Count` / `Flag`，`graph.rs:30-33`）；表示由 A1 决定，本条只约束来源与依赖。
   **`SPEC-GATE-A0-07` 的 memo 条款不因本条被声明为已验证。** 它在 A0 没有可执行对象（没有 `For`）；
   A0-07 的 `PROBE` 只覆盖判据列的计数，证据列 MUST 写明「`Memo<RowSet>` 节点在 A0 不存在，归 A1」。
   把 A0-07 的绿读成「memo 已验」是误读，任何文档、测试名或评审结论不得这样写。
2. **粗粒度 membership 掩码的成立前提是：每次真正改变 membership 的写都同时推进 order revision。**
   在上述前提下，三 filter 全 bump 与逐集合掩码在缓存与 `Counters` 上不可区分，取粗粒度。但这是
   **前置条件，不是许可**：
   - 它不是不可观测：`Store::epoch_for` 是公开访问器，夹具可以断言精确的 membership 行为；本条不把
     粗粒度说成类型级隐藏，只把它说成一条以 order 为前提的取舍。
   - 它确实会遮住一种 bug：若某次生效的 insert/remove/clear 不再推进 order，`Completed` 会因被过度
     失效而 miss，缓存反而拿到正确的新顺序，任何基于缓存的夹具都不会红。切成逐集合掩码后，同一遗漏
     会让 `Completed` 命中旧顺序，暴露为错误结果。
   - 因此这条前提必须有机械守卫：在 `txn.rs` 加 `#[cfg(test)]` 断言——生效的 insert / remove /
     clear / reorder 推进 `staged_order`，`completed` 写不推进；该守卫是 A0-07 / A0-08 翻 `PROBE`
     的前置条件。调用方没有实际改变模型时（已标记删除的 key、无行的 `clear`、顺序未变的 `reorder`）
     提前返回，不在此列。

本条不修改 `ADR-0022` 的任一条决定；它补的是消费者层边界与掩码前提。

## 依据

- `SPEC-GATE-A0-07`（`docs/spec/08-gates.md:81`）：机制列含「`Memo<RowSet>` 只追踪 cache
  revision」；判据列是「每 epoch query_eval/cache_build/id_materialize 各恰 +1；两个消费者
  String clones = 0」。本条只区分这两列的作用，不改写任何一列。
- `SPEC-MACRO-011`（`docs/spec/03-macro.md:125`、`:129`）：「`each` MUST 是共享的
  `Memo<RowSet>`（或 runtime 声明的同构投影）」。
- `SPEC-RT-043`：query cache MUST 在拓扑稳定后、真实 list 首读时求值并物化一次，第二个消费者
  MUST 共享同一不可变快照。
- `SPEC-RT-033`：事务内 tracked 读看见自己刚写的值是合法且必需的。
- `SRC crates/core/src/query.rs:139-142`、`:155`、`:184-214`：槽键是 `{membership, order}` 二元组，
  两个 revision 都取自视图；`QueryCache` 为 `pub(crate)`。
- `SRC crates/core/src/txn.rs:805-808`：`Tx::rows` 构造 `StagedView` 后转调 `rows_at`。
- `SRC crates/core/src/txn.rs:618-619`、`:644-645`、`:672-673`、`:700`、`:729`：生效的结构写推进三个
  membership 与 order；reorder 只推进 order；`completed` 写只推进 `Active`/`Completed`。
- `SRC crates/core/src/txn.rs:362-369` 与 `:1248`，`crates/core/src/store.rs:164-168`、`:417-418`：
  staged revision 以 committed 值为种子，提交时装回 `Store`。`store.rs` 里同名的 `insert` / `remove` /
  `clear_completed` / `reorder` / `set_completed` 是 `#[cfg(test)]` 直连助手，不是生产提交路径。
- `SRC crates/core/src/store.rs:422`、`crates/core/src/lib.rs:63`、`crates/core/src/runtime.rs:65`：
  `epoch_for` 是公开只读访问器，`Store` 由根导出，`Runtime::committed()` 返回 `&Store`。
- `SRC crates/app/src/lib.rs:180-206`：A0 唯一的行集消费者是每帧 `tx.rows(ui.filter)` 拉取；
  golden 的 reorder 步骤在 `:391`。
- `SRC crates/core/src/lib.rs:48`、`:61`：`mod query;` 为私有模块，根只重导出 `Filter` / `RowSet`。
- `EVID-VAL-33`：A0-07/08 的 7 条夹具；5 条 mutation 中 **4 条判红**，第 5 条（`is_empty_for` 改
  扫描）0 条红，已如实记为边界；`Tx::rows` 改回直接展开 `effective_rows()` 判红 6 条。
- `EVID-VAL-34`：记录 `Store::reorder` 是 `#[cfg(test)]` 直连助手、`Tx::reorder` 才是生产路径。
- `ADR-0022` 决定 2、3、4：`RowView` 契约、`(membership, order)` 键、`QueryCache` 不可从外部构造。
- `ADR-0011`：一个事实只定义一次——memo 的失效规则与缓存的键必须是同一条规则。

## 影响与撤销条件

**影响**

- 本条不改代码，但提出一个必做的测试守卫（决定 2 末尾）。它由 Core Engineer 在 `txn.rs` 落地，是
  A0-07 / A0-08 翻 `PROBE` 的前置条件。
- A0-07 可以在计数判据完整的前提下翻 `PROBE`，但证据列必须带「memo 节点归 A1」这一句；A0 的整体
  状态不因本条改变。`SPEC-RT-045` 的 `<For>` 契约在本条下仍然有效，只是属于 A1 的实现范围。
- 在 A1 之前，`edge_count` 与 `memo_recomputes` 不观察查询。A0-11 的基线因此不包含 query 节点，
  这是有意为之，不是遗漏。
- `EVID-VAL-33` 的「行集 memo 就是缓存槽」需要更正。按证据层规则，历史条目的运行结果与数字不改写：
  由 Release Engineer 在状态翻转 PR 中，在 `matrix.json` 的 `doesNotProve` 写明「memo 未在 A0 验证，
  行集投影是缓存快照」，并新增一条 `EVID-VAL-*` 勘误条目指向本 ADR；不得就地改写 `EVID-VAL-33`。

**撤销条件（可判定）**

1. 若某次生效的 `insert` / `remove` / `clear_completed` 不再推进 order revision，或新增只改
   membership、不改顺序的写，决定 2 失效。处置顺序固定：先由新 ADR supersede `ADR-0022` 决定 3 的
   粗粒度条款，再改成逐集合掩码，并补一条「插入后 `Completed` 的 `query_eval` 不增加」的夹具。在
   superseding ADR 出现之前，`ADR-0022` 决定 3 仍是权威，唯一允许的动作是恢复 order 推进。
2. 若 A0 出现真实的行集订阅者（例如 headless `For`），必须先裁决 memo 形态再实现；不得用缓存快照
   冒充可订阅节点，也不得引用本条跳过 memo。
3. 若 `SPEC-MACRO-011` / `SPEC-RT-045` 的措辞变化使「缓存快照 = 同构投影」不再成立，决定 1 失效，
   需要新开 ADR 重新定义投影。
