use zenui::prelude::*;

fn bad(rowset: RowSet) -> MountResult<MountPlan> {
    view! {
        <For each={rowset} key={|row: &TodoRow| row.id}>
            {|_row: RowView<TodoRow>| view! { <text>"row"</text> }}
        </For>
    }
}

fn main() {}
