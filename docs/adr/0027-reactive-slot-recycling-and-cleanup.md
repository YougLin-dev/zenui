# Signal/Memo 槽位回收、消费者索引与 owner cleanup 边界

Signal 与 Memo 的槽位在 owner 释放后必须可以复用，否则滚动、过滤和分支切换会让响应式 arena 随历史增长。本文记录 #386 落地的三项决策：槽位代际回收、来源到绑定/结构触发的消费者索引，以及把 cleanup 推迟到 `RefCell<State>` 借用之外的释放批次。

## 槽位代际与依赖键

`Signal<T>`/`Memo<T>` 句柄、`SourceRef`、`SignalCell`/`MemoCell` 都携带 32-bit generation。释放时槽位进入 free list，再次创建时 generation 递增并安装新值；generation 到 `u32::MAX` 的槽位直接退休，不回绕。读、写和 memo 就绪检查比较句柄代际：不一致按已释放拒绝，可失败读取返回 `ReadFault::Disposed`，同 owner 内的 `get` 契约 panic。旧代际命中复用槽位时记录 `RuntimeDiagnostic::StaleSource`，由下一次公开刷新的 `RefreshReport::diagnostics()` 交付。

`DepKey` 仍以槽位索引为键（`Signal(u32)`、`Memo(u32)`），不携带 generation。释放一个来源时同时完成三件事：

1. 从它作为消费者的反向表中移除自己；
2. 对每个依赖它的 memo 删除这条 `deps` 条目；
3. 对每个绑定/结构触发消费者删除其边并标脏，使下一次求值读到 `Disposed`。

这保证槽位复用时不存在仍指向旧代际的订阅边，写入新值不会失效无关 memo，旧绑定也不会读到新值。若只给 `DepKey` 加代际而不解绑，旧边仍会常驻并阻止错误命中，但会保留一份随释放历史增长的表；本实现选择强制解绑并保持键布局不变。

oracle 的独立 memo 缓存同样以 `(slot, generation)` 为键；绑定读取、坏源检查和字段写入代数都过滤代际不符的槽位。

## 消费者索引与脏节点

`State` 维护 `DepKey → Vec<ConsumerRef>` 的绑定消费者索引，以及按 `(TreeKey, NodeRef)` 的反向表。挂载文本、元素属性/样式、`Show::when`、`For` 的行集来源时登记消费者；节点随 owner 退休时删除对应边。消费边记录节点身份与种类（值绑定或结构触发），不跟随绑定槽的释放单独漂移。

来源写入、memo 失效、相等写入和字段写入尝试都会标脏对应节点：

- `dirty_value_nodes`：值阶段只对集合内节点调用 `refresh_element`/`refresh_text`，不再为每个挂载元素重建属性/样式计划；
- `dirty_structure_nodes`：结构阶段只重跑集合内的 `Show`/`For` 触发；未变脏的触发仍会下钻已挂载分支，保证独立变脏的嵌套触发被访问。

`For` 额外按 `StoreId` 登记 `DepKey::Projection`：通用 Store 的任一提交保守地重跑它的结构触发，`refresh_for` 用 `row_live` 剔除已删除行，并应用宿主行窗口。挂载编辑器与复选框即使只有静态值也标脏一次，使本地状态与绑定值完成首次对账。

值节点的 fault 会重新标脏以保持逐帧重试；结构触发的 fault/错误同样重新标脏。`memo_deps_unsettled` 使用 `State` 上的复用栈与 stamp 数组，不按调用分配集合。

相等写入不改变值代数，但文本输入的 `value` 绑定必须重新观察写入尝试以结算本地编辑。消费者因此区分普通值绑定与编辑器绑定：相等写入只标脏编辑器绑定，避免为一次相等写入把整张共享绑定表（例如 10k 行直接绑定同一 Signal）全部拉进值阶段。真实值变化仍标脏全部消费者。

## cleanup 与延迟释放

`on_cleanup(f)` 在挂载期把回调登记到当前 owner。cleanup 是用户代码，不能在 `State` 已被 `RefCell` 可变借用时执行：`Signal::get`、`Transaction::finish` 等公开入口会再次借用同一 State。因此 `dispose_owner` 只做摘除工作——标记 disposed、取走子 owner/资源/cleanup、清行带、回收元素与组件槽、从父链移除——并把 `(cleanups, resources)` 推入 `pending_disposals`。

`with_state`/`with_state_of` 在闭包返回并释放借用后调用 `drain_disposals`：

1. 批次按后序排列（子 owner 先于父 owner）；
2. `disposing` 计数加一，期间拒绝创建 Signal/Memo、写 Signal/Store；
3. 每个 owner 的回调按登记逆序执行，单条 panic 收敛为 `RuntimeDiagnostic::CleanupPanicked`，后续回调继续；
4. 全部 cleanup 结束后才释放资源（含 Signal/Memo 值与事件槽），因此回调仍能读取本 owner 与父 owner 的存活 Signal/Memo。

这一顺序也把用户 `Drop` 与 cleanup 移出了 State 借用窗口。释放批次本身可以再触发新的 disposed owner（cleanup 不能写，但运行时收尾可能），drain 循环到队列为空。

## 备选方案

- **给 `DepKey` 加 generation**：可以检测旧边，但释放历史仍保留边与反向表，且 `DepKey` 变宽影响所有依赖表；强制解绑更小且能复用现有布局。拒绝。
- **cleanup 在 `dispose_owner` 内直接执行**：用户回调重入 runtime 会触发 `RefCell already borrowed`；除非所有 dispose 调用点都改成两阶段 API，风险高于集中到 `with_state` 边界。拒绝。
- **cleanup 在资源释放后执行**：实现简单，但回调无法读取本 owner 的存活状态，与“最后保存草稿”类用法冲突。拒绝。
- **只维护 memo 反向边，绑定继续全量扫描**：绑定计划构建是每帧主要成本之一；消费者索引不改变求值语义，却能让干净节点完全跳过。采纳。
- **结构阶段继续全量求值 `Show`/`For`**：脏集合只决定是否重跑触发，未变脏的触发仍下钻分支；这保留嵌套结构正确性，同时避免无关行集重算。采纳。

## 后果

- StorageReport 增加 free list 长度、消费者边/节点、字段写入与失效代数、owner 记录；`memory_soak` 与 `tests/memory.rs` 用这些计数约束 churn 后的高水位。
- MemoryReport 增加 `signal_memo_bytes`（槽头、值估算与 memo 依赖容量）与 `owner_bytes`。值的堆字节是诊断估算：`String` 按容量、其他类型按 inline 大小，共享 `RowSet` 缓冲仍在 `rowset_bytes` 单列。
- 删除行时清除该业务键的 `field_writes` 与 `write_epochs` 条目；删除提交仍先标脏消费者，使保留的绑定本帧读到 `FieldMissing`。
- 结构触发仍沿保留树遍历（`Show`/`For` 的已挂载分支必须下钻）；“不再重扫全部绑定”指值绑定阶段，而不是取消结构遍历。
