// 目标界面语法预览，不参与默认 workspace 构建。
// `TaskScope` 在应用启动和窗口挂载时分别取得；一个 app job 返回 prepared
// 数据，在仍然有效的 application continuation 中提交，不经 worker 修改 UI Store。
use zenui::platform::{
    App, AppCx, CloseReply, Dialog, DialogOutcome, Menu, Shortcut, ShutdownCx, Tray, WindowClose,
    WindowCtx, WindowSpec,
};
use zenui::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Active,
    Done,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Win {
    Main,
    Preferences,
    ConfirmDiscard,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct TodoId(u64);

#[derive(Clone, Copy)]
struct TodoRow {
    id: TodoId,
    completed: Field<bool>,
}

#[derive(Clone, Copy)]
struct TodoStore(StoreId);

#[derive(Clone, Copy)]
struct ActivityRow {
    id: u64,
}

impl ActivityRow {
    fn text(self) -> String {
        format!("event {}", self.id)
    }
}

impl TodoStore {
    fn new(cx: &mut AppCx) -> MountResult<Self> {
        todo!()
    }

    fn rows(self, filter: Filter) -> RowSet {
        todo!()
    }

    fn is_empty(self, filter: Filter) -> bool {
        todo!()
    }

    fn remaining(self) -> Memo<u32> {
        todo!()
    }

    fn has_unsaved(self) -> bool {
        todo!()
    }

    fn insert(self, title: String) -> WriteOp<TodoId> {
        todo!()
    }

    fn title(self, id: TodoId) -> FieldQuery<String> {
        todo!()
    }

    fn completed(self, id: TodoId) -> Field<bool> {
        todo!()
    }

    fn set_completed(self, id: TodoId, done: bool) -> WriteOp<()> {
        todo!()
    }

    fn remove(self, id: TodoId) -> WriteOp<()> {
        todo!()
    }

    fn clear_completed(self) -> WriteOp<()> {
        todo!()
    }

    fn activity(self) -> RowSet {
        todo!()
    }

    fn clear_activity(self) -> WriteOp<()> {
        todo!()
    }

    fn per_day(self) -> Series {
        todo!()
    }

    fn showing_completed(self) -> Memo<bool> {
        todo!()
    }

    fn set_show_completed(self, value: bool) -> WriteOp<()> {
        todo!()
    }

    fn toggle_completed(self) -> WriteOp<()> {
        todo!()
    }

    fn progress(self) -> Memo<Progress> {
        todo!()
    }

    fn is_importing(self) -> Memo<bool> {
        todo!()
    }

    fn commit_import(self, prepared: PreparedImport) -> WriteOp<()> {
        todo!()
    }

    fn discard_revision(self, revision: Revision) -> WriteOp<()> {
        todo!()
    }

    fn flush_bounded(self, deadline: Deadline) -> FlushOutcome {
        todo!()
    }
}

#[derive(Clone, Copy)]
struct AppState {
    todos: TodoStore,
    tasks: TaskScope,
    draft: Signal<String>,
    filter: Signal<Filter>,
    errors: ErrorSignal,
    remaining_txt: Memo<String>,
}

#[derive(Clone, Copy)]
struct WindowScope {
    window: WindowCtx,
    env: WindowEnv,
}

fn main() -> zenui::Result<()> {
    App::new("com.example.zenui.tour")
        .activation_policy(ActivationPolicy::Regular)
        .tabbing(Tabbing::Disallowed)
        .quit_on_last_window_closed(false)
        .on_reopen(|cx: &mut AppCx| cx.host().window(Win::Main).show())
        .run(move |cx: &mut AppCx| -> MountResult<()> {
            let todos = TodoStore::new(cx)?;
            let remaining_txt = memo(move || format!("{} remaining", todos.remaining().get()));
            let tasks = task_scope();

            let state = AppState {
                todos,
                tasks,
                draft: signal(String::new()),
                filter: signal(Filter::All),
                errors: cx.errors(),
                remaining_txt,
            };

            let progress = todos.progress();

            cx.menu(app_menu(state));
            cx.tray(
                Tray::new(assets::TRAY_ICON)
                    .tooltip(remaining_txt)
                    .on_click(|cx: &mut AppCx| cx.host().window(Win::Main).show()),
            );
            cx.on_session_end(move |cx: ShutdownCx| {
                let outcome = state.todos.flush_bounded(cx.deadline());
                cx.report_flush(outcome)
            });

            cx.window(
                WindowSpec::new("Todo")
                    .primary()
                    .content_size(LogicalSize::new(880.0, 640.0))
                    .min_size(LogicalSize::new(480.0, 360.0))
                    .title(remaining_txt)
                    .taskbar(progress)
                    .initial_focus(FocusId::Draft)
                    .on_close_request(move || {
                        if state.todos.has_unsaved() {
                            WindowClose::Ask
                        } else {
                            WindowClose::Allow
                        }
                    }),
                Win::Main,
                move |cx| {
                    let scope = WindowScope {
                        window: cx.context(),
                        env: cx.env(),
                    };
                    Ok(view! {
                        <TodoApp
                            state={state}
                            scope={scope}
                            caps={cx.capabilities()}
                        />
                    })
                },
            );

            cx.window(
                WindowSpec::new("Preferences")
                    .content_size(LogicalSize::new(560.0, 420.0))
                    .resizable(false),
                Win::Preferences,
                move |_cx| Ok(view! { <Preferences state={state} /> }),
            );

            cx.window(
                WindowSpec::new("Unsaved changes")
                    .content_size(LogicalSize::new(420.0, 200.0))
                    .owner(Win::Main)
                    .modal(),
                Win::ConfirmDiscard,
                move |cx| {
                    let scope = WindowScope {
                        window: cx.context(),
                        env: cx.env(),
                    };
                    Ok(view! { <ConfirmDiscard state={state} scope={scope} /> })
                },
            );

            Ok(())
        })
}

fn app_menu(state: AppState) -> Menu {
    Menu::new()
        .submenu("File", |menu| {
            menu.item(
                "Clear completed",
                Shortcut::primary(Key::K),
                move |_cx: &mut AppCx| state.todos.clear_completed(),
            )
            .separator()
            .item("Close window", Shortcut::primary(Key::W), |cx: &mut AppCx| {
                cx.host()
                    .focused()
                    .map(WindowCtx::close)
                    .unwrap_or(Ok(()))
            })
        })
        .submenu("ZenUI", |menu| {
            menu.item(
                "Preferences…",
                Shortcut::primary(Key::Comma),
                |cx: &mut AppCx| cx.host().window(Win::Preferences).show(),
            )
            .check_item(
                "Show completed",
                state.todos.showing_completed(),
                move |_cx: &mut AppCx| state.todos.toggle_completed(),
            )
        })
}

#[component]
fn TodoApp(
    state: AppState,
    scope: WindowScope,
    caps: PlatformCapabilities,
) -> MountResult<MountPlan> {
    let AppState {
        todos,
        tasks: _tasks,
        draft,
        filter,
        errors,
        remaining_txt,
    } = state;
    let focused = signal(false);

    let visible = memo(move || todos.rows(filter.get()));
    let no_tasks = memo(move || todos.is_empty(Filter::All));
    let no_matches = memo(move || !todos.is_empty(Filter::All) && todos.is_empty(filter.get()));
    let cannot_add = memo(move || draft.with(|text| text.trim().is_empty()));
    let has_error = memo(move || errors.with(|error| error.is_some()));
    let error_text = memo(move || {
        errors.with(|error| {
            error
                .as_ref()
                .map(AppError::user_message)
                .unwrap_or_default()
        })
    });
    let activity = memo(move || todos.activity());
    let streaming = memo(move || todos.is_importing().get());

    let panel_tint = memo(move || match scope.env.appearance().get() {
        Appearance::Dark => rgb(0x18181b),
        Appearance::Light => rgb(0xffffff),
    });
    let panel_opacity = memo(move || if has_error.get() { 0.45 } else { 1.0 });

    let scroller = scroll(Axis::Y);
    let log_scroller = scroll(Axis::Y);
    let fade = Motion::ease(ms(150)).opacity();
    let tick = ticker(Hertz::new(60)?, TickPurpose::Decorative);
    let chart = canvas_content(
        "Completion by day",
        memo(move || Chart::new(todos.per_day())),
    );
    let video = texture_source(TextureSpec::rgba8_srgb_premultiplied());
    let poster = texture_source(TextureSpec::rgba8_srgb_premultiplied());
    let scene = scene3d(SceneOptions::default());
    let can_export = once(caps.file_dialogs.is_supported());
    let import = Dialog::open_files()
        .title("Import tasks")
        .filter("Task export", &["json"]);

    let add = move || -> Result<(), AppError> {
        let title = draft.with(|text| text.trim().to_owned());
        if title.is_empty() {
            return Ok(());
        }

        Transaction::new()
            .write(todos.insert(title))
            .finish()?;

        // 模型提交与草稿清理在同一个 command turn 中结束，随后再统一刷新。
        // 这里不把 UI 值伪装成可跨域回滚的模型写。
        draft.set(String::new());
        Ok(())
    };

    view! {
        <view role="main" class="flex h-full flex-col gap-4 bg-zinc-50 p-6 text-zinc-950">
            <view class="flex items-baseline justify-between">
                <text class="text-2xl font-bold">"Todo"</text>
                <text class="text-sm text-zinc-500">{remaining_txt}</text>
            </view>

            <Show when={has_error}>
                <text role="alert" class="rounded-md bg-rose-50 p-3 text-rose-900">
                    {error_text}
                </text>
            </Show>

            <form class="flex gap-2" on:submit={move |event: SubmitEvent| {
                event.prevent_default();
                add()
            }}>
                <text-input
                    id={FocusId::Draft}
                    class="flex-1 rounded-md border border-zinc-300 bg-white px-3 py-2"
                    aria:label="What needs to be done?"
                    placeholder="Add a task"
                    value={draft}
                    on:input={move |value: String| draft.set(value)}
                    on:keydown={move |event: KeyEvent| {
                        if event.key() == Key::Escape {
                            draft.set(String::new());
                        }
                    }}
                    on:focus={move || focused.set(true)}
                    on:blur={move || focused.set(false)}
                />
                <button
                    type="submit"
                    class="rounded-md bg-blue-600 px-4 py-2 text-white disabled:opacity-50"
                    disabled={cannot_add}
                >"Add"</button>
            </form>

            <Show when={focused}>
                <text class="text-xs text-zinc-400">"Enter 保存，Esc 清空"</text>
            </Show>

            <view role="group" aria:label="Filter" class="flex gap-1">
                <FilterButton current={filter} value={Filter::All} label="All" />
                <FilterButton current={filter} value={Filter::Active} label="Active" />
                <FilterButton current={filter} value={Filter::Done} label="Done" />
            </view>

            <view class="flex min-h-0 flex-1 gap-4">
                <Show when={no_tasks}>
                    <text class="flex-1 rounded-lg bg-white p-6 text-center text-zinc-500">
                        "Nothing here yet. Add your first task."
                    </text>
                </Show>

                <Show when={no_matches}>
                    <text class="flex-1 rounded-lg bg-white p-6 text-center text-zinc-500">
                        "No tasks match this filter."
                    </text>
                </Show>

                <view
                    role="list"
                    aria:label="Tasks"
                    scroll={scroller}
                    class="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto"
                >
                    <For each={visible} key={|row: &TodoRow| row.id}>
                        {move |row: TodoRow| view! {
                            <TodoItem todo={row} store={todos} />
                        }}
                    </For>
                </view>

                <view class="flex w-80 flex-col gap-4">
                    <Panel
                        title="Activity"
                        actions={move || view! {
                            <button
                                class="text-xs text-blue-600"
                                on:click={move || todos.clear_activity()}
                            >"Clear"</button>
                        }}
                    >
                        <VirtualList
                            rows={activity}
                            key={|event: &ActivityRow| event.id}
                            item={|event: ActivityRow| view! {
                                <text class="text-xs text-zinc-500">{once(event.text())}</text>
                            }}
                            item_extent={em(1.75)}
                            overscan={em(4.5)}
                            scroll={log_scroller}
                            a11y_item={|event: &ActivityRow| {
                                A11yItem::listitem(event.text())
                            }}
                        />
                    </Panel>

                    <view
                        motion={fade}
                        style:background={panel_tint}
                        style:opacity={panel_opacity}
                        class="grid gap-4 rounded-lg p-2"
                    >
                        <Show
                            when={streaming}
                            fallback={move || view! {
                                <canvas content={chart} class="h-40 rounded-lg bg-white" />
                            }}
                        >
                            <canvas
                                content={chart}
                                ticker={tick}
                                class="h-40 rounded-lg bg-white"
                            />
                        </Show>

                        <image source={video} placeholder={poster} class="h-40 rounded-lg" />

                        <Scene3d
                            view={scene}
                            interaction={SceneInteraction::PresentationOnly}
                            animation={SceneAnimation::Static}
                            fallback={move || view! {
                                <text class="p-4 text-sm text-zinc-500">"3D unavailable"</text>
                            }}
                            class="h-40 rounded-lg bg-zinc-100"
                        />
                    </view>

                    <Show when={can_export}>
                        <button
                            class="rounded-md bg-white px-3 py-2 text-sm shadow-sm"
                            on:click={move || -> zenui::Result<()> {
                                let ticket = import.open(&scope.window)?;
                                let app_tasks = state.tasks;
                                ticket.on_done(move |outcome: DialogOutcome| {
                                    if let DialogOutcome::Picked(files) = outcome {
                                        let job = ImportJob::new(todos, files);
                                        app_tasks.spawn(move |token| {
                                            let prepared = job.prepare(token)?;
                                            // Prepared result 只包含 Send 数据；UI continuation
                                            // 校验任务归属后才提交模型事务。
                                            prepared
                                        });
                                    }
                                    Ok(())
                                });
                                Ok(())
                            }}
                        >"Import…"</button>
                    </Show>
                </view>
            </view>
        </view>
    }
}

#[component]
fn FilterButton(
    current: Signal<Filter>,
    value: Filter,
    label: &'static str,
) -> MountResult<MountPlan> {
    let selected = memo(move || current.get() == value);

    view! {
        <button
            class="rounded-md bg-zinc-100 px-3 py-1 text-sm aria-pressed:bg-blue-600 aria-pressed:text-white"
            aria:pressed={selected}
            on:click={move || current.set(value)}
        >{label}</button>
    }
}

#[component]
fn TodoItem(todo: TodoRow, store: TodoStore) -> MountResult<MountPlan> {
    let id = todo.id;
    let title = store.title(id).read_once()?;
    let done_label = format!("Complete {title}");
    let delete_label = format!("Delete {title}");
    let completed = memo(move || todo.completed.get());
    let opacity = memo(move || completed.get().map(|done| if done { 0.55 } else { 1.0 }));
    let strike = memo(move || {
        completed.get().map(|done| {
            if done {
                TextDecoration::LineThrough
            } else {
                TextDecoration::None
            }
        })
    });

    view! {
        <view role="listitem" class="flex items-center gap-3 rounded-lg bg-white p-3">
            <checkbox
                checked={completed}
                aria:label={once(done_label)}
                on:change={move |checked: bool| store.set_completed(id, checked)}
            />
            <text style:opacity={opacity} style:text-decoration-line={strike}>
                {once(title)}
            </text>
            <button
                class="ms-auto rounded-md p-2 text-rose-600"
                aria:label={once(delete_label)}
                on:click={move || store.remove(id)}
            >"Delete"</button>
        </view>
    }
}

#[component]
fn Panel(
    title: &'static str,
    actions: impl Into<ChildrenFactory>,
    children: impl Into<Children>,
) -> MountResult<MountPlan> {
    let actions = actions.into();
    let children = children.into();

    view! {
        <view class="flex min-h-0 flex-col rounded-lg bg-white p-3 shadow-sm">
            <view class="mb-2 flex items-center justify-between">
                <text class="text-sm font-semibold">{title}</text>
                <view class="flex gap-1">{actions}</view>
            </view>
            <view class="min-h-0 flex-1">{children}</view>
        </view>
    }
}

#[component]
fn Preferences(state: AppState) -> MountResult<MountPlan> {
    let show_completed = state.todos.showing_completed();

    view! {
        <view class="flex flex-col gap-4 p-6">
            <text class="text-lg font-semibold">"Preferences"</text>
            <checkbox
                checked={show_completed}
                aria:label="Show completed tasks"
                on:change={move |value: bool| state.todos.set_show_completed(value)}
            />
            <view class="flex gap-1">
                <FilterButton current={state.filter} value={Filter::All} label="All" />
                <FilterButton current={state.filter} value={Filter::Active} label="Active" />
            </view>
        </view>
    }
}

#[component]
fn ConfirmDiscard(state: AppState, scope: WindowScope) -> MountResult<MountPlan> {
    view! {
        <view class="flex flex-col gap-4 p-6">
            <text role="alert">"You have unsaved changes."</text>
            <view class="flex justify-end gap-2">
                <button
                    class="rounded-md px-3 py-2"
                    on:click={move || scope.window.reply(CloseReply::Deny)}
                >"Keep editing"</button>
                <button
                    class="rounded-md bg-rose-600 px-3 py-2 text-white"
                    on:click={move || {
                        let revision = scope.window.close_revision()?;
                        Transaction::new()
                            .write(state.todos.discard_revision(revision))
                            .finish()?;
                        state.draft.set(String::new());
                        scope.window.reply(CloseReply::Allow)
                    }}
                >"Discard"</button>
            </view>
        </view>
    }
}

mod assets {
    use zenui::prelude::*;

    pub const TRAY_ICON: Icon = Icon::embedded(include_bytes!("../assets/tray.png"));
}
