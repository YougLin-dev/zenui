//! headless 组件验收：不创建原生窗口也能挂载并点击计数器。

#![allow(
    unused_braces,
    reason = "{} is the intentionally supported event-expression syntax"
)]

use minimal_app::Starter;
use zenui::prelude::*;

#[test]
fn starter_mounts_headlessly_and_increments() {
    let mut app = Runtime::new();
    app.mount(|| view! { <Starter /> })
        .expect("mount the starter component");

    assert_eq!(
        app.texts(),
        vec!["Count: 0", "中文示例：你好，世界", "Increment"]
    );

    app.click_text("Increment").expect("click the counter");
    assert_eq!(
        app.texts(),
        vec!["Count: 1", "中文示例：你好，世界", "Increment"]
    );

    app.click_text("Increment")
        .expect("click the counter again");
    assert_eq!(
        app.texts(),
        vec!["Count: 2", "中文示例：你好，世界", "Increment"]
    );
}
