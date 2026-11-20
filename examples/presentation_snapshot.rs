//! 打印 mounted headless Todo 的 presentation snapshot。
//!
//! 只检查已提交界面的身份、属性、文本与本地编辑状态；不声明原生窗口或像素呈现。
#![allow(
    unused_braces,
    reason = "{} is the intentionally supported event-expression syntax"
)]

use zenui::prelude::*;

#[component]
fn Demo() -> MountResult<MountPlan> {
    let todos = Store::new();
    let draft = signal(String::new());
    let rows = memo(move || todos.rows(Filter::All));
    let empty = memo(move || todos.is_empty(Filter::All));

    let add = move || {
        let title = draft.with(|text| text.trim().to_owned());
        if title.is_empty() {
            return;
        }
        let _ = Transaction::new().write(todos.insert(title)).finish();
        draft.set(String::new());
    };

    view! {
        <view role="main">
            <form aria:label="Add task" on:submit={move |_event: SubmitEvent| add()}>
                <text-input
                    value={draft}
                    aria:label="Draft"
                    on:input={move |value: String| draft.set(value)}
                />
                <button on:click={move || add()}>"Add"</button>
            </form>
            <Show when={empty}>
                <text>"Nothing here yet"</text>
            </Show>
            <For each={rows} key={|todo: TodoId| todo}>
                {move |todo: TodoId| view! {
                    <text aria:label={once(format!("row {}", todo.raw()))}>
                        {todos.title(todo).read_once()?}
                    </text>
                }}
            </For>
        </view>
    }
}

fn main() {
    let mut app = Runtime::new();
    app.mount(|| view! { <Demo /> }).expect("mount");

    app.input("Draft", "Write tests").expect("input");
    app.submit("Add task").expect("submit");
    app.input("Draft", "Read docs").expect("input");
    app.submit("Add task").expect("submit");

    app.focus("Draft").expect("focus the draft");
    app.input("Draft", "Third").expect("input");
    app.set_caret("Draft", 2);
    app.compose_start("Draft").expect("composition start");
    app.compose_update("Draft", "ni")
        .expect("composition update");

    println!("{}", app.presentation().expect("presentation snapshot"));
    println!("preedit via helper: {:?}", app.preedit("Draft"));
}
