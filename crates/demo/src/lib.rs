//! 共享的最小 Todo demo：headless 示例与 Windows 宿主挂载同一棵组件树。
//!
//! `seed` 提供一致的初始任务；组件只使用 SDK 的公开 API，不包含平台代码。
#![allow(
    unused_braces,
    reason = "{} is the intentionally supported event-expression syntax"
)]

use zenui::prelude::*;

fn report(result: Result<(), TaskError>, error: Signal<String>) {
    if let Err(failure) = result {
        error.set(failure.to_string());
    }
}

#[component]
pub fn FilterButton(
    current: Signal<Filter>,
    value: Filter,
    label: &'static str,
) -> MountResult<MountPlan> {
    let selected = memo(move || current.get() == value);

    view! {
        <button
            class="px-3 py-1 rounded-sm bg-control hover:bg-control-hover active:bg-control-pressed aria-pressed:bg-accent aria-pressed:text-inverse"
            aria:pressed={selected}
            on:click={move || current.set(value)}
        >{label}</button>
    }
}

#[component]
pub fn TodoItem(todo: TodoId, todos: Store) -> MountResult<MountPlan> {
    let title = todos.title(todo).read_once()?;
    let completed = memo(move || todos.completed(todo).get());
    let tint = memo(move || {
        if completed.get().is_ok_and(|done| done) {
            Color::Role(ColorRole::TextMuted)
        } else {
            Color::Role(ColorRole::TextDefault)
        }
    });
    let editing = signal(String::new());
    let complete_label = format!("Complete {title}");
    let edit_label = format!("Edit {title}");
    let delete_label = format!("Delete {title}");

    view! {
        <view role="listitem">
            <checkbox
                checked={completed}
                aria:label={once(complete_label)}
                on:change={move |done: bool| {
                    let _ = Transaction::new()
                        .write(todos.set_completed(todo, done))
                        .finish();
                }}
            />
            <text style:color={tint}>{once(title.clone())}</text>
            <text-input
                class="focus-visible:border-accent"
                value={editing}
                aria:label={once(edit_label)}
                on:input={move |value: String| {
                    editing.set(value.clone());
                    let _ = Transaction::new().write(todos.set_title(todo, value)).finish();
                }}
            />
            <button
                class="px-2 py-1 bg-danger text-inverse rounded-sm"
                aria:label={once(delete_label)}
                on:click={move || {
                    let _ = Transaction::new().write(todos.remove(todo)).finish();
                }}
            >"Delete"</button>
        </view>
    }
}

/// 应用层共享的 Todo 状态；在窗口挂载前通过 `Runtime::setup` 创建。
///
/// 所有字段都是 Copy handle；窗口关闭只释放窗口 owner，应用状态保留。
#[derive(Clone, Copy)]
pub struct TodoState {
    pub todos: Store,
    pub draft: Signal<String>,
    pub filter: Signal<Filter>,
    pub preview: Signal<String>,
    pub task_error: Signal<String>,
    remaining_txt: Memo<String>,
    no_tasks: Memo<bool>,
    no_matches: Memo<bool>,
    has_error: Memo<bool>,
    has_preview: Memo<bool>,
}

impl TodoState {
    /// 在 `Runtime::setup` 中调用，创建归属于应用 owner 的 Store 与派生值。
    #[expect(
        clippy::new_without_default,
        reason = "TodoState is only valid inside Runtime::setup, so it has no Default"
    )]
    pub fn new() -> Self {
        let todos = Store::new();
        let filter = signal(Filter::All);
        let remaining = memo(move || todos.rows(Filter::Active).len());
        let remaining_txt = memo(move || format!("{} remaining", remaining.get()));
        let no_tasks = memo(move || todos.is_empty(Filter::All));
        let no_matches = memo(move || !no_tasks.get() && todos.is_empty(filter.get()));
        let task_error = signal(String::new());
        let preview = signal(String::new());
        let has_error = memo(move || !task_error.get().is_empty());
        let has_preview = memo(move || !preview.get().is_empty());
        Self {
            todos,
            draft: signal(String::new()),
            filter,
            preview,
            task_error,
            remaining_txt,
            no_tasks,
            no_matches,
            has_error,
            has_preview,
        }
    }
}

/// 创建应用级 Todo 状态、声明单窗口目录并打开主窗口。
///
/// 状态必须先于窗口 factory 运行创建，因此本函数内部先运行 `Runtime::setup`，
/// 再把共享状态交给目录 factory。桌面宿主与 headless 示例共用这个入口。
pub fn open_todo_window(runtime: &mut Runtime) -> Result<WindowCtx, WindowError> {
    let state = runtime.setup(TodoState::new)?;
    runtime.declare_windows(move |catalog| {
        catalog.window(
            WindowSpec::new("main")
                .primary()
                .titled("ZenUI Todo")
                .sized(900, 700),
            move |window| view! { <TodoApp window=window state=state /> },
        )
    })?;
    runtime.show("main")
}

#[component]
pub fn TodoApp(window: WindowCtx, state: TodoState) -> MountResult<MountPlan> {
    let TodoState {
        todos,
        draft,
        filter,
        preview,
        task_error,
        remaining_txt,
        no_tasks,
        no_matches,
        has_error,
        has_preview,
    } = state;
    let visible = memo(move || todos.rows(filter.get()));
    let dark = memo(move || {
        window
            .appearance()
            .is_ok_and(|value| value == Appearance::Dark)
    });

    let add = move || {
        let title = draft.with(|text| text.trim().to_owned());
        if title.is_empty() {
            return;
        }
        let result = Transaction::new().write(todos.insert(title)).finish();
        if result.is_ok() {
            draft.set(String::new());
        }
    };
    let reverse = move || {
        let mut ids: Vec<TodoId> = visible.get().iter().map(TodoId::from_key).collect();
        ids.reverse();
        let _ = Transaction::new().write(todos.reorder(&ids)).finish();
    };
    let load_sample = move || {
        let result = (|| -> Result<(), TaskError> {
            let task = window
                .app_task_scope()
                .spawn(|_| vec![String::from("Imported A"), String::from("Imported B")])?;
            task.on_done(move |titles| {
                let mut transaction = Transaction::new();
                for title in titles {
                    transaction = transaction.write(todos.insert(title));
                }
                transaction.finish()?;
                Ok(())
            })
        })();
        report(result, task_error);
    };
    let preview_import = move || {
        let result = (|| -> Result<(), TaskError> {
            let task = window
                .task_scope()
                .spawn(|_| String::from("preview ready"))?;
            task.on_done(move |text| {
                preview.set(text);
                Ok(())
            })
        })();
        report(result, task_error);
    };

    view! {
        <view role="main">
            <text class="text-sm dark:text-muted">{remaining_txt}</text>
            <form
                aria:label="Add task"
                on:submit={move |event: SubmitEvent| {
                    let mut event = event;
                    event.prevent_default();
                    add();
                }}
            >
                <text-input
                    class="focus-visible:border-accent"
                    value={draft}
                    aria:label="What needs to be done?"
                    on:input={move |value: String| draft.set(value)}
                />
                <button
                    class="px-4 py-2 bg-accent text-inverse rounded-md font-semibold"
                    on:click={move || add()}
                >"Add"</button>
            </form>
            <view role="group" aria:label="Filter">
                <FilterButton current={filter} value={Filter::All} label="All" />
                <FilterButton current={filter} value={Filter::Active} label="Active" />
                <FilterButton current={filter} value={Filter::Completed} label="Done" />
            </view>
            <view role="toolbar" class="gap-3">
                <button on:click={move || reverse()}>"Reverse"</button>
                <button on:click={move || {
                    let _ = Transaction::new().write(todos.clear_completed()).finish();
                }}>"Clear done"</button>
                <button on:click={move || load_sample()}>"Load sample"</button>
                <button on:click={move || preview_import()}>"Preview import"</button>
                <button
                    aria:label="Toggle theme"
                    on:click={move || {
                        let next = if dark.get() { Appearance::Light } else { Appearance::Dark };
                        let _ = window.set_appearance(next);
                    }}
                >"Theme"</button>
            </view>
            <Show when={has_error}>
                <text>{task_error}</text>
            </Show>
            <Show when={has_preview}>
                <text>{preview}</text>
            </Show>
            <Show when={no_tasks}>
                <text>"Nothing here yet"</text>
            </Show>
            <Show when={no_matches}>
                <text>"Nothing matches"</text>
            </Show>
            <For each={visible} key={|id: TodoId| id}>
                {move |todo: TodoId| view! { <TodoItem todo={todo} todos={todos} /> }}
            </For>
        </view>
    }
}

/// 预置两条任务：Write tests 已完成，Read docs 未完成；编辑框显示标题。
///
/// 宿主与示例共用同一初始状态，保证原生窗口与 headless 场景看到同一棵树。
pub fn seed(runtime: &mut Runtime) -> Result<(), MountError> {
    runtime.input("What needs to be done?", "Write tests")?;
    runtime.submit("Add task")?;
    runtime.input("What needs to be done?", "Read docs")?;
    runtime.submit("Add task")?;
    runtime.change("Complete Write tests", true)?;
    runtime.input("Edit Write tests", "Write tests")?;
    runtime.input("Edit Read docs", "Read docs")?;
    Ok(())
}
