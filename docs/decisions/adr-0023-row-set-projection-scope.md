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
  - crates/core/src/query.rs
  - crates/core/src/txn.rs
replaces: []
---

# ADR-0023：A0 的行集投影是缓存快照，可订阅的 `Memo<RowSet>` 归 A1

## 背景

`SPEC-GATE-A0-07` 的机制列写着「query cache 单接线……`Memo<RowSet>` 只追踪 cache revision」，
`SPEC-MACRO-011` 又要求 `<For>` 的 `each`「MUST 是共享的 `Memo<RowSet>`（**或 runtime 声明的同构
投影**）」。两处措辞都预设了一个图节点携带行集。P3.2 按 `ADR-0022` 决定 2 完成后，代码里没有这样
的节点：

- `QueryCache` 是私有的（`mod query;`，`lib.rs:48`），槽位键为
  `(membership revision[filter], order revision)`（`query.rs:155`、`query.rs:184`）。
- `Tx::rows` 只做转调，返回缓存持有的共享 `RowSet`（`txn.rs:805-808`）；两个消费者拿到同一
  `Arc<[RowKey]>`（`shares_with`）。
- P3.2 移除了三只行集 memo，`memo_recomputes` 全量 trace 由 22 降到 14，`edge_count` 不再观察查询。

A0 没有 UI 树、没有 `For`、没有宏，`render` 每帧直接读 `tx.rows`。因此「memo 节点」在 A0 没有
任何消费者，它的唯一可观测后果是让 `edge_count` 动。

## 备选项

1. **为每个 filter 建一个 memo 节点，body 读 revision 源再调缓存。** 否决：A0 没有消费者，这个
   节点除了移动计数器不改变任何行为——正是本轮反复清理的「为计数器造行为」；它还与缓存叠成两层
   缓存，和 A0-07 判据里的「单接线」相反；revision 源每帧重算会把 `memo_recomputes` 灌成噪声。
2. **改写 `SPEC-GATE-A0-07` 与 `SPEC-RT-045`，删掉 `Memo<RowSet>` 的措辞。** 否决：那是把 MUST
   降级以迁就实现。A1 的 `For` 确实需要可订阅句柄，删掉要求只是把问题推给下一层。
3. **A0 由缓存快照承担共享投影，可订阅节点随 `For` 在 A1 落地，并在证据里写明 A0 未验证 memo
   条款**（选中）。它使用 `SPEC-MACRO-011` 自己留的口子，不降低任何判据强度，也不制造空节点。
4. **让 `Tx::rows` 返回一个没有节点的 `Memo<RowSet>` 占位句柄。** 否决：句柄没有订阅能力，只是
   改名；它会让「已有 memo」这个误读显得有据可依。

## 决定

1. **A0 的共享行集投影是缓存快照。** `Tx::rows(filter)` 返回的 `RowSet` 即
   `SPEC-MACRO-011` 所称的「runtime 声明的同构投影」：由缓存槽按
   `(membership revision[filter], order revision)` 持有，第二个消费者 MUST 共享同一不可变快照。
2. **可订阅的 `Memo<RowSet>` 属于消费者层，随 `For` 在 A1 落地。** 落地时它 MUST 只从
   `(membership revision, order revision)` 派生：MUST NOT 读整表、MUST NOT 依赖 store 节点。这是
   `ADR-0022` 决定 3 的键在消费者层的直接投影，不是另一套失效规则。
3. **`SPEC-GATE-A0-07` 的 memo 条款不因本条被声明为已验证。** A0-07 的 `PROBE` 只覆盖判据列的
   计数；证据列 MUST 写明「`Memo<RowSet>` 节点在 A0 不存在，归 A1」。把 A0-07 的绿读成「memo 已验」
   是误读，任何文档、测试名或评审结论不得这样写。
4. **粗粒度 membership 掩码的成立前提。** `insert`、`remove`、`clear_completed` 同时推进 order
   revision（`txn.rs:618`、`:644`、`:672`），因此三个 filter 的槽必然全部失效；在这个前提下，
   三 filter 全 bump 的粗粒度掩码与逐集合掩码结果等价，取粗粒度。`completed` 写不推进 order
   revision，只推进 `Active`/`Completed`（`txn.rs:729`）。这条前提是可判定的，见撤销条件 1。
5. 不修改 `ADR-0022`；本条是它在消费者层与 revision 前提上的补充。

## 依据

- `SPEC-GATE-A0-07`（`docs/spec/08-gates.md:81`）：机制列含「`Memo<RowSet>` 只追踪 cache
  revision」；判据列是「每 epoch query_eval/cache_build/id_materialize 各恰 +1；两个消费者
  String clones = 0」。本条只区分这两列的作用，不改写任何一列。
- `SPEC-MACRO-011`（`docs/spec/03-macro.md:125`、`:129`）：「`each` MUST 是共享的
  `Memo<RowSet>`（或 runtime 声明的同构投影）」。
- `SPEC-RT-043`：query cache MUST 在拓扑稳定后、真实 list 首读时求值并物化一次，第二个消费者
  MUST 共享同一不可变快照。
- `SRC crates/core/src/query.rs:155`、`:184`：`QueryCache` 为 `pub(crate)`，`rows_at` 自取两个
  revision，不接受调用方传入。
- `SRC crates/core/src/txn.rs:805-808`：`Tx::rows` 构造 `StagedView` 后转调 `rows_at`。
- `SRC crates/core/src/txn.rs:618`、`:644`、`:672`、`:729`：三类结构写推进三个 membership
  revision；`completed` 写只推进 `Active`/`Completed`；`note_order` 在前三类写后被调用。
- `SRC crates/core/src/lib.rs:48`：`mod query;` 为私有模块。
- `ADR-0022` 决定 2、3：`RowView` 契约与 `(membership, order)` 键。
- `ADR-0011`：一个事实只定义一次——memo 的失效规则与缓存的键必须是同一条规则。

## 影响与撤销条件

**影响**

- A0-07 可以在计数判据完整的前提下翻 `PROBE`，但证据列必须带「memo 节点归 A1」这一句；A0 的整体
  状态不因本条改变。
- 在 A1 之前，`edge_count` 与 `memo_recomputes` 不观察查询。A0-11 的基线因此不包含 query 节点，
  这是有意为之，不是遗漏。
- `SPEC-RT-045` 的 `<For>` 契约在本条下仍然有效，只是它属于 A1 的实现范围。

**撤销条件（可判定）**

1. 若 `insert`、`remove`、`clear_completed` 中任何一类不再推进 order revision（例如为了省一次失效
   只推进 membership），粗粒度掩码会把「未变的集合」当成变了或反过来，必须回到逐集合掩码并新开
   ADR 记录这次改动。
2. 若 A1 实现 `For` 时发现 revision 对不足以决定订阅失效（例如需要逐行粒度），必须新开 ADR 定义
   消费者侧节点；不得把 `RowView` 或 `QueryCache` 重新公开来满足它。
3. 若出现外部消费者要求「持有 `RowSet` 并收到失效通知」，必须先提供 `Memo` 节点；不得用公开缓存
   实例来代替订阅语义。
