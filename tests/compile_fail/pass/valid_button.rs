use zenui::prelude::*;

fn main() {
    let _: MountResult<MountPlan> = view! { <button on:click=move || {}>"Add"</button> };
}
