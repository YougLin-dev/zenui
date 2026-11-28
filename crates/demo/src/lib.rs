//! 共享的最小 Todo demo：headless 示例与 Windows 宿主挂载同一棵组件树。
//!
//! `seed` 提供一致的初始任务；组件只使用 SDK 的公开 API，不包含平台代码。
#![allow(
    unused_braces,
    reason = "{} is the intentionally supported event-expression syntax"
)]

use std::path::{Path, PathBuf};

use zenui::prelude::*;

fn report(result: Result<(), impl std::fmt::Display>, error: Signal<String>) {
    if let Err(failure) = result {
        error.set(failure.to_string());
    }
}

/// 导入文本的标题：一行一个，去掉首尾空白，跳过空行。
fn import_titles(text: &str) -> impl Iterator<Item = &str> {
    text.lines().map(str::trim).filter(|line| !line.is_empty())
}

/// 读取所有选中的文件；任一文件读取失败时不返回部分结果。
fn read_import(paths: &[PathBuf]) -> Result<Vec<String>, String> {
    let mut titles = Vec::new();
    for path in paths {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
        titles.extend(import_titles(&text).map(str::to_owned));
    }
    Ok(titles)
}

/// 失败时的统一提示迁移：清掉旧提示，显示带上下文前缀的错误。
fn fail(notice: Signal<String>, task_error: Signal<String>, message: String) {
    notice.set(String::new());
    task_error.set(message);
}

/// 成功时的统一提示迁移：显示结果并清掉旧错误。
fn succeed(notice: Signal<String>, task_error: Signal<String>, message: String) {
    notice.set(message);
    task_error.set(String::new());
}

/// 文件对话框返回后启动应用所有权 worker；取消与失败都不改变任务列表。
fn begin_import(
    window: WindowCtx,
    todos: Store,
    notice: Signal<String>,
    task_error: Signal<String>,
    outcome: FileDialogOutcome,
) -> Result<(), MountError> {
    match outcome {
        FileDialogOutcome::Picked(paths) => start_import(window, todos, notice, task_error, paths),
        FileDialogOutcome::Cancelled => Ok(()),
        FileDialogOutcome::Failed(error) => {
            fail(notice, task_error, error.to_string());
            Ok(())
        }
    }
}

/// 选择器与文件拖放共用的导入 worker；任务属于应用，窗口关闭后仍可提交。
fn start_import(
    window: WindowCtx,
    todos: Store,
    notice: Signal<String>,
    task_error: Signal<String>,
    paths: Vec<PathBuf>,
) -> Result<(), MountError> {
    let task = window
        .app_task_scope()
        .spawn(move |_| read_import(&paths))
        .map_err(|_| MountError::Component("start the import worker"))?;
    task.on_done(move |outcome| {
        let titles = match outcome {
            Ok(titles) => titles,
            Err(message) => {
                fail(notice, task_error, format!("Import failed: {message}"));
                return Ok(());
            }
        };
        let count = titles.len();
        let mut transaction = Transaction::new();
        for title in titles {
            transaction = transaction.write(todos.insert(title));
        }
        transaction.finish()?;
        succeed(
            notice,
            task_error,
            match count {
                1 => String::from("Imported 1 task"),
                _ => format!("Imported {count} tasks"),
            },
        );
        Ok(())
    })
    .map_err(|_| MountError::Component("register the import continuation"))
}

fn read_titles(fields: impl IntoIterator<Item = Field<String>>) -> Result<Vec<String>, String> {
    fields
        .into_iter()
        .map(|field| field.read_once().map_err(|error| error.to_string()))
        .collect()
}

/// 按已提交顺序读取全部行；当前过滤只影响呈现，不影响保存内容。
fn committed_titles(todos: Store) -> Result<Vec<String>, String> {
    read_titles(
        todos
            .keys(Filter::All)
            .into_iter()
            .map(|id| todos.title(id)),
    )
}

/// 写入一个 UTF-8 文本文件：每个标题一行，每行以 `\n` 结束。
///
/// 换行会破坏行格式，因此在写入任何字节前拒绝；空列表产生空文件，
/// 空标题产生一个空行。
fn save_titles(path: &Path, titles: &[String]) -> Result<usize, String> {
    if titles
        .iter()
        .any(|title| title.contains('\n') || title.contains('\r'))
    {
        return Err(String::from("a task title contains a line break"));
    }
    let mut text = String::new();
    for title in titles {
        text.push_str(title);
        text.push('\n');
    }
    std::fs::write(path, text)
        .map_err(|error| format!("Could not write {}: {error}", path.display()))?;
    Ok(titles.len())
}

fn saved_message(count: usize) -> String {
    match count {
        1 => String::from("Saved 1 task"),
        _ => format!("Saved {count} tasks"),
    }
}

fn begin_save(
    window: WindowCtx,
    todos: Store,
    notice: Signal<String>,
    task_error: Signal<String>,
    outcome: FileDialogOutcome,
) -> Result<(), MountError> {
    let path = match outcome {
        FileDialogOutcome::Picked(mut paths) => {
            let Some(path) = paths.pop() else {
                fail(
                    notice,
                    task_error,
                    String::from("Save failed: the dialog returned no path"),
                );
                return Ok(());
            };
            path
        }
        FileDialogOutcome::Cancelled => return Ok(()),
        FileDialogOutcome::Failed(error) => {
            fail(notice, task_error, format!("Save failed: {error}"));
            return Ok(());
        }
    };
    let titles = match committed_titles(todos) {
        Ok(titles) => titles,
        Err(message) => {
            fail(notice, task_error, format!("Save failed: {message}"));
            return Ok(());
        }
    };
    let task = window
        .app_task_scope()
        .spawn(move |_| save_titles(&path, &titles))
        .map_err(|_| MountError::Component("start the save worker"))?;
    task.on_done(move |result| {
        match result {
            Ok(count) => succeed(notice, task_error, saved_message(count)),
            Err(message) => fail(notice, task_error, format!("Save failed: {message}")),
        }
        Ok(())
    })
    .map_err(|_| MountError::Component("register the save continuation"))
}

fn begin_folder(notice: Signal<String>, task_error: Signal<String>, outcome: FileDialogOutcome) {
    match outcome {
        FileDialogOutcome::Picked(mut paths) => {
            let Some(path) = paths.pop() else {
                fail(
                    notice,
                    task_error,
                    String::from("Choose folder failed: the dialog returned no path"),
                );
                return;
            };
            succeed(notice, task_error, format!("Folder: {}", path.display()));
        }
        FileDialogOutcome::Cancelled => {}
        FileDialogOutcome::Failed(error) => {
            fail(notice, task_error, format!("Choose folder failed: {error}"));
        }
    }
}

/// 发起一次文件对话框请求，并注册返回后的续期。
///
/// 请求或续期注册在本地失败时走与其他失败相同的提示迁移，避免旧的
/// `Saved N tasks` / `Folder: …` 留在新错误旁边。
fn request_dialog(
    window: WindowCtx,
    options: FileDialogOptions,
    notice: Signal<String>,
    task_error: Signal<String>,
    failure_prefix: &str,
    on_outcome: impl FnOnce(FileDialogOutcome) -> Result<(), MountError> + 'static,
) {
    let result = window
        .request_file_dialog(options, window.app_task_scope())
        .map_err(|error| error.to_string())
        .and_then(|picker| {
            picker
                .on_done(on_outcome)
                .map_err(|error| error.to_string())
        });
    if let Err(message) = result {
        fail(notice, task_error, format!("{failure_prefix}: {message}"));
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
    // `checked` 直接绑定字段：行失效时绑定保留最后值并报告 typed fault。
    let completed = todos.completed(todo);
    let done = memo(move || completed.get().is_ok_and(|done| done));
    let tint = memo(move || {
        if done.get() {
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
                on:change={move |event: &mut ChangeEvent| {
                    let done = event.proposed();
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
                on:input={move |event: &mut InputEvent| {
                    let value = event.value().to_owned();
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
    /// 最近一次导入、保存或选择文件夹的共享提示；为空时不渲染。
    pub notice: Signal<String>,
    pub task_error: Signal<String>,
    remaining_txt: Memo<String>,
    no_tasks: Memo<bool>,
    no_matches: Memo<bool>,
    has_error: Memo<bool>,
    has_notice: Memo<bool>,
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
        let notice = signal(String::new());
        let has_error = memo(move || !task_error.get().is_empty());
        let has_notice = memo(move || !notice.get().is_empty());
        Self {
            todos,
            draft: signal(String::new()),
            filter,
            notice,
            task_error,
            remaining_txt,
            no_tasks,
            no_matches,
            has_error,
            has_notice,
        }
    }
}

/// 主 Todo 窗口的目录键；工具栏的 Details 动作打开名为 `details` 的附属窗口。
const MAIN_WINDOW: &str = "main";
/// 共享 Todo 模型的摘要窗口目录键；由主窗口工具栏按需打开或聚焦。
pub const DETAILS_WINDOW: &str = "details";
/// 主窗口有待保存草稿时的关闭确认窗口目录键。
pub const CLOSE_CONFIRMATION_WINDOW: &str = "close";

/// 创建应用级 Todo 状态、声明窗口目录并打开主窗口。
///
/// 状态必须先于窗口 factory 运行创建，因此本函数内部先运行 `Runtime::setup`，
/// 再把共享状态交给窗口的 factory。Details 窗口只在被请求时挂载；主窗口采用
/// `Confirm` 关闭策略，草稿非空时显示关闭确认窗口。桌面宿主与 headless 示例
/// 共用这个入口。
pub fn open_todo_window(runtime: &mut Runtime) -> Result<WindowCtx, WindowError> {
    let state = runtime.setup(TodoState::new)?;
    runtime.declare_windows(move |catalog| {
        catalog.window(
            WindowSpec::new(MAIN_WINDOW)
                .primary()
                .titled("ZenUI Todo")
                .sized(900, 700)
                .close_policy(ClosePolicy::Confirm),
            move |window| {
                window
                    .on_close(move |event| {
                        if state.draft.with(String::is_empty) {
                            report(event.request().allow(), state.task_error);
                            CloseOutcome::Allow
                        } else {
                            report(
                                window.request_show(CLOSE_CONFIRMATION_WINDOW),
                                state.task_error,
                            );
                            CloseOutcome::Pending
                        }
                    })
                    .map_err(|_| MountError::Component("register the todo close handler"))?;
                window
                    .on_drop(move |drop| {
                        report(
                            start_import(
                                window,
                                state.todos,
                                state.notice,
                                state.task_error,
                                drop.into_paths(),
                            ),
                            state.task_error,
                        );
                    })
                    .map_err(|_| MountError::Component("register the todo drop handler"))?;
                view! { <TodoApp window=window state=state /> }
            },
        )?;
        catalog.window(
            WindowSpec::new(DETAILS_WINDOW)
                .titled("ZenUI Details")
                .sized(520, 420),
            move |window| {
                window
                    .on_drop(move |drop| {
                        report(
                            start_import(
                                window,
                                state.todos,
                                state.notice,
                                state.task_error,
                                drop.into_paths(),
                            ),
                            state.task_error,
                        );
                    })
                    .map_err(|_| MountError::Component("register the details drop handler"))?;
                view! { <TodoDetails window=window state=state /> }
            },
        )?;
        catalog.window(
            WindowSpec::new(CLOSE_CONFIRMATION_WINDOW)
                .confirmation_for(MAIN_WINDOW)
                .titled("Discard draft?")
                .sized(420, 220),
            move |window| view! { <CloseConfirmation window=window state=state /> },
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
        notice,
        task_error,
        remaining_txt,
        no_tasks,
        no_matches,
        has_error,
        has_notice,
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
        let mut ids: Vec<TodoId> = visible
            .get()
            .ids()
            .iter()
            .filter_map(|row| todos.key_of(*row))
            .collect();
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
    let import_file = move || {
        let options = FileDialogOptions::open_files()
            .titled("Import tasks")
            .filter("Text files", ["txt"]);
        request_dialog(
            window,
            options,
            notice,
            task_error,
            "Import failed",
            move |outcome| begin_import(window, todos, notice, task_error, outcome),
        );
    };
    let save_tasks = move || {
        let options = FileDialogOptions::save_file()
            .titled("Save tasks")
            .filter("Text files", ["txt"])
            .with_suggested_name("tasks.txt")
            .with_default_extension("txt");
        request_dialog(
            window,
            options,
            notice,
            task_error,
            "Save failed",
            move |outcome| begin_save(window, todos, notice, task_error, outcome),
        );
    };
    let choose_folder = move || {
        let options = FileDialogOptions::pick_folder().titled("Choose a folder");
        request_dialog(
            window,
            options,
            notice,
            task_error,
            "Choose folder failed",
            move |outcome| {
                begin_folder(notice, task_error, outcome);
                Ok(())
            },
        );
    };

    view! {
        <view role="main">
            <text class="text-sm dark:text-muted">{remaining_txt}</text>
            <form
                aria:label="Add task"
                on:submit={move |event: &mut SubmitEvent| {
                    event.prevent_default();
                    add();
                }}
            >
                <text-input
                    class="focus-visible:border-accent"
                    value={draft}
                    aria:label="What needs to be done?"
                    on:input={move |event: &mut InputEvent| draft.set(event.value().to_owned())}
                />
                <button
                    class="px-4 py-2 bg-accent text-inverse rounded-md font-semibold"
                    type="submit"
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
            <view role="toolbar" class="gap-3 flex-wrap">
                <button on:click={move || reverse()}>"Reverse"</button>
                <button on:click={move || {
                    let _ = Transaction::new().write(todos.clear_completed()).finish();
                }}>"Clear done"</button>
                <button on:click={move || load_sample()}>"Load sample"</button>
                <button on:click={move || import_file()}>"Import file"</button>
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
            <view role="toolbar" class="gap-3 flex-wrap">
                <button on:click={move || save_tasks()}>"Save tasks"</button>
                <button on:click={move || choose_folder()}>"Choose folder"</button>
            </view>
            <Show when={has_error}>
                <text aria:label="Error">{task_error}</text>
            </Show>
            <Show when={has_notice}>
                <text aria:label="Notice">{notice}</text>
            </Show>
            <Show when={no_tasks}>
                <text>"Nothing here yet"</text>
            </Show>
            <Show when={no_matches}>
                <text>"Nothing matches"</text>
            </Show>
            <view class="gap-0">
                <For each={visible} store={todos} key={|row: &TodoRow| row.id} row-height={em(2.5)}>
                    {move |row: RowView<TodoRow>| view! {
                        <view style:height={once(em(2.5))}>
                            <TodoItem todo={row.key()} todos={todos} />
                        </view>
                    }}
                </For>
            </view>
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
        <view role="listitem" class="flex-row items-center gap-3" style:height={once(em(2.5))}>
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
        notice,
        task_error,
        has_error,
        has_notice,
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
                on:input={move |event: &mut InputEvent| note.set(event.value().to_owned())}
            />
            <view role="list" aria:label="Details rows" class="gap-0">
                <For each={visible} store={todos} key={|row: &TodoRow| row.id} row-height={em(2.5)}>
                    {move |row: RowView<TodoRow>| view! {
                        <TodoDetailRow todo={row.key()} todos={todos} />
                    }}
                </For>
            </view>
            <Show when={no_tasks}>
                <text>"Nothing here yet"</text>
            </Show>
            <Show when={no_matches}>
                <text>"Nothing matches"</text>
            </Show>
            <Show when={has_error}>
                <text aria:label="Error">{task_error}</text>
            </Show>
            <Show when={has_notice}>
                <text aria:label="Notice">{notice}</text>
            </Show>
            <view role="toolbar" class="gap-3 flex-wrap">
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

/// 主窗口的关闭确认提示。
///
/// 窗口只在父窗口有 pending close epoch 时由关闭处理器显示；factory 读不到
/// 待决请求时拒绝挂载，而不是渲染一对失效的按钮。Continue editing 使用
/// Deny，保留父窗口的草稿、焦点与 caret；Discard draft 清空应用级草稿，
/// 并使用 Allow 把父窗口与提示窗作为一组释放。
#[component]
fn CloseConfirmation(window: WindowCtx, state: TodoState) -> MountResult<MountPlan> {
    let request = window
        .close_request()
        .map_err(|_| MountError::Component("read the todo close request"))?
        .ok_or(MountError::Component(
            "the todo close confirmation has no pending request",
        ))?;
    let TodoState {
        draft, task_error, ..
    } = state;

    view! {
        <view role="main" class="p-4 gap-3">
            <text class="text-lg font-semibold">"Discard the draft?"</text>
            <text class="text-sm dark:text-muted">"The unsaved task draft will be lost."</text>
            <view role="toolbar" class="gap-3 flex-wrap">
                <button
                    class="px-4 py-2 bg-accent text-inverse rounded-md font-semibold"
                    aria:label="Continue editing"
                    on:click={move || report(request.deny(), task_error)}
                >"Continue editing"</button>
                <button
                    class="px-4 py-2 bg-danger text-inverse rounded-md"
                    aria:label="Discard draft"
                    on:click={move || {
                        let result = request.allow();
                        if result.is_ok() {
                            draft.set(String::new());
                        }
                        report(result, task_error);
                    }}
                >"Discard draft"</button>
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::Path;
    use std::rc::Rc;

    use super::{
        DETAILS_WINDOW, TodoState, import_titles, open_todo_window, read_titles, save_titles,
    };
    use zenui::prelude::*;

    type StaleRead = Rc<RefCell<Option<Result<Vec<String>, String>>>>;

    #[component]
    fn StaleProbe(state: TodoState, result: StaleRead) -> MountResult<MountPlan> {
        let make_stale = move || {
            let _ = Transaction::new()
                .write(state.todos.insert("gone"))
                .finish();
            let Some(id) = state.todos.keys(Filter::All).into_iter().next() else {
                return;
            };
            let _ = Transaction::new().write(state.todos.remove(id)).finish();
            *result.borrow_mut() = Some(read_titles([state.todos.title(id)]));
        };

        view! { <button on:click={move || make_stale()}>"Make stale"</button> }
    }

    #[test]
    fn import_titles_skip_blank_lines_and_trim_whitespace() {
        let text = "\r\n  Write tests \r\n\r\nRead docs\n   \n";
        assert_eq!(
            import_titles(text).collect::<Vec<_>>(),
            ["Write tests", "Read docs"]
        );
    }

    #[test]
    fn import_titles_keep_inner_spacing_and_unicode() {
        assert_eq!(
            import_titles("  Buy 牛奶 \n\n 复习 IME  ").collect::<Vec<_>>(),
            ["Buy 牛奶", "复习 IME"]
        );
    }

    #[test]
    fn import_titles_of_an_empty_file_are_empty() {
        assert_eq!(import_titles("\n \r\n").count(), 0);
    }

    #[test]
    fn save_titles_rejects_both_line_endings_before_writing() {
        for separator in [10u8, 13u8] {
            let title = format!("bad{}line", char::from(separator));
            let error =
                save_titles(Path::new("unused"), &[title]).expect_err("a line break is rejected");
            assert!(error.contains("line break"), "unexpected error: {error}");
        }
    }

    #[test]
    fn the_details_window_mounts_only_a_bounded_row_window() {
        let mut app = Runtime::new();
        open_todo_window(&mut app).expect("open the todo window");
        for index in 0..1_000 {
            app.input("What needs to be done?", &format!("Task {index}"))
                .expect("fill the draft");
            app.submit("Add task").expect("add a row");
        }
        let details = app.show(DETAILS_WINDOW).expect("show the details window");
        let snapshot = app.presentation_of(details).expect("details snapshot");
        let bands = snapshot.row_bands();
        assert_eq!(bands.len(), 1, "the details list is the only windowed list");
        assert_eq!(
            bands[0].total_rows(),
            1_000,
            "every shared row is declared to the list"
        );
        assert_eq!(bands[0].first_row(), 0);
        let mounted = snapshot.elements().count();
        assert!(
            mounted < 250,
            "the details window mounts only its visible rows, got {mounted} elements"
        );
    }

    #[test]
    fn a_stale_title_aborts_the_committed_title_read() {
        let mut app = Runtime::new();
        let state = app.setup(TodoState::new).expect("application setup");
        let result: StaleRead = Rc::new(RefCell::new(None));
        let slot = result.clone();
        app.open_window("main", move |_| {
            let slot = slot.clone();
            view! { <StaleProbe state={state} result={slot} /> }
        })
        .expect("open the main window");
        app.click_text("Make stale").expect("make the row stale");

        let outcome = result
            .borrow_mut()
            .take()
            .expect("the probe records one read");
        let error = outcome.expect_err("the stale row cannot be read");
        assert!(
            error.contains("no longer live"),
            "unexpected error: {error}"
        );
    }
}
