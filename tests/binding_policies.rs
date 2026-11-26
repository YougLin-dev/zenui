//! `Field` 求值失败的 part 策略：有最后值时保留已提交值；首次失败时文本为空、
//! bool 属性缺席、`Show` 不挂载。失败不是把缺失行解释成空串或 `false`。

#![allow(unused_braces)]

use zenui::prelude::*;

#[component]
fn MissingBindings(store: Store, id: TodoId) -> MountResult<MountPlan> {
    view! {
        <view>
            <view aria:label="bound"><text>{store.title(id)}</text></view>
            <button aria:label="disabled" disabled={store.completed(id)}>"button"</button>
            <Show when={store.completed(id)}><text>"inside"</text></Show>
        </view>
    }
}

#[component]
fn Host() -> MountResult<MountPlan> {
    let store = Store::new();
    let target: Signal<Option<TodoId>> = signal(None);
    let ready = signal(false);
    view! {
        <view>
            <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                Transaction::new().write(store.insert("row")).finish()?;
                let id = store
                    .rows(Filter::All)
                    .iter()
                    .next()
                    .map(TodoId::from_key)
                    .expect("the inserted row");
                target.set(Some(id));
                Ok(())
            }}>"seed"</button>
            <button aria:label="delete" on:click={move || -> Result<(), WriteError> {
                let id = target.get().expect("seeded");
                Transaction::new().write(store.remove(id)).finish()
            }}>"delete"</button>
            <button aria:label="bind" on:click={move || ready.set(true)}>"bind"</button>
            <Show when={ready}>
                <MissingBindings store={store} id={target.get().expect("seeded")} />
            </Show>
        </view>
    }
}

#[component]
fn KeepHost() -> MountResult<MountPlan> {
    let store = Store::new();
    let target: Signal<Option<TodoId>> = signal(None);
    let ready = signal(false);
    view! {
        <view>
            <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                Transaction::new().write(store.insert("row")).finish()?;
                let id = store
                    .rows(Filter::All)
                    .iter()
                    .next()
                    .map(TodoId::from_key)
                    .expect("the inserted row");
                target.set(Some(id));
                Ok(())
            }}>"seed"</button>
            <button aria:label="complete" on:click={move || -> Result<(), WriteError> {
                let id = target.get().expect("seeded");
                Transaction::new()
                    .write(store.set_completed(id, true))
                    .finish()
            }}>"complete"</button>
            <button aria:label="bind" on:click={move || ready.set(true)}>"bind"</button>
            <button aria:label="delete" on:click={move || -> Result<(), WriteError> {
                let id = target.get().expect("seeded");
                Transaction::new().write(store.remove(id)).finish()
            }}>"delete"</button>
            <Show when={ready}>
                <MissingBindings store={store} id={target.get().expect("seeded")} />
            </Show>
        </view>
    }
}

#[test]
fn a_later_field_failure_keeps_the_last_text_attr_and_show_branch() {
    let mut app = Runtime::new();
    app.mount(|| view! { <KeepHost /> }).expect("mount");

    app.click_label("seed").expect("seed a row");
    app.click_label("complete").expect("complete the row");
    app.click_label("bind").expect("bind the live row");
    {
        let snapshot = app.presentation().expect("snapshot");
        assert!(
            snapshot.find_label("bound").is_some(),
            "the bound element is mounted"
        );
        assert!(
            app.texts().iter().any(|text| text == "row"),
            "the field text is committed while the row exists"
        );
        assert!(
            snapshot
                .find_label("disabled")
                .expect("the button")
                .disabled()
        );
        assert!(app.texts().iter().any(|text| text == "inside"));
    }

    app.click_label("delete").expect("delete the row");
    let report = app.refresh().expect("refresh reports the fault");
    assert!(
        report
            .faults()
            .iter()
            .any(|fault| matches!(fault, ReadFault::FieldMissing(_))),
        "the removed row is a typed fault: {:?}",
        report.faults()
    );

    let snapshot = app.presentation().expect("snapshot");
    assert!(
        snapshot.find_label("bound").is_some(),
        "the bound element stays mounted"
    );
    assert!(
        app.texts().iter().any(|text| text == "row"),
        "a later text failure keeps the last committed text"
    );
    assert!(
        snapshot
            .find_label("disabled")
            .expect("the button")
            .disabled(),
        "a later bool failure keeps the last committed value"
    );
    assert!(
        app.texts().iter().any(|text| text == "inside"),
        "a later Show failure keeps the active branch mounted"
    );
}

#[test]
fn a_first_field_failure_leaves_text_empty_and_the_boolean_attr_absent() {
    let mut app = Runtime::new();
    app.mount(|| view! { <Host /> }).expect("mount");

    app.click_label("seed").expect("seed a row");
    app.click_label("delete").expect("delete the row");
    app.click_label("bind").expect("bind the deleted row");

    let snapshot = app.presentation().expect("snapshot");
    let bound = snapshot.find_label("bound").expect("the bound element");
    assert!(
        bound.children().iter().any(
            |child| matches!(child, SnapshotNode::Element(element) if element.tag() == "text")
        ),
        "a first text failure keeps the text element mounted"
    );
    assert!(
        app.texts().iter().all(|text| text != "row"),
        "a first text failure commits no field text"
    );
    let disabled = snapshot.find_label("disabled").expect("the button");
    assert!(
        disabled.attr("disabled").is_none(),
        "a first bool attribute failure stays absent instead of becoming false"
    );
    assert!(!disabled.disabled());
    assert!(
        app.texts().iter().all(|text| text != "inside"),
        "a first Show failure keeps the branch unmounted"
    );

    let report = app.refresh().expect("refresh reports the fault");
    assert!(
        report
            .faults()
            .iter()
            .any(|fault| matches!(fault, ReadFault::FieldMissing(_))),
        "the missing row is a typed fault: {:?}",
        report.faults()
    );
}
