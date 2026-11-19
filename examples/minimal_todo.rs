//! 阶段一目标 API 的最小 headless Todo；不声明原生窗口、IME 或 Windows 完成。
#![allow(
    unused_braces,
    reason = "{} is the intentionally supported event-expression syntax"
)]

use zenui::prelude::*;

#[component]
pub fn FilterButton(
    current: Signal<Filter>,
    value: Filter,
    label: &'static str,
) -> MountResult<MountPlan> {
    let selected = memo(move || current.get() == value);

    view! {
        <button aria:pressed={selected} on:click={move || current.set(value)}>{label}</button>
    }
}

#[component]
pub fn TodoItem(todo: TodoId, todos: Store) -> MountResult<MountPlan> {
    let title = todos.title(todo).read_once()?;
    let completed = memo(move || todos.completed(todo).get());
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
            <text>{once(title.clone())}</text>
            <text-input
                value={editing}
                aria:label={once(edit_label)}
                on:input={move |value: String| {
                    editing.set(value.clone());
                    let _ = Transaction::new().write(todos.set_title(todo, value)).finish();
                }}
            />
            <button
                aria:label={once(delete_label)}
                on:click={move || {
                    let _ = Transaction::new().write(todos.remove(todo)).finish();
                }}
            >"Delete"</button>
        </view>
    }
}

#[component]
pub fn TodoApp() -> MountResult<MountPlan> {
    let todos = Store::new();
    let draft = signal(String::new());
    let filter = signal(Filter::All);
    let visible = memo(move || todos.rows(filter.get()));
    let remaining = memo(move || todos.rows(Filter::Active).len());
    let remaining_txt = memo(move || format!("{} remaining", remaining.get()));
    let no_tasks = memo(move || todos.is_empty(Filter::All));
    let no_matches = memo(move || !todos.is_empty(Filter::All) && todos.is_empty(filter.get()));

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

    view! {
        <view role="main">
            <text>{remaining_txt}</text>
            <form
                aria:label="Add task"
                on:submit={move |event: SubmitEvent| {
                    let mut event = event;
                    event.prevent_default();
                    add();
                }}
            >
                <text-input
                    value={draft}
                    aria:label="What needs to be done?"
                    on:input={move |value: String| draft.set(value)}
                />
                <button on:click={move || add()}>"Add"</button>
            </form>
            <view role="group" aria:label="Filter">
                <FilterButton current={filter} value={Filter::All} label="All" />
                <FilterButton current={filter} value={Filter::Active} label="Active" />
                <FilterButton current={filter} value={Filter::Completed} label="Done" />
            </view>
            <view role="toolbar">
                <button on:click={move || reverse()}>"Reverse"</button>
                <button on:click={move || {
                    let _ = Transaction::new().write(todos.clear_completed()).finish();
                }}>"Clear done"</button>
            </view>
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

fn main() {
    let mut app = Runtime::new();
    app.mount(|| view! { <TodoApp /> }).expect("mount");

    app.input("What needs to be done?", "Write tests")
        .expect("input");
    app.submit("Add task").expect("add");
    app.input("What needs to be done?", "Read docs")
        .expect("input");
    app.submit("Add task").expect("add");
    app.change("Complete Write tests", true).expect("complete");
    app.click_text("Active").expect("filter");
    println!("active: {}", app.texts().join(" | "));

    app.click_text("All").expect("filter");
    app.click_text("Reverse").expect("reorder");
    app.click_label("Delete Read docs").expect("delete");
    println!("all: {}", app.texts().join(" | "));
}
