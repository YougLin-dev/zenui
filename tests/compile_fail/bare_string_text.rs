use zenui::prelude::*;

fn main() {
    let title = String::from("bare");
    let _: MountResult<MountPlan> = view! { <text>{title}</text> };
}
