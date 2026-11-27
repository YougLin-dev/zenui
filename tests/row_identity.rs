#![allow(unused_braces)]

//! #339 验收：行身份在重排、删除重建与过滤迁移中的稳定语义。
//!
//! 全部通过公开 UI 观察：NodeId 是否存活、焦点与本地编辑状态、以及旧
//! `Field` 句柄在行删除后的 `Missing` 读取。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use zenui::prelude::*;

fn mount(root: impl FnOnce() -> MountResult<MountPlan> + 'static) -> Runtime {
    let mut app = Runtime::new();
    app.mount(root).expect("mount");
    app
}

fn editor(app: &Runtime, label: &str) -> NodeId {
    app.presentation()
        .expect("snapshot")
        .find_label(label)
        .expect("the row editor")
        .id()
}

fn row_label(todo: TodoId) -> String {
    format!("row {}", todo.raw())
}

fn todo_rows(app: &Runtime) -> Vec<(RowId, TodoId)> {
    app.for_items()
        .iter()
        .map(|item| (item.row, item.todo.expect("todo row")))
        .collect()
}

fn presentation_order(app: &Runtime, first: &str, second: &str) -> Vec<String> {
    app.presentation()
        .expect("snapshot")
        .elements()
        .filter_map(SnapshotElement::label)
        .filter(|label| *label == first || *label == second)
        .map(str::to_owned)
        .collect()
}

#[component]
fn ReorderHost() -> MountResult<MountPlan> {
    let store = Store::new();
    let rows = memo(move || store.rows(Filter::All));
    view! {
        <view>
            <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                Transaction::new()
                    .write(store.insert("alpha"))
                    .write(store.insert("bravo"))
                    .finish()
            }}>"Seed"</button>
            <button aria:label="reverse" on:click={move || -> Result<(), WriteError> {
                let reversed: Vec<TodoId> = store.keys(Filter::All).into_iter().rev().collect();
                Transaction::new().write(store.reorder(&reversed)).finish()
            }}>"Reverse"</button>
            <For each={rows} store={store} key={|row: &TodoRow| row.id}>
                {move |row: RowView<TodoRow>| {
                    let id = row.key();
                    view! {
                        <text-input
                            aria:label={once(row_label(id))}
                            value={store.title(id)}
                            on:input={move |_text: String| {}}
                        />
                    }
                }}
            </For>
        </view>
    }
}

#[test]
fn reorder_keeps_the_focused_rows_caret_and_selection() {
    let mut app = mount(|| view! { <ReorderHost /> });
    app.click_label("seed").expect("seed two rows");
    let rows = todo_rows(&app);
    assert_eq!(rows.len(), 2);
    let first_label = row_label(rows[0].1);
    let first = editor(&app, &first_label);

    app.focus(&first_label).expect("focus the first editor");
    app.set_caret_node(first, 1).expect("place the caret");
    app.insert_text_node(first, "X")
        .expect("leave a pending local edit");
    app.set_selection_node(first, 1, 4)
        .expect("select an interior range");
    {
        let snapshot = app.presentation().expect("snapshot");
        let element = snapshot.find_label(&first_label).expect("the editor");
        assert_eq!(element.text_value(), Some("aXlpha"));
        let edit = element.edit().expect("edit state");
        assert_eq!(edit.caret(), 4);
        assert_eq!(edit.selection(), Some((1, 4)));
    }

    app.click_label("reverse").expect("reorder the rows");
    assert_eq!(
        todo_rows(&app)
            .iter()
            .map(|(_, key)| *key)
            .collect::<Vec<_>>(),
        vec![rows[1].1, rows[0].1],
        "the model really reordered"
    );
    let second_label = row_label(rows[1].1);
    assert_eq!(
        presentation_order(&app, &first_label, &second_label),
        vec![second_label.clone(), first_label.clone()],
        "the mounted nodes really moved in presentation order"
    );
    let snapshot = app.presentation().expect("snapshot");
    let element = snapshot
        .find_label(&first_label)
        .expect("the editor survives");
    assert_eq!(element.id(), first, "reorder keeps the mounted NodeId");
    assert_eq!(
        snapshot.focus(),
        Some(first),
        "focus stays on the same editor node"
    );
    assert_eq!(
        element.text_value(),
        Some("aXlpha"),
        "the pending local edit survives reorder"
    );
    let edit = element.edit().expect("edit state");
    assert_eq!(edit.caret(), 4, "caret survives reorder");
    assert_eq!(
        edit.selection(),
        Some((1, 4)),
        "an interior selection survives reorder"
    );
}

/// 删除/重建同一业务标题，以及旧 `Field` 句柄的删除后读取。
#[component]
fn ReinsertHost() -> MountResult<MountPlan> {
    let store = Store::new();
    let rows = memo(move || store.rows(Filter::All));
    let a_key = signal(None::<TodoId>);
    let captured = Rc::new(RefCell::new(None::<Field<String>>));
    let seed_captured = Rc::clone(&captured);
    let probe = signal(String::from("idle"));
    let probes = Rc::new(Cell::new(0u32));
    let probe_count = Rc::clone(&probes);
    // 应用侧缓存：memo 依赖 Field，删除行后必须被失效为 Missing。
    let watch_epoch = signal(0u32);
    let watched_captured = Rc::clone(&captured);
    let watched_text = memo(move || {
        let _ = watch_epoch.get();
        match watched_captured.borrow().as_ref() {
            Some(field) => match field.get() {
                Ok(title) => format!("memo {title}"),
                Err(FieldError::Missing(id)) => format!("memo missing {}", id.raw()),
            },
            None => String::from("memo idle"),
        }
    });

    view! {
        <view>
            <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                Transaction::new()
                    .write(store.insert("A"))
                    .write(store.insert("B"))
                    .finish()?;
                let key = store.keys(Filter::All)[0];
                a_key.set(Some(key));
                let field = store.title(key);
                *seed_captured.borrow_mut() = Some(field);
                watch_epoch.set(1);
                Ok(())
            }}>"Seed"</button>
            <button aria:label="delete-a" on:click={move || -> Result<(), WriteError> {
                if let Some(key) = a_key.get() {
                    Transaction::new().write(store.remove(key)).finish()?;
                }
                Ok(())
            }}>"DeleteA"</button>
            <button aria:label="reinsert-a" on:click={move || -> Result<(), WriteError> {
                Transaction::new().write(store.insert("A")).finish()
            }}>"ReinsertA"</button>
            <button aria:label="probe-field" on:click={move || {
                let field = captured.borrow().expect("the captured field");
                let count = probe_count.get() + 1;
                probe_count.set(count);
                let text = match field.read_once() {
                    Ok(title) => format!("title {title} #{count}"),
                    Err(FieldError::Missing(id)) => format!("missing {} #{count}", id.raw()),
                };
                probe.set(text);
            }}>"Probe"</button>
            <text>{probe}</text>
            <text>{watched_text}</text>
            <For each={rows} store={store} key={|row: &TodoRow| row.id}>
                {move |row: RowView<TodoRow>| {
                    let id = row.key();
                    view! {
                        <text-input
                            aria:label={once(row_label(id))}
                            value={store.title(id)}
                            on:input={move |_text: String| {}}
                        />
                    }
                }}
            </For>
        </view>
    }
}

#[test]
fn delete_and_reinsert_with_the_same_title_starts_a_fresh_row() {
    let mut app = mount(|| view! { <ReinsertHost /> });
    app.click_label("seed").expect("seed A and B");
    let before = todo_rows(&app);
    assert_eq!(before.len(), 2);
    let (old_row, old_key) = before[0];
    let old_label = row_label(old_key);
    let old_node = editor(&app, &old_label);
    let other_node = editor(&app, &row_label(before[1].1));
    assert!(
        app.texts().iter().any(|text| text == "memo A"),
        "the memoized Field consumer sees the live row: {:?}",
        app.texts()
    );

    app.focus(&old_label).expect("focus A");
    app.set_caret_node(old_node, 1).expect("place the caret");
    app.insert_text_node(old_node, "X")
        .expect("edit the old row");
    assert_eq!(
        app.presentation()
            .expect("snapshot")
            .find_label(&old_label)
            .expect("the old editor")
            .text_value(),
        Some("AX"),
        "the local edit is visible"
    );

    app.click_label("delete-a").expect("delete A");
    assert_eq!(app.node_status(old_node), NodeStatus::Stale);
    assert_eq!(todo_rows(&app), vec![before[1]]);
    assert_eq!(
        app.presentation().expect("snapshot").focus(),
        Some(other_node),
        "deleting the focused row settles focus on the surviving editor"
    );

    app.click_label("probe-field").expect("probe the old field");
    assert!(
        app.texts()
            .iter()
            .any(|text| text == &format!("missing {} #1", old_key.raw())),
        "the old Field reads Missing after the delete: {:?}",
        app.texts()
    );
    assert!(
        app.texts()
            .iter()
            .any(|text| text == &format!("memo missing {}", old_key.raw())),
        "the row deletion invalidates memoized Field consumers: {:?}",
        app.texts()
    );

    app.click_label("reinsert-a")
        .expect("reinsert a row titled A");
    let after = todo_rows(&app);
    assert_eq!(after.len(), 2);
    let (new_row, new_key) = after[1];
    assert_ne!(
        new_row, old_row,
        "delete and reinsert allocates a new RowId"
    );
    assert_ne!(new_key, old_key, "the business key is new as well");
    let new_label = row_label(new_key);
    let new_node = editor(&app, &new_label);
    assert_ne!(new_node, old_node, "the reinserted row mounts a fresh node");
    assert_eq!(app.node_status(old_node), NodeStatus::Stale);

    let snapshot = app.presentation().expect("snapshot");
    assert_eq!(
        snapshot.focus(),
        Some(other_node),
        "the reinserted row does not inherit focus"
    );
    let element = snapshot.find_label(&new_label).expect("the new editor");
    assert_eq!(
        element.text_value(),
        Some("A"),
        "the old local edit is not inherited"
    );
    let edit = element.edit().expect("edit state");
    assert_eq!(edit.caret(), 0);
    assert_eq!(edit.selection(), None);

    app.insert_text_node(new_node, "Y")
        .expect("the fresh row accepts edits");
    assert_eq!(
        app.insert_text_node(old_node, "Z"),
        Err(DispatchError::StaleNode(old_node)),
        "the old mount rejects delivery"
    );

    app.click_label("probe-field")
        .expect("probe the old field after the reinsert");
    assert!(
        app.texts()
            .iter()
            .any(|text| text == &format!("missing {} #2", old_key.raw())),
        "the old Field never aliases the reinserted row: {:?}",
        app.texts()
    );
    assert!(
        app.texts()
            .iter()
            .any(|text| text == &format!("memo missing {}", old_key.raw())),
        "the memoized consumer stays Missing after the reinsert: {:?}",
        app.texts()
    );
    assert!(
        !app.texts().iter().any(|text| text == "memo A"),
        "the memoized consumer never resolves the old handle to the new row: {:?}",
        app.texts()
    );
}

/// 过滤移出再移回：行身份不变，元素与本地编辑状态重新开始。
#[component]
fn FilterHost() -> MountResult<MountPlan> {
    let store = Store::new();
    let active = memo(move || store.rows(Filter::Active));
    let a_key = signal(None::<TodoId>);
    view! {
        <view>
            <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                Transaction::new()
                    .write(store.insert("A"))
                    .write(store.insert("B"))
                    .finish()?;
                a_key.set(Some(store.keys(Filter::All)[0]));
                Ok(())
            }}>"Seed"</button>
            <button aria:label="complete-a" on:click={move || -> Result<(), WriteError> {
                if let Some(key) = a_key.get() {
                    Transaction::new().write(store.set_completed(key, true)).finish()?;
                }
                Ok(())
            }}>"CompleteA"</button>
            <button aria:label="reactivate-a" on:click={move || -> Result<(), WriteError> {
                if let Some(key) = a_key.get() {
                    Transaction::new().write(store.set_completed(key, false)).finish()?;
                }
                Ok(())
            }}>"ReactivateA"</button>
            <For each={active} store={store} key={|row: &TodoRow| row.id}>
                {move |row: RowView<TodoRow>| {
                    let id = row.key();
                    view! {
                        <text-input
                            aria:label={once(row_label(id))}
                            value={store.title(id)}
                            on:input={move |_text: String| {}}
                        />
                    }
                }}
            </For>
        </view>
    }
}

#[test]
fn filtering_a_row_out_and_back_in_mounts_a_fresh_element() {
    let mut app = mount(|| view! { <FilterHost /> });
    app.click_label("seed")
        .expect("seed under the active filter");
    let before = todo_rows(&app);
    assert_eq!(before.len(), 2);
    let (row_id, key) = before[0];
    let label = row_label(key);
    let old_node = editor(&app, &label);
    let other_node = editor(&app, &row_label(before[1].1));

    app.focus(&label).expect("focus A");
    app.set_caret_node(old_node, 1).expect("place the caret");
    app.insert_text_node(old_node, "X")
        .expect("edit the mounted row");
    assert_eq!(
        app.presentation()
            .expect("snapshot")
            .find_label(&label)
            .expect("the editor")
            .text_value(),
        Some("AX")
    );

    app.click_label("complete-a")
        .expect("completing A removes it from Active");
    assert_eq!(todo_rows(&app), vec![before[1]]);
    assert_eq!(app.node_status(old_node), NodeStatus::Stale);
    assert_eq!(
        app.presentation().expect("snapshot").focus(),
        Some(other_node),
        "unmounting the focused row settles focus on the surviving editor"
    );

    app.click_label("reactivate-a")
        .expect("uncompleting A returns it to Active");
    let after = todo_rows(&app);
    assert_eq!(after.len(), 2);
    assert_eq!(
        after[0],
        (row_id, key),
        "filtering does not change the row identity"
    );
    let new_node = editor(&app, &label);
    assert_ne!(new_node, old_node, "the returning row mounts a fresh node");
    assert_eq!(app.node_status(old_node), NodeStatus::Stale);

    let snapshot = app.presentation().expect("snapshot");
    assert_eq!(
        snapshot.focus(),
        Some(other_node),
        "the returning row does not inherit focus"
    );
    let element = snapshot.find_label(&label).expect("the remounted editor");
    assert_eq!(
        element.text_value(),
        Some("A"),
        "the old local edit is not inherited"
    );
    let edit = element.edit().expect("edit state");
    assert_eq!(edit.caret(), 0);
    assert_eq!(edit.selection(), None);
    assert_eq!(
        app.insert_text_node(old_node, "Z"),
        Err(DispatchError::StaleNode(old_node)),
        "the unmounted row rejects delivery"
    );
}
