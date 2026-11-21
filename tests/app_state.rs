#![allow(unused_braces)]

use zenui::prelude::*;

#[test]
fn application_setup_shares_state_with_the_window_factory() {
    let mut app = Runtime::new();
    let value = app
        .setup(|| signal(String::from("app-owned")))
        .expect("application setup");
    let factory_value = value;
    app.open_window("main", move |_| {
        assert_eq!(factory_value.get(), "app-owned");
        view! {
            <view>
                <text aria:label="value">{factory_value}</text>
                <button
                    aria:label="update"
                    on:click={move || factory_value.set(String::from("updated"))}
                >"Update"</button>
            </view>
        }
    })
    .expect("window");
    let snapshot = app.presentation().expect("snapshot");
    assert_eq!(
        snapshot.find_label("value").expect("value text").text(),
        "app-owned"
    );

    app.click_label("update").expect("update the app signal");
    let snapshot = app.presentation().expect("snapshot");
    assert_eq!(
        snapshot.find_label("value").expect("value text").text(),
        "updated"
    );
}

#[test]
fn application_setup_must_run_before_a_window_mounts() {
    let mut app = Runtime::new();
    app.open_window("main", |_| view! { <view /> })
        .expect("window");
    let error = app
        .setup(|| ())
        .expect_err("setup after a mounted window must fail");
    assert!(matches!(error, MountError::Component(_)));
}

#[test]
fn application_state_survives_window_close_and_reopen() {
    let mut app = Runtime::new();
    let value = app
        .setup(|| signal(String::from("kept")))
        .expect("application setup");
    let first = value;
    let window = app
        .open_window("main", move |_| view! { <text>{first}</text> })
        .expect("open window");
    app.close_window(window).expect("close window");

    let second = value;
    app.open_window("main", move |_| view! { <text>{second}</text> })
        .expect("reopen window");
    let snapshot = app.presentation().expect("snapshot");
    assert!(
        snapshot.elements().any(|element| element.text() == "kept"),
        "the app-owned signal survives the window owner"
    );
}
