use zenui::prelude::*;

fn with_memo(store: Store, rows: Memo<RowSet>) -> MountResult<MountPlan> {
    view! {
        <For each={rows} store={store} key={|row: &TodoRow| row.id}>
            {|_row: RowView<TodoRow>| view! { <text>"row"</text> }}
        </For>
    }
}

fn with_rowset(store: Store, set: RowSet) -> MountResult<MountPlan> {
    view! {
        <For each={set} store={store} key={|row: &TodoRow| row.id}>
            {|_row: RowView<TodoRow>| view! { <text>"row"</text> }}
        </For>
    }
}

fn with_once(store: Store, set: Once<RowSet>) -> MountResult<MountPlan> {
    view! {
        <For each={set} store={store} key={|row: &TodoRow| row.id}>
            {|_row: RowView<TodoRow>| view! { <text>"row"</text> }}
        </For>
    }
}

fn main() {}
