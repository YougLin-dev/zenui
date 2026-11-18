//! 目标语法预览，不能在当前 Zenui PoC 上运行。
//!
//! `MountResult<T> = Result<T, MountError>`，组件允许在 mount 时失败。
//! Store、完整组件宏、TextEdit、For 和事务错误处理尚未实现。

use zenui::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter { All, Active, Done }

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct TodoId(u64); // 业务身份，不是 ViewId；有效性仍由 row generation 检查。

#[derive(Clone, Copy)]
struct TodoRow { id: TodoId, completed: CompletedField }

#[derive(Clone, Copy)]
struct TodoStore(StoreId);

#[component]
fn TodoApp(todos: TodoStore) -> MountResult<MountPlan> {
    let draft = signal(String::new());
    let filter = signal(Filter::All);
    let error = signal(None::<String>);

    // 标量查询可复制读取；列表直接消费共享 RowSet，不需要整表 String/Vec clone。
    let visible = memo(move || todos.rows(filter.get()));
    let no_matches = memo(move || todos.is_empty(filter.get()));
    let has_rows = memo(move || todos.has_rows());
    let remaining = memo(move || todos.remaining());
    let has_completed = memo(move || todos.has_completed());

    let show_error = move |failure: AppError| {
        // 应用错误进入本 scope 的 visible error binding。
        // 若当前 UI 已被销毁，Signal 写入也不能静默成功或写到下一世代。
        if let Err(runtime_failure) = error.set(Some(failure.user_message())) {
            report_runtime_failure(runtime_failure);
        }
    };

    let add_todo = move || {
        let title = draft.with(|text| text.trim().to_owned());
        if title.is_empty() {
            return;
        }
        // 目标事务构造器：预演插入和草稿清理，任一项失败都不应用。
        // Transaction 是显式 API；不会假定普通两条 store 调用自动原子。
        if let Err(failure) = Transaction::new()
            .write(todos.insert(title))
            .write(draft.clear())
            .finish()
        {
            show_error(failure);
        }
    };

    view! {
        <view role="main" class="mx-auto flex w-full max-w-xl flex-col gap-6 bg-zinc-50 p-6 text-zinc-950">
            <header class="flex flex-col gap-1">
                <text class="text-2xl font-bold">"TodoList"</text>
                <text class="text-sm text-zinc-500">
                    {move || format!("{} remaining", remaining.get())}
                </text>
            </header>

            <form class="flex gap-2" on:submit={move |event: SubmitEvent| {
                // 原生命令事件：button activation 或正常 Enter 只会路由一次。
                // IME 候选确认 Enter 已由 TextEdit 消费，不能走到这里。
                event.prevent_default();
                add_todo();
            }}>
                <text-input
                    class="flex-1 rounded-md border border-zinc-300 bg-white px-3 py-2 focus-visible:ring-2"
                    aria:label="What needs to be done?"
                    placeholder="Add a task"
                    // value 使外部 draft 更新控件；on:input 才把用户写回。
                    // 专属适配器知道输入来源，不把同一次本地输入当外部值覆盖。
                    value={draft}
                    on:input={move |value: String| {
                        if let Err(failure) = draft.set(value) {
                            report_runtime_failure(failure);
                        }
                    }}
                />
                <button
                    type="submit"
                    class="rounded-md bg-blue-600 px-4 py-2 text-white hover:bg-blue-500 disabled:opacity-50"
                    disabled={move || draft.with(|text| text.trim().is_empty())}
                >
                    "Add"
                </button>
            </form>

            <Show when={move || error.with(Option::is_some)}>
                <text role="alert" class="rounded-md bg-rose-50 p-3 text-rose-900">
                    {move || error.with(Clone::clone).unwrap_or_default()}
                </text>
            </Show>

            <view role="group" aria:label="Filter todos" class="flex gap-1">
                // 这里把信号 handle 当作普通 typed prop 传递；不是它当前的 Filter 值。
                <FilterButton current={filter} value={Filter::All} label="All" />
                <FilterButton current={filter} value={Filter::Active} label="Active" />
                <FilterButton current={filter} value={Filter::Done} label="Done" />
            </view>

            <Show when={move || !has_rows.get()}>
                <text class="rounded-lg bg-white p-4 text-center text-zinc-500">
                    "No tasks yet. Add your first task."
                </text>
            </Show>
            <Show when={move || has_rows.get()}>
                <view role="list" aria:label="Tasks" class="flex flex-col gap-2">
                    <Show when={move || no_matches.get()}>
                        <text class="rounded-lg bg-white p-4 text-center text-zinc-500">
                            "No tasks match this filter."
                        </text>
                    </Show>
                    // key 借用 row 取业务 id；child factory 接收含小字段 handle 的 TodoRow。
                    <For each={visible} key={|row: &TodoRow| row.id}>
                        {move |row: TodoRow| view! {
                            <TodoItem todo={row} store={todos} error={error} />
                        }}
                    </For>
                </view>
            </Show>

            <Show when={move || has_completed.get()}>
                <button
                    class="rounded-md px-3 py-2 text-start text-rose-600 hover:bg-rose-50"
                    on:click={move || {
                        if let Err(failure) = todos.clear_completed() {
                            show_error(failure);
                        }
                    }}
                >
                    "Clear completed"
                </button>
            </Show>
        </view>
    }
}

#[component]
fn FilterButton(current: Signal<Filter>, value: Filter, label: &'static str) -> MountResult<MountPlan> {
    // 命名闭包与 {move || current.get() == value} 相同；不是一次性的 bool。
    // aria:pressed 和样式条件由同一个状态派生。
    let selected = move || current.get() == value;
    view! {
        <button
            class="rounded-md bg-zinc-100 px-3 py-1 text-sm text-zinc-900 aria-pressed:bg-blue-600 aria-pressed:text-white"
            aria:pressed={selected}
            on:click={move || {
                if let Err(failure) = current.set(value) {
                    report_runtime_failure(failure);
                }
            }}
        >
            {label}
        </button>
    }
}

#[component]
fn TodoItem(todo: TodoRow, store: TodoStore, error: Signal<Option<String>>) -> MountResult<MountPlan> {
    let completed = todo.completed;
    let id = todo.id;
    // 只有一次性字段读取需要拥有 String；失效直接取消本次 mount。
    // `?` 的实际 Rust 形状已通过独立类型夹具验证，但未验证未来宏降级。
    let title = store.title(id).read_once()?;
    // 两个名称各自拥有字符串；事件闭包不借用 mount 栈，也不重读 Store。
    let complete_label = format!("Complete {title}");
    let delete_label = format!("Delete {title}");
    let show_error = move |failure: AppError| {
        if let Err(runtime_failure) = error.set(Some(failure.user_message())) {
            report_runtime_failure(runtime_failure);
        }
    };

    view! {
        <view role="listitem" class="flex items-center gap-3 rounded-lg bg-white p-3 shadow-sm">
            <checkbox
                checked={move || completed.get()}
                on:change={move |checked: bool| {
                    if let Err(failure) = store.set_completed(id, checked) {
                        show_error(failure);
                    }
                }}
                aria:label={complete_label}
            />
            <text
                // 动态样式是 typed 值；不接受运行时拼接的任意 class 字符串。
                style:color={move || if completed.get() {
                    rgb(0xa1a1aa)
                } else {
                    rgb(0x09090b)
                }}
                style:text-decoration-line={move || if completed.get() {
                    TextDecoration::LineThrough
                } else {
                    TextDecoration::None
                }}
            >
                {title}
            </text>
            <button
                class="ms-auto rounded-md p-2 text-rose-600 hover:bg-rose-50"
                aria:label={delete_label}
                on:click={move || {
                    if let Err(failure) = store.remove(id) {
                        show_error(failure);
                    }
                }}
            >
                "Delete"
            </button>
        </view>
    }
}
