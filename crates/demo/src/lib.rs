//! 共享的最小 Todo demo：headless 示例与 Windows 宿主挂载同一棵组件树。
//!
//! `seed` 提供一致的初始任务；组件只使用 SDK 的公开 API，不包含平台代码。
#![allow(
    unused_braces,
    reason = "{} is the intentionally supported event-expression syntax"
)]

use zenui::prelude::*;

fn report(result: Result<(), impl std::fmt::Display>, error: Signal<String>) {
    if let Err(failure) = result {
        error.set(failure.to_string());
    }
}

/// 过滤条件在界面上的标签；主窗口按钮与 Details 摘要共用同一份词表。
const fn filter_label(filter: Filter) -> &'static str {
    match filter {
        Filter::All => "All",
        Filter::Active => "Active",
        Filter::Completed => "Done",
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

/// 主 Todo 窗口的目录键；工具栏的 Details 动作打开名为 `details` 的附属窗口。
const MAIN_WINDOW: &str = "main";
/// 共享 Todo 模型的摘要窗口目录键；由主窗口工具栏按需打开或聚焦。
pub const DETAILS_WINDOW: &str = "details";

/// 创建应用级 Todo 状态、声明窗口目录并打开主窗口。
///
/// 状态必须先于窗口 factory 运行创建，因此本函数内部先运行 `Runtime::setup`，
/// 再把共享状态交给两个窗口的 factory。Details 窗口只在被请求时挂载；桌面
/// 宿主与 headless 示例共用这个入口。
pub fn open_todo_window(runtime: &mut Runtime) -> Result<WindowCtx, WindowError> {
    let state = runtime.setup(TodoState::new)?;
    runtime.declare_windows(move |catalog| {
        catalog.window(
            WindowSpec::new(MAIN_WINDOW)
                .primary()
                .titled("ZenUI Todo")
                .sized(900, 700),
            move |window| view! { <TodoApp window=window state=state /> },
        )?;
        catalog.window(
            WindowSpec::new(DETAILS_WINDOW)
                .titled("ZenUI Details")
                .sized(520, 420),
            move |window| view! { <TodoDetails window=window state=state /> },
        )
    })?;
    runtime.show(MAIN_WINDOW)
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
                <FilterButton
                    current={filter}
                    value={Filter::All}
                    label={filter_label(Filter::All)}
                />
                <FilterButton
                    current={filter}
                    value={Filter::Active}
                    label={filter_label(Filter::Active)}
                />
                <FilterButton
                    current={filter}
                    value={Filter::Completed}
                    label={filter_label(Filter::Completed)}
                />
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
                <button
                    aria:label="Open details"
                    on:click={move || report(window.request_show(DETAILS_WINDOW), task_error)}
                >"Details"</button>
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

/// Details 的一行：只读呈现共享标题。
///
/// 与 `TodoItem` 一样在挂载时读取行字段；失效读取沿 `?` 传播，不以空串
/// 伪装成功。剩余数量、过滤结果与行集合仍由 Memo 实时刷新。
#[component]
fn TodoDetailRow(todo: TodoId, todos: Store) -> MountResult<MountPlan> {
    let title = todos.title(todo).read_once()?;

    view! {
        <view role="listitem" class="flex-row items-center gap-3">
            <text>{once(title)}</text>
        </view>
    }
}

/// 与主窗口共享 TodoState 的摘要窗口。
///
/// remaining 与过滤结果随共享模型实时刷新；本地 note 输入保留这个窗口
/// 自己的焦点、caret 与草稿，与主窗口的编辑状态互不影响。
#[component]
pub fn TodoDetails(window: WindowCtx, state: TodoState) -> MountResult<MountPlan> {
    let TodoState {
        todos,
        filter,
        remaining_txt,
        no_tasks,
        no_matches,
        task_error,
        has_error,
        ..
    } = state;
    let visible = memo(move || todos.rows(filter.get()));
    let filter_text = memo(move || filter_label(filter.get()).to_owned());
    let note = signal(String::new());

    view! {
        <view role="main" class="p-4 gap-3">
            <text class="text-sm dark:text-muted">{remaining_txt}</text>
            <text class="font-semibold" aria:label="Details filter">{filter_text}</text>
            <text-input
                class="focus-visible:border-accent"
                value={note}
                aria:label="Details note"
                on:input={move |value: String| note.set(value)}
            />
            <view role="list" aria:label="Details rows" class="gap-3">
                <For each={visible} key={|id: TodoId| id}>
                    {move |todo: TodoId| view! { <TodoDetailRow todo={todo} todos={todos} /> }}
                </For>
            </view>
            <Show when={no_tasks}>
                <text>"Nothing here yet"</text>
            </Show>
            <Show when={no_matches}>
                <text>"Nothing matches"</text>
            </Show>
            <Show when={has_error}>
                <text>{task_error}</text>
            </Show>
            <view role="toolbar" class="gap-3">
                <button
                    class="px-3 py-1 rounded-sm bg-control hover:bg-control-hover"
                    aria:label="Hide details"
                    on:click={move || report(window.request_hide(), task_error)}
                >"Hide"</button>
                <button
                    class="px-3 py-1 rounded-sm bg-danger text-inverse"
                    aria:label="Close details"
                    on:click={move || report(window.request_close(), task_error)}
                >"Close"</button>
            </view>
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
