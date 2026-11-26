use zenui::prelude::*;

fn main() {
    let _ = view! { <view on:drop={move |_event: DropEvent| {}}>"drop"</view> };
}
