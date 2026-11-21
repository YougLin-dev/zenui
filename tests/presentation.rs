#![allow(dead_code, unused_braces)]

use std::cell::Cell;
use std::rc::Rc;

use zenui::prelude::*;
use zenui_demo::*;

fn runtime() -> Runtime {
    let mut app = Runtime::new();
    open_todo_window(&mut app).expect("open window");
    app
}

fn add(app: &mut Runtime, title: &str) {
    app.input("What needs to be done?", title).expect("input");
    app.submit("Add task").expect("submit");
}

/// Pre-order `(tag, direct text)` outline, with `-` for bare text nodes.
fn outline(nodes: &[SnapshotNode], out: &mut Vec<(String, String)>) {
    for node in nodes {
        match node {
            SnapshotNode::Text(text) => out.push((String::from("-"), text.clone())),
            SnapshotNode::Element(element) => {
                out.push((element.tag().to_owned(), element.text()));
                outline(element.children(), out);
            }
        }
    }
}

fn snapshot_outline(snapshot: &PresentationSnapshot) -> Vec<(String, String)> {
    let mut out = Vec::new();
    outline(snapshot.roots(), &mut out);
    out
}

fn pairs(rows: &[(&str, &str)]) -> Vec<(String, String)> {
    rows.iter()
        .map(|(tag, text)| ((*tag).to_owned(), (*text).to_owned()))
        .collect()
}

#[test]
fn a_mounted_todo_exposes_committed_elements_text_attributes_and_order() {
    let mut app = runtime();

    let empty = app.presentation().expect("the empty Todo is committed");
    assert_eq!(
        snapshot_outline(&empty),
        pairs(&[
            ("view", ""),
            ("text", "0 remaining"),
            ("-", "0 remaining"),
            ("form", ""),
            ("text-input", ""),
            ("button", "Add"),
            ("-", "Add"),
            ("view", ""),
            ("button", "All"),
            ("-", "All"),
            ("button", "Active"),
            ("-", "Active"),
            ("button", "Done"),
            ("-", "Done"),
            ("view", ""),
            ("button", "Reverse"),
            ("-", "Reverse"),
            ("button", "Clear done"),
            ("-", "Clear done"),
            ("button", "Load sample"),
            ("-", "Load sample"),
            ("button", "Preview import"),
            ("-", "Preview import"),
            ("button", "Theme"),
            ("-", "Theme"),
            ("text", "Nothing here yet"),
            ("-", "Nothing here yet"),
        ]),
        "the committed tree keeps declaration order"
    );

    let root = empty.roots()[0]
        .as_element()
        .expect("the Todo is rooted at an element");
    assert_eq!(root.tag(), "view");
    assert_eq!(
        root.attr("role").and_then(SnapshotAttr::as_text),
        Some("main")
    );

    let form = empty.find_label("Add task").expect("the form label");
    assert_eq!(form.tag(), "form");
    let draft = empty
        .find_label("What needs to be done?")
        .expect("the draft input");
    assert_eq!(draft.tag(), "text-input");
    assert_eq!(draft.text_value(), Some(""));

    add(&mut app, "Write tests");
    add(&mut app, "Read docs");
    app.change("Complete Write tests", true)
        .expect("complete the first row");

    let populated = app.presentation().expect("the populated Todo is committed");
    let outline = snapshot_outline(&populated);
    assert_eq!(
        outline[1],
        (String::from("text"), String::from("1 remaining"))
    );
    assert!(
        !outline
            .iter()
            .any(|(tag, text)| tag == "text" && text == "Nothing here yet"),
        "the empty-state branch is not committed"
    );

    let complete = populated
        .find_label("Complete Write tests")
        .expect("the first row checkbox");
    assert_eq!(complete.tag(), "checkbox");
    assert_eq!(
        complete
            .attrs()
            .iter()
            .map(SnapshotAttr::name)
            .collect::<Vec<_>>(),
        vec!["checked", "aria:label"],
        "presentation attributes keep declaration order"
    );
    assert_eq!(complete.checked(), Some(true));
    assert_eq!(complete.label(), Some("Complete Write tests"));
    assert_eq!(
        populated.node(complete.id()).map(SnapshotElement::tag),
        Some("checkbox"),
        "an element is reachable by its Node ID"
    );

    let edit = populated
        .find_label("Edit Write tests")
        .expect("the first row editor");
    assert_eq!(edit.tag(), "text-input");
    assert_eq!(edit.text_value(), Some(""));
    assert_eq!(edit.label(), Some("Edit Write tests"));

    let delete = populated
        .find_label("Delete Write tests")
        .expect("the first row delete button");
    assert_eq!(delete.tag(), "button");
    assert_eq!(delete.text(), "Delete");

    let row_tags = populated
        .elements()
        .filter(|element| element.text() == "Write tests")
        .map(SnapshotElement::tag)
        .collect::<Vec<_>>();
    assert_eq!(row_tags, vec!["text"], "the row title is a text element");

    let first = populated
        .elements()
        .position(|element| element.label() == Some("Complete Write tests"))
        .expect("the first row is committed");
    let second = populated
        .elements()
        .position(|element| element.label() == Some("Complete Read docs"))
        .expect("the second row is committed");
    assert!(first < second, "rows keep business order");
}

fn label_order(snapshot: &PresentationSnapshot, labels: &[&str]) -> Vec<String> {
    let wanted: Vec<&str> = labels.to_vec();
    snapshot
        .elements()
        .filter_map(SnapshotElement::label)
        .filter(|label| wanted.contains(label))
        .map(str::to_owned)
        .collect()
}

#[test]
fn node_ids_survive_reorder_and_filtering_and_unmounted_ids_are_stale() {
    let mut app = runtime();
    add(&mut app, "A");
    add(&mut app, "B");

    let before = app.presentation().expect("snapshot");
    let complete_a = before.find_label("Complete A").expect("A checkbox").id();
    let edit_a = before.find_label("Edit A").expect("A editor").id();
    let complete_b = before.find_label("Complete B").expect("B checkbox").id();
    let unique: std::collections::BTreeSet<NodeId> =
        before.elements().map(SnapshotElement::id).collect();
    assert_eq!(
        unique.len(),
        before.elements().count(),
        "every committed element has its own Node ID"
    );

    app.click_text("Reverse").expect("reorder");
    let reordered = app.presentation().expect("snapshot");
    assert_eq!(
        reordered.find_label("Complete A").map(SnapshotElement::id),
        Some(complete_a),
        "reorder keeps the surviving identity"
    );
    assert_eq!(
        reordered.find_label("Edit A").map(SnapshotElement::id),
        Some(edit_a)
    );
    assert_eq!(
        label_order(&reordered, &["Complete A", "Complete B"]),
        vec![String::from("Complete B"), String::from("Complete A")],
        "the fixture must really reorder the rows"
    );

    app.change("Complete B", true).expect("complete B");
    app.click_text("Active").expect("filter active");
    let filtered = app.presentation().expect("snapshot");
    assert!(
        filtered.find_label("Complete B").is_none(),
        "a filtered row is not committed"
    );
    assert!(
        filtered.node(complete_b).is_none(),
        "an unmounted ID resolves to nothing in the snapshot"
    );
    assert_eq!(app.node_status(complete_b), NodeStatus::Stale);
    assert_eq!(
        filtered.find_label("Complete A").map(SnapshotElement::id),
        Some(complete_a),
        "filtering another row keeps this row's identity"
    );

    app.click_text("All").expect("filter all");
    let remounted = app.presentation().expect("snapshot");
    let returned = remounted.find_label("Complete B").expect("B returns").id();
    assert_ne!(returned, complete_b, "a new mount gets new IDs");
    assert_eq!(
        app.node_status(complete_b),
        NodeStatus::Stale,
        "the unmounted ID is distinguishable from a live one"
    );
    assert_eq!(app.node_status(returned), NodeStatus::Live);

    let mut other = runtime();
    add(&mut other, "C");
    assert_eq!(
        other.node_status(complete_a),
        NodeStatus::Unknown,
        "an ID from another Runtime was never minted here"
    );
}

#[test]
fn focus_and_local_text_edit_state_appear_in_the_snapshot() {
    let mut app = runtime();
    add(&mut app, "A");

    let idle = app.presentation().expect("snapshot");
    assert_eq!(idle.focus(), None);
    let editor = idle.find_label("Edit A").expect("the row editor");
    let edit = editor.edit().expect("a text input carries edit state");
    assert_eq!(edit.caret(), 0);
    assert_eq!(edit.selection(), None);
    assert_eq!(edit.preedit(), None);
    assert!(
        idle.find_label("Complete A")
            .expect("the row checkbox")
            .edit()
            .is_none(),
        "only text inputs carry local text-edit state"
    );

    app.focus("Edit A").expect("focus the editor");
    assert!(app.set_caret("Edit A", 1), "place the caret");
    let focused = app.presentation().expect("snapshot");
    let editor = focused.find_label("Edit A").expect("the row editor");
    assert_eq!(focused.focus(), Some(editor.id()));
    let edit = editor.edit().expect("edit state");
    assert_eq!(edit.caret(), 1);
    assert_eq!(edit.selection(), Some((1, 1)));

    app.compose_start("Edit A").expect("start composing");
    app.compose_update("Edit A", "ni").expect("update preedit");
    let composing = app.presentation().expect("snapshot");
    let edit = composing
        .find_label("Edit A")
        .expect("the row editor")
        .edit()
        .expect("edit state");
    assert_eq!(edit.preedit(), Some("ni"));
    assert_eq!(
        edit.composition(),
        Some((1, 1)),
        "composition replaces its base range"
    );
    assert_eq!(edit.caret(), 3, "the caret follows the preedit end");
    assert_eq!(edit.selection(), None);

    app.compose_cancel("Edit A").expect("cancel composing");
    let cancelled = app.presentation().expect("snapshot");
    let edit = cancelled
        .find_label("Edit A")
        .expect("the row editor")
        .edit()
        .expect("edit state");
    assert_eq!(edit.preedit(), None);
    assert_eq!(edit.composition(), None);
    assert_eq!(edit.caret(), 1, "cancel restores the caret");
    assert_eq!(edit.selection(), Some((1, 1)));

    app.click_text("Reverse").expect("reorder");
    let reordered = app.presentation().expect("snapshot");
    assert_eq!(
        reordered.focus(),
        Some(reordered.find_label("Edit A").expect("the row editor").id()),
        "focus keeps its Node ID across reorder"
    );
    let edit = reordered
        .find_label("Edit A")
        .expect("the row editor")
        .edit()
        .expect("edit state");
    assert_eq!(edit.caret(), 1, "local edit state survives reorder");

    app.blur().expect("clear focus");
    assert_eq!(app.presentation().expect("snapshot").focus(), None);
}

#[component]
fn BareCheckbox() -> MountResult<MountPlan> {
    view! {
        <checkbox aria:label="bare" on:change={move |_done: bool| {}} />
    }
}

#[test]
fn local_checkbox_state_is_presented_without_a_bound_attribute() {
    let mut app = Runtime::new();
    app.mount(|| view! { <BareCheckbox /> }).expect("mount");

    let initial = app.presentation().expect("snapshot");
    assert_eq!(
        initial.find_label("bare").expect("the checkbox").checked(),
        Some(false),
        "an uncontrolled checkbox presents its local state"
    );

    app.change("bare", true).expect("toggle locally");
    let checked = app.presentation().expect("snapshot");
    assert_eq!(
        checked.find_label("bare").expect("the checkbox").checked(),
        Some(true)
    );
}

#[component]
fn Branchy() -> MountResult<MountPlan> {
    let store = Store::new();
    let shown = signal(true);
    let when = memo(move || shown.get());
    let rows = memo(move || store.rows(Filter::All));

    view! {
        <view>
            <button aria:label="toggle" on:click={move || shown.set(!shown.get())}>"Toggle"</button>
            <Show
                when={when}
                fallback={move || view! { <text aria:label="fallback">"off"</text> }}
            >
                <text aria:label="on">"on"</text>
            </Show>
            <button aria:label="seed" on:click={move || {
                let _ = Transaction::new()
                    .write(store.insert("A"))
                    .write(store.insert("B"))
                    .finish();
            }}>"Seed"</button>
            <For each={rows} key={|id: TodoId| id}>
                {move |todo: TodoId| view! {
                    <text aria:label={once(format!("row {}", todo.raw()))} />
                }}
            </For>
        </view>
    }
}

#[test]
fn show_and_for_expose_only_committed_visible_nodes() {
    let mut app = Runtime::new();
    app.mount(|| view! { <Branchy /> }).expect("mount");

    let initial = app.presentation().expect("snapshot");
    let on = initial.find_label("on").expect("the then branch");
    let on_id = on.id();
    assert_eq!(on.text(), "on");
    assert!(
        initial.find_label("fallback").is_none(),
        "the inactive fallback is not committed"
    );

    app.click_label("toggle").expect("show the fallback branch");
    let fallback = app.presentation().expect("snapshot");
    assert!(fallback.find_label("on").is_none());
    assert_eq!(
        fallback.find_label("fallback").expect("fallback").text(),
        "off"
    );
    assert_eq!(
        app.node_status(on_id),
        NodeStatus::Stale,
        "an unmounted branch node is stale"
    );

    app.click_label("toggle")
        .expect("show the then branch again");
    let again = app.presentation().expect("snapshot");
    let returned = again.find_label("on").expect("the then branch returns");
    assert_ne!(returned.id(), on_id, "a new Show mount gets a new ID");

    app.click_label("seed").expect("seed two rows");
    let seeded = app.presentation().expect("snapshot");
    assert_eq!(
        label_order(&seeded, &["row 0", "row 1", "row 2"]),
        vec![String::from("row 1"), String::from("row 2")],
        "only committed For items appear, in business order"
    );
    assert_eq!(
        seeded
            .elements()
            .filter(|element| element
                .label()
                .is_some_and(|label| label.starts_with("row ")))
            .count(),
        2,
        "the empty list committed no item nodes"
    );
}

#[component]
fn Instrumented(runs: Rc<Cell<u32>>) -> MountResult<MountPlan> {
    let value = signal(String::from("one"));
    let label = {
        let runs = runs.clone();
        memo(move || {
            runs.set(runs.get() + 1);
            value.get()
        })
    };

    view! {
        <view>
            <text>{label}</text>
            <button aria:label="edit" on:click={move || value.set(String::from("two"))}>"Edit"</button>
        </view>
    }
}

fn assert_send_sync_static<T: Send + Sync + 'static>() {}

#[test]
fn snapshot_generation_runs_no_user_closures_and_changes_nothing() {
    let runs = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let runs = runs.clone();
        move || view! { <Instrumented runs={runs} /> }
    })
    .expect("mount");
    app.click_label("edit").expect("recompute the memo once");
    let computed = runs.get();

    let counters = app.counters();
    let texts = app.texts();
    let once_reads = app.once_reads();
    let unmounts = app.unmounts();
    let signals = app.live_signals();
    let memos = app.live_memos();
    let setups = app.setup_count("Instrumented");

    let first = app.presentation().expect("snapshot");
    let second = app.presentation().expect("snapshot again");

    assert_eq!(runs.get(), computed, "the snapshot must not run a memo");
    assert_eq!(app.counters(), counters, "the snapshot changes no counters");
    assert_eq!(app.texts(), texts, "the snapshot does not mutate bindings");
    assert_eq!(app.once_reads(), once_reads);
    assert_eq!(app.unmounts(), unmounts);
    assert_eq!(app.live_signals(), signals);
    assert_eq!(app.live_memos(), memos);
    assert_eq!(app.setup_count("Instrumented"), setups);
    assert_eq!(
        first, second,
        "a committed interface presents deterministically"
    );
    assert_eq!(
        first
            .elements()
            .map(|element| element.tag())
            .collect::<Vec<_>>(),
        vec!["view", "text", "button"],
        "the fixture generated a real snapshot"
    );

    // The snapshot is owned plain data: no Rc closures, runtime handles or platform handles.
    assert_send_sync_static::<PresentationSnapshot>();
}

#[test]
fn a_snapshot_requires_a_committed_interface() {
    let app = Runtime::new();
    assert_eq!(
        app.presentation().expect_err("nothing is committed yet"),
        MountError::Component("presentation snapshot requires a visible committed interface")
    );

    let mut app = Runtime::new();
    app.mount(|| view! { <text>"ok"</text> }).expect("mount");
    assert!(
        app.presentation().is_ok(),
        "a committed mount is snapshotable"
    );
}

#[component]
fn Printable() -> MountResult<MountPlan> {
    let draft = signal(String::new());
    let hello = signal(String::from("hello"));

    view! {
        <view role="main">
            <text>{hello}</text>
            <text-input aria:label="Edit" value={draft} />
            <button aria:label="Go">"Go"</button>
        </view>
    }
}

/// Drop `#<digits>` node numbers so the assertion checks tree shape, not allocation order.
fn without_node_numbers(printed: &str) -> String {
    let mut out = String::with_capacity(printed.len());
    let mut chars = printed.chars().peekable();
    while let Some(current) = chars.next() {
        out.push(current);
        if current == '#' {
            while chars.peek().is_some_and(char::is_ascii_digit) {
                chars.next();
            }
        }
    }
    out
}

#[test]
fn the_snapshot_display_prints_the_committed_tree_for_inspection() {
    let mut app = Runtime::new();
    app.mount(|| view! { <Printable /> }).expect("mount");

    let printed = app.presentation().expect("snapshot").to_string();
    assert_eq!(
        without_node_numbers(&printed),
        concat!(
            "focus=-\n",
            "view# role=\"main\"\n",
            "  text#\n",
            "    \"hello\"\n",
            "  text-input# aria:label=\"Edit\" value=\"\" caret=0\n",
            "  button# aria:label=\"Go\"\n",
            "    \"Go\"\n",
        )
    );

    app.focus("Go").expect("focus the button");
    let focused = app.presentation().expect("snapshot").to_string();
    let focus_token = focused
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("focus="))
        .expect("the focus line");
    let button = focused
        .lines()
        .find(|line| line.trim_start().starts_with("button#"))
        .expect("the button line");
    assert!(
        button
            .trim_start()
            .starts_with(&format!("button{focus_token}")),
        "the focused node is marked in the printed snapshot: {focused}"
    );
}

#[component]
fn DeadButton() -> MountResult<MountPlan> {
    view! { <button aria:label="dead" disabled={true}>"Dead"</button> }
}

#[test]
fn presentation_attributes_include_disabled() {
    let mut app = Runtime::new();
    app.mount(|| view! { <DeadButton /> }).expect("mount");

    let snapshot = app.presentation().expect("snapshot");
    let dead = snapshot.find_label("dead").expect("the disabled button");
    assert!(dead.disabled());
    assert_eq!(
        dead.attr("disabled").and_then(SnapshotAttr::as_bool),
        Some(true)
    );
}

#[component]
fn DynamicStyles() -> MountResult<MountPlan> {
    let width = signal(px(100.0));
    let tint = signal(Color::Role(ColorRole::Accent));

    view! {
        <view>
            <button aria:label="wider" on:click={move || width.set(px(160.0))}>"Wider"</button>
            <button aria:label="recolor" on:click={move || tint.set(Color::Role(ColorRole::Danger))}>"Recolor"</button>
            <view
                aria:label="styled"
                style:width={width}
                style:background={tint}
                style:color={once(Color::Role(ColorRole::TextMuted))}
                style:border-color={Color::Role(ColorRole::BorderAccent)}
                style:opacity={Opacity::percent(50)}
            />
        </view>
    }
}

#[test]
fn typed_dynamic_styles_travel_through_the_snapshot_and_update_in_place() {
    let mut app = Runtime::new();
    app.mount(|| view! { <DynamicStyles /> }).expect("mount");

    let before = app.presentation().expect("snapshot");
    let styled = before.find_label("styled").expect("styled view");
    let id = styled.id();
    assert_eq!(
        styled.styles(),
        &[
            StyleDeclaration::inline(Property::Width, StyleValue::Length(px(100.0))),
            StyleDeclaration::inline(
                Property::Background,
                StyleValue::Color(Color::Role(ColorRole::Accent))
            ),
            StyleDeclaration::inline(
                Property::TextColor,
                StyleValue::Color(Color::Role(ColorRole::TextMuted))
            ),
            StyleDeclaration::inline(
                Property::BorderColor,
                StyleValue::Color(Color::Role(ColorRole::BorderAccent))
            ),
            StyleDeclaration::inline(Property::Opacity, StyleValue::Opacity(50)),
        ],
        "plain, Once and Signal values reach the snapshot as inline declarations"
    );

    app.click_label("wider").expect("update the width");
    let after = app.presentation().expect("snapshot");
    let styled = after.find_label("styled").expect("styled view");
    assert_eq!(styled.id(), id, "a value update does not remount");
    assert_eq!(
        styled.styles()[0],
        StyleDeclaration::inline(Property::Width, StyleValue::Length(px(160.0)))
    );
    assert_ne!(before, after, "the committed snapshot reflects the change");
}

#[component]
fn ThemeProbe(
    window: WindowCtx,
    themed: Rc<Cell<u32>>,
    plain: Rc<Cell<u32>>,
) -> MountResult<MountPlan> {
    let label = memo(move || {
        themed.set(themed.get() + 1);
        if window.appearance() == Ok(Appearance::Dark) {
            String::from("dark")
        } else {
            String::from("light")
        }
    });
    let fixed = memo(move || {
        plain.set(plain.get() + 1);
        String::from("fixed")
    });

    view! {
        <view>
            <text aria:label="theme">{label}</text>
            <text aria:label="fixed">{fixed}</text>
        </view>
    }
}

#[test]
fn appearance_is_committed_state_and_only_theme_memos_recompute() {
    let themed = Rc::new(Cell::new(0));
    let plain = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.open_window("main", {
        let themed = themed.clone();
        let plain = plain.clone();
        move |window| {
            let themed = themed.clone();
            let plain = plain.clone();
            view! { <ThemeProbe window={window} themed={themed} plain={plain} /> }
        }
    })
    .expect("open window");

    let light = app.presentation().expect("snapshot");
    assert_eq!(light.appearance(), Appearance::Light);
    assert_eq!(
        light.find_label("theme").expect("theme text").text(),
        "light"
    );
    assert_eq!((themed.get(), plain.get()), (1, 1));

    app.set_appearance(Appearance::Dark).expect("switch theme");
    let dark = app.presentation().expect("snapshot");
    assert_eq!(dark.appearance(), Appearance::Dark);
    assert_eq!(dark.find_label("theme").expect("theme text").text(), "dark");
    assert_ne!(light, dark, "appearance participates in snapshot equality");
    assert_eq!(themed.get(), 2, "the theme memo recomputes");
    assert_eq!(plain.get(), 1, "an unrelated memo does not recompute");
}

#[test]
fn the_demo_theme_toggle_keeps_focus_caret_and_selection() {
    let mut app = runtime();
    add(&mut app, "A");
    app.focus("Edit A").expect("focus the editor");
    assert!(app.set_caret("Edit A", 1), "place the caret");
    let before = app.presentation().expect("snapshot");
    let editor = before.find_label("Edit A").expect("editor").id();
    let edit = before
        .find_label("Edit A")
        .expect("editor")
        .edit()
        .expect("edit state");
    let caret = (edit.caret(), edit.anchor());

    app.click_label("Toggle theme").expect("toggle the theme");
    let dark = app.presentation().expect("snapshot");
    assert_eq!(dark.appearance(), Appearance::Dark);
    assert_eq!(
        dark.focus(),
        Some(editor),
        "focus survives the theme switch"
    );
    let edit = dark
        .find_label("Edit A")
        .expect("editor")
        .edit()
        .expect("edit state");
    assert_eq!((edit.caret(), edit.anchor()), caret, "caret survives");

    app.click_label("Toggle theme").expect("toggle back");
    assert_eq!(
        app.presentation().expect("snapshot").appearance(),
        Appearance::Light
    );
}

#[test]
fn class_literals_become_typed_declarations() {
    let mut app = Runtime::new();
    app.mount(|| view! { <button class="px-3 py-2 flex-row bg-accent rounded-md">"Add"</button> })
        .expect("mount");

    let snapshot = app.presentation().expect("snapshot");
    let button = snapshot
        .elements()
        .find(|element| element.tag() == "button")
        .expect("button");
    assert_eq!(
        button.styles(),
        &[
            StyleDeclaration::new(Property::PaddingX, StyleValue::Points(12), Condition::Base),
            StyleDeclaration::new(Property::PaddingY, StyleValue::Points(8), Condition::Base),
            StyleDeclaration::new(Property::FlexDirection, StyleValue::Row, Condition::Base),
            StyleDeclaration::new(
                Property::Background,
                StyleValue::Color(Color::Role(ColorRole::Accent)),
                Condition::Base,
            ),
            StyleDeclaration::new(Property::Radius, StyleValue::Points(6), Condition::Base),
        ]
    );
    assert!(
        button.attr("class").is_none(),
        "class is compiled away, not a presentation attribute"
    );
}
