use zenui::prelude::*;

fn main() {
    let class = "p-2";
    let _: MountResult<MountPlan> = view! { <button class={class}>"Add"</button> };
}
