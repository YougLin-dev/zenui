use zenui::prelude::*;

fn main() {
    let blocked = true;
    let _: MountResult<MountPlan> = view! { <button disabled={blocked}>"Blocked"</button> };
}
