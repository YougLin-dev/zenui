use zenui::prelude::*;

fn bad(rows: Memo<RowSet>) -> MountResult<MountPlan> {
    view! {
        <For each={rows} key={|row: &TodoRow| row.id}>
            {|_row: RowView<TodoRow>| view! { <text>"row"</text> }}
        </For>
    }
}

fn main() {}
