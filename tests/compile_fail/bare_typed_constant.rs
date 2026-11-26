use zenui::prelude::*;

const WIDTH: Length = Length::from_fixed(1280, LengthUnit::Points);

fn main() {
    let _: MountResult<MountPlan> = view! { <view style:width={WIDTH}>"Width"</view> };
}
