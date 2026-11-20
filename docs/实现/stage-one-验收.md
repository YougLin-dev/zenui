# Stage-one headless Todo 验收

本页记录阶段一完成判据的可执行验收结果。结论只覆盖 headless target API；不把 headless 行为换算成原生 Windows、IME、焦点或呈现的完成度，也不给出总体百分比。

验收日期：2026-11-20（对应 PR #57、#58、#59 合并后的 `main`，`30381cf`）。

## 验收对象与方法

对象是根包 `zenui` 的 target API 与 `examples/minimal_todo.rs`，包括：

- Copy Signal/Memo/Store Field、共享 RowSet 与安装在运行时上的执行上下文；
- mount-once `#[component]`、`view!` 数据位、typed events、`Show`/`For`；
- `Transaction`/`WriteOp` 模型提交；
- headless 窗口 activation 与 app/window 任务 ownership、worker 交付边界；
- 有界发布历史、Owner 存储与索引化 Todo reconciliation。

验收命令：

```sh
cargo run -p zenui --example minimal_todo --locked
cargo test -p zenui --locked --test stage_one_acceptance --test minimal_todo \
    --test acceptance --test tasks --test sdk
cargo test -p zenui --doc --locked
cargo test -p zenui-core --locked
cargo test -p zenui-app --locked
```

`--test` 过滤不包含 doctest；跨线程句柄边界的 4 个 `compile_fail` doctest 由 `cargo test -p zenui --doc` 单独运行。

`minimal_todo` 的实际输出（headless）：

```text
active: 1 remaining | Add | All | Active | Done | Reverse | Clear done | Load sample | Preview import | Read docs | Delete
all: 0 remaining | Add | All | Active | Done | Reverse | Clear done | Load sample | Preview import | Write tests | Delete
tasks: delivered=2 | 2 remaining | Add | All | Active | Done | Reverse | Clear done | Load sample | Preview import | preview ready | Write tests | Delete | Imported A | Delete | Imported B | Delete
```

这证明同一个 target Todo 走完输入、添加、完成、筛选、重排、删除、窗口任务和 app 任务；它不证明原生呈现。

`view!` 数据位由 `IntoText`/`IntoNode` 等类型接受 handle、字面量与 `once`，没有对任意零参数闭包的实现；事件位才是 typed handler 闭包。示例中的 `FilterButton`、`TodoItem` 与 `For` factory 捕获 Copy handle；`title.clone()` 只为 `once` 文本位保留一个独立字符串，`value.clone()` 在 input handler 中把值同时交给本地编辑状态和模型事务，其余分支不复制状态。

## 路线图完成判据对照

| 路线图判据 | 可执行证据 | 结果 |
| --- | --- | --- |
| 最小 Todo 使用目标 API 运行，数据位不依赖任意零参数闭包或层层 clone | `tests/stage_one_acceptance.rs::the_target_todo_completes_the_stage_one_workflow` 在同一 runtime 完成 add/toggle/filter/reorder/remove；`tests/minimal_todo.rs::typed_data_slots_distinguish_reactive_literal_and_once`；类型层面 `IntoText`/`IntoNode` 不接受闭包 | 通过（headless） |
| 字段更新不重跑组件 | `tests/minimal_todo.rs::a_field_update_does_not_rerun_the_row_component`；验收工作流在 toggle 后断言 `setup_count("TodoItem")` 与 `once_reads()` 不变 | 通过（headless） |
| 列表重排保留行身份 | `tests/minimal_todo.rs::reorder_keeps_row_identity_owner_and_local_editing_state`；验收工作流断言 instance/Owner/caret 保留且 `unmounts` 不增加 | 通过（headless） |
| 业务删除与界面卸载不互相替代 | `tests/minimal_todo.rs::deleting_a_row_removes_the_model_and_interface`；验收工作流断言删除后行与文本消失，模型行由 Store 事务删除 | 通过（headless） |
| 跨 Runtime、已释放 Owner、代际回收、非法写入、失败后恢复 | 见下方“边界与恢复证据” | 通过（headless） |
| 验证目标 SDK 的实际行为，由对应 API 和测试定义实现 | 根包证据通过 target API 与 `tests/`；底层语义由 `crates/core/tests/`、`crates/app/tests/` 的公开行为测试补充，单元测试只作辅助 | 通过（headless） |

## 边界与恢复证据

| 行为 | 具名证据 |
| --- | --- |
| 跨 Runtime 句柄拒绝 | `tests/acceptance.rs::a_handle_cannot_cross_runtimes`、`a_store_cannot_write_across_runtimes`；`tests/tasks.rs::handles_from_another_runtime_are_clear_errors` |
| 已释放 Owner 的资源释放 | `tests/acceptance.rs::slot_factory_resources_are_released_with_their_branch`、`a_failing_component_keeps_committed_siblings_and_releases_setup_state`；`tests/tasks.rs::reopening_uses_a_new_activation_and_discards_the_old_delivery`；`crates/core/tests/lifecycle.rs::churn_returns_to_the_baseline` |
| 代际回收不别名旧身份 | `crates/core/tests/transactions.rs::stale_handle_does_not_alias_the_reused_slot`、`a_disposed_view_cannot_target_its_replacement`；`tests/tasks.rs::reopening_uses_a_new_activation_and_discards_the_old_delivery` |
| 非法 phase/context 在状态变化前拒绝 | `tests/acceptance.rs::illegal_context_writes_fail_before_state_changes_and_recover`、`signal_with_rejects_writes_without_changing_state`、`a_setup_write_is_rejected_and_the_mount_can_retry`；`tests/tasks.rs::delivery_obeys_context_checks_and_later_delivery_recovers` |
| 挂载失败清理与重试 | `tests/acceptance.rs::a_failing_component_keeps_committed_siblings_and_releases_setup_state`、`a_failing_root_plan_releases_its_setup_resources`；`tests/tasks.rs::a_failing_window_factory_releases_resources_and_can_retry`、`a_failing_child_mount_aborts_the_window_plan_and_can_retry`、`a_binding_fault_aborts_the_window_plan_and_can_retry` |
| 失败 setup 启动的任务被撤销 | `tests/tasks.rs::a_failed_window_factory_cancels_tasks_started_in_its_setup` |
| 过滤返场读取新 once 数据 | `tests/minimal_todo.rs::filtering_remounts_rows_with_a_fresh_once_read` |

## Worker、窗口与交付证据

| 行为 | 具名证据 |
| --- | --- |
| 窗口 activation 内交付 | `tests/tasks.rs::a_window_owned_task_delivers_inside_its_activation` |
| 窗口关闭取消窗口所有权交付 | `tests/tasks.rs::closing_a_window_cancels_window_owned_delivery`、`a_closed_window_cancels_status_and_rejects_late_registration` |
| 重开使用新代际，旧交付不命中新界面 | `tests/tasks.rs::reopening_uses_a_new_activation_and_discards_the_old_delivery` |
| app 所有权任务跨窗口提交共享模型 | `tests/tasks.rs::an_app_owned_task_commits_the_shared_model_after_the_window_closes` |
| worker 完成、continuation 与 UI 交付是不同事实 | `tests/tasks.rs::completion_is_distinct_from_delivery_and_a_late_continuation_runs`、`task_status_tracks_completion_and_delivery` |
| 交付仍受执行上下文检查 | `tests/tasks.rs::delivery_obeys_context_checks_and_later_delivery_recovers`、`an_app_continuation_writing_a_disposed_window_signal_is_rejected`、`a_panicking_read_scope_does_not_poison_later_delivery` |
| UI 句柄不能进入 worker | `src/task.rs` 的 4 个 `compile_fail` doctest（`Signal`、`Store`、`WindowCtx`、`Memo`），以及 `TaskScope::spawn` 的 `Send` 约束 |

## 有界运行时证据

| 行为 | 具名证据 |
| --- | --- |
| 发布历史有界且序号新鲜 | `crates/core/tests/transactions.rs::publication_history_is_bounded_and_keeps_newest_sequences`；`crates/app/tests/bounded_state.rs::focus_only_frames_keep_a_bounded_publication_history` |
| 空帧/未变化列表不产生结构工作 | `crates/app/tests/bounded_state.rs::unchanged_reconciliation_does_no_structural_work_and_stays_bounded` |
| Owner 存储回到存活规模 | `crates/core/tests/lifecycle.rs::churn_returns_to_the_baseline`；`crates/app/tests/bounded_state.rs::filter_churn_returns_owner_storage_to_the_live_set` |

## 已完成（headless）与未完成（native）

**已完成并通过可执行证据固定：**

- target `view!`、mount-once 组件、数据位、typed events、`Show`/`For`、`once`；
- Copy Signal/Memo/Store Field、共享 RowSet、依赖更新与不重跑组件；
- 模型事务、字段更新、重排、删除、筛选返场的一次性读取；
- headless 窗口 activation、app/window 任务 ownership、worker 的 `Send` 边界与代际交付检查；
- 有界发布历史、Owner 存储与索引化 reconciliation。

**未完成，不在本页声称已完成：**

| 缺口 | 为什么现在没有证据 | 窄 blocker |
| --- | --- | --- |
| 原生 Windows 窗口与 `view!` 宿主 | `Runtime::open_window` 只是 headless activation；根二进制仍是 `Hello, world!`，没有 HWND 或原生事件循环 | [#60](https://github.com/YougLin-dev/zenui/issues/60) |
| IME composition | target event 只有 click/input/change/submit/keydown；`on:input` 是 `String`，无法表达 preedit/commit/cancel，也没有候选 Enter 不提交的测试 | [#61](https://github.com/YougLin-dev/zenui/issues/61) |
| 原生焦点适配 | headless target runtime 已提供 `on:focus`/`on:blur`、`focus`、`focus_next`/`focus_previous` 与重挂/重排后的焦点结算（`tests/focus.rs`）；Windows 原生焦点适配仍未实现 | [#62](https://github.com/YougLin-dev/zenui/issues/62)（headless 部分完成，原生适配待做） |
| 原生布局、文本、GPU、DPI 呈现 | target Todo 只通过 `texts()` 观察；`native_poc`/`desktop_poc` 是独立原型，不消费 `MountPlan` | [#63](https://github.com/YougLin-dev/zenui/issues/63) |

这些缺口是阶段二/三的工作。阶段一验收不因它们失败，也不把 headless 测试当作原生完成的替代。

## 复现说明

上述命令全部通过；具体行为由上表的具名测试固定。`cargo test -p zenui --doc` 的 4 个 `compile_fail` 用例验证 UI 句柄不能进入 worker。`crates/app/tests/golden_trace.rs::trace_matches_golden` 保持字节一致。

本页不统计类型、原型或测试数量，也不把它们换算为完成度。

## 结论

阶段一的 headless 完成判据全部有可复跑证据；原生 Windows `view!` 集成、IME、焦点和原生呈现仍缺失，已分别记录为 #60–#63 的窄 blocker。当前仓库不声明原生窗口、真实 IME、Windows 或跨平台完成。
