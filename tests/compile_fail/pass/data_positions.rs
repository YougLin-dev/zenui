#![allow(unused_braces)]

use zenui::prelude::*;

#[component]
fn ReadText(value: Read<String>) -> MountResult<MountPlan> {
    let value = value.read_once()?;
    view! { <text>{once(value)}</text> }
}

fn build(text: Signal<String>, shown: Signal<bool>) -> MountResult<MountPlan> {
    view! {
        <view>
            <ReadText value={text} />
            <text style:width={once(px(10.0))}>{once(String::from("snapshot"))}</text>
            <button disabled={true}>"Literal bool"</button>
            <Show when={true}><text>"Literal branch"</text></Show>
            <Show when={shown}><text>"Shown"</text></Show>
        </view>
    }
}

fn main() {
    let _ = build;
}
