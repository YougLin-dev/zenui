use zenui::prelude::*;

fn main() {
    let color = Color::Role(ColorRole::Accent);
    let _: MountResult<MountPlan> = view! { <view style:width={color}>"Add"</view> };
}
