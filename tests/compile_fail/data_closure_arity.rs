use zenui::prelude::*;

fn main() {
    let _: MountResult<MountPlan> =
        view! { <button disabled={|value: bool| value}>"Blocked"</button> };
}
