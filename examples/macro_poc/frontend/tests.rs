use std::cell::Cell;
use std::rc::Rc;
use zenui_macro_facade::{
    runtime::{Condition, Property, StyleRule, Value},
    view,
};

#[allow(
    unused_braces,
    reason = "{} is the intentionally supported event-expression syntax"
)]
#[test]
fn html_shape_compiles_classes_and_preserves_rust_closures() {
    let count = Rc::new(Cell::new(0));
    let binding_reads = Rc::new(Cell::new(0));
    let read_count = count.clone();
    let record_read = binding_reads.clone();
    let event_count = count.clone();
    let mut mounted = view! {
        <view class="flex flex-col w-72 bg-blue-600 hover:bg-blue-800 active:bg-blue-800">
            <text>{move || { record_read.set(record_read.get() + 1); read_count.get().to_string() }}</text>
            <button on:click={move || event_count.set(event_count.get() + 1)}>"打开"</button>
        </view>
    };
    assert_eq!(mounted.children.len(), 2);
    assert!(mounted.styles.contains(&StyleRule {
        property: Property::Width,
        value: Value::LogicalPoints(288.0),
        condition: Condition::Always,
    }));
    assert_eq!(binding_reads.get(), 0, "bindings are not eagerly evaluated");
    assert_eq!(count.get(), 0, "mount never dispatches events");
    assert_eq!(mounted.children[0].evaluate_content().as_deref(), Some("0"));
    assert_eq!(binding_reads.get(), 1, "explicit evaluation occurs once");
    assert!(
        !mounted.fire("click"),
        "events are not implicitly retargeted"
    );
    assert!(mounted.children[1].fire("click"));
    assert_eq!(count.get(), 1);
    assert_eq!(mounted.children[0].evaluate_content().as_deref(), Some("1"));
    assert_eq!(mounted.children[1].content(), Some("打开"));
}

#[test]
fn literal_rust_text_is_static_content_not_a_child_container() {
    let mounted = view! { <text>{"标题"}</text> };
    assert_eq!(mounted.content(), Some("标题"));
    assert!(mounted.children.is_empty());
}
