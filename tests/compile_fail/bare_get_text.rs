use zenui::prelude::*;

fn main() {
    let title = signal(String::from("bare"));
    let _: MountResult<MountPlan> = view! { <text>{title.get()}</text> };
}
