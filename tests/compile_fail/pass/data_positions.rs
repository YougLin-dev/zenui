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

fn derived(
    count: Signal<u32>,
    draft: Signal<String>,
    tint: Signal<Color>,
    shown: Signal<bool>,
    ready: Signal<bool>,
) -> MountResult<MountPlan> {
    view! {
        <view>
            <text aria:label={|| format!("count-{}", count.get())}>
                {|| format!("{} left", count.get())}
            </text>
            <text>{move || format!("{} left", count.get())}</text>
            <button disabled={|| count.get() == 0}>"Clear"</button>
            <checkbox checked={|| ready.get()} on:change={move |_event: &mut ChangeEvent| {}} />
            <view style:color={|| tint.get()} />
            <Show when={|| shown.get()}><text>"Shown"</text></Show>
            <text-input
                value={draft}
                on:input={move |event: &mut InputEvent| draft.set(event.value().to_owned())}
            />
            <text-input
                value={draft}
                on:input={move |event| draft.set(event.value().to_owned())}
            />
        </view>
    }
}

fn main() {
    let _ = (build, derived);
}
