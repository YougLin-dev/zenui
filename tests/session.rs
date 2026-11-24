//! 应用会话策略、显式退出与会话级窗口动作。

#![allow(unused_braces)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use zenui::prelude::*;

#[test]
fn the_default_session_quits_on_the_last_close_and_is_declared_once() {
    let mut app = Runtime::new();
    assert_eq!(app.session_policy(), SessionPolicy::QuitOnLastClose);
    app.declare_session(SessionPolicy::StayResident)
        .expect("declare the resident policy");
    assert_eq!(app.session_policy(), SessionPolicy::StayResident);
    assert_eq!(
        app.declare_session(SessionPolicy::QuitOnLastClose),
        Err(SessionError::AlreadyDeclared)
    );

    let mut started = Runtime::new();
    started
        .declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
            Ok(())
        })
        .expect("declare the window");
    started.show("main").expect("open the window");
    assert_eq!(
        started.declare_session(SessionPolicy::StayResident),
        Err(SessionError::RegistrationClosed)
    );
}

#[test]
fn an_app_command_can_quit_and_close_every_window() {
    let runs = Rc::new(Cell::new(0));
    let handler_runs = runs.clone();
    let mut app = Runtime::new();
    let session = app.session();
    app.on_app_command("app.quit", move |_| {
        handler_runs.set(handler_runs.get() + 1);
        session.request_quit().expect("queue the quit");
    })
    .expect("register the quit command");
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
        catalog.window(WindowSpec::new("details"), |_| view! { <view /> })?;
        Ok(())
    })
    .expect("declare the windows");
    app.show("main").expect("open main");
    app.show("details").expect("open details");
    assert_eq!(app.windows().len(), 2);

    app.dispatch_app_command("app.quit", CommandSource::Tray, None)
        .expect("run the quit command");
    assert_eq!(runs.get(), 1);
    assert_eq!(app.windows().len(), 2, "quit waits for settlement");
    assert!(app.session_alive());

    assert_eq!(app.apply_session_requests().expect("settle the quit"), 1);
    assert!(app.windows().is_empty(), "quit closes every activation");
    assert!(!app.session_alive());
    assert!(matches!(
        app.show("main"),
        Err(WindowError::State("the application session has ended"))
    ));
}

#[test]
fn a_session_show_reopens_the_last_closed_window() {
    let mut app = Runtime::new();
    let session = app.session();
    app.on_app_command("app.show", move |_| {
        session.request_show("main").expect("queue the show");
    })
    .expect("register the show command");
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
        Ok(())
    })
    .expect("declare the window");
    let first = app.show("main").expect("open main");
    app.close_window(first).expect("close the only window");
    assert!(app.windows().is_empty());

    app.dispatch_app_command("app.show", CommandSource::Tray, None)
        .expect("run the show command");
    assert_eq!(app.apply_session_requests().expect("settle the show"), 1);
    assert_eq!(app.windows()[0].spec.key(), "main");
    let reopened = app.windows()[0].window;
    assert_ne!(reopened, first, "reopening creates a new activation");
    assert!(app.window_alive(reopened));
}

#[test]
fn a_session_show_reveals_a_hidden_window_without_remounting() {
    let mut app = Runtime::new();
    let session = app.session();
    app.on_app_command("app.show", move |_| {
        session.request_show("main").expect("queue the show");
    })
    .expect("register the show command");
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
        Ok(())
    })
    .expect("declare the window");
    let window = app.show("main").expect("open main");
    app.hide(window).expect("hide main");
    assert!(!app.windows()[0].visible);

    app.dispatch_app_command("app.show", CommandSource::Tray, None)
        .expect("run the show command");
    assert_eq!(app.apply_session_requests().expect("settle the show"), 1);
    assert_eq!(app.windows()[0].window, window, "show keeps the activation");
    assert!(app.windows()[0].visible);
}

#[test]
fn a_session_show_rejects_an_unknown_key_at_queue_time() {
    let error = Rc::new(RefCell::new(None));
    let handler_error = error.clone();
    let mut app = Runtime::new();
    let session = app.session();
    app.on_app_command("app.show", move |_| {
        *handler_error.borrow_mut() = Some(
            session
                .request_show("missing")
                .expect_err("an unknown key is rejected"),
        );
    })
    .expect("register the show command");
    app.dispatch_app_command("app.show", CommandSource::Tray, None)
        .expect("run the show command");
    assert_eq!(
        error.borrow().clone(),
        Some(SessionError::UnknownWindow("missing"))
    );
    assert_eq!(app.apply_session_requests().expect("nothing to settle"), 0);
}

#[test]
fn quit_closes_a_stage_one_window_and_rejects_later_opens() {
    let mut app = Runtime::new();
    let window = app
        .open_window("main", |_| view! { <view /> })
        .expect("open the stage-one window");
    assert!(app.window_alive(window));

    app.quit().expect("quit the session");
    assert!(!app.window_alive(window));
    assert!(!app.session_alive());
    assert!(matches!(
        app.open_window("again", |_| view! { <view /> }),
        Err(WindowError::State("the application session has ended"))
    ));
}

#[test]
fn a_show_queued_after_quit_is_discarded_by_settlement() {
    let mut app = Runtime::new();
    let session = app.session();
    app.on_app_command("app.quit_then_show", move |_| {
        session.request_quit().expect("queue the quit");
        session.request_show("main").expect("queue the show");
    })
    .expect("register the command");
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
        Ok(())
    })
    .expect("declare the window");

    app.dispatch_app_command("app.quit_then_show", CommandSource::Tray, None)
        .expect("run the command");
    assert_eq!(
        app.apply_session_requests()
            .expect("quit settles without an error"),
        1
    );
    assert!(app.windows().is_empty());
    assert!(!app.session_alive());
}

#[test]
fn direct_quit_discards_queued_window_requests() {
    let opened = Rc::new(Cell::new(0));
    let other_opens = opened.clone();
    let mut app = Runtime::new();
    app.on_app_command("app.open_other", |event| {
        if let Some(window) = event.window() {
            window.request_show("other").expect("queue the show");
        }
    })
    .expect("register the open command");
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
        catalog.window(WindowSpec::new("other"), move |_| {
            other_opens.set(other_opens.get() + 1);
            view! { <view /> }
        })?;
        Ok(())
    })
    .expect("declare the windows");
    let main = app.show("main").expect("open main");

    app.dispatch_app_command("app.open_other", CommandSource::Tray, Some(main))
        .expect("queue the show request");
    app.quit().expect("quit before settlement");
    assert_eq!(
        app.apply_window_requests()
            .expect("discarded requests settle cleanly"),
        0
    );
    assert!(app.windows().is_empty());
    assert_eq!(opened.get(), 0, "the queued window never mounts");
}

#[test]
fn explicit_quit_releases_a_pending_close_confirmation() {
    let close_runs = Rc::new(Cell::new(0));
    let handler_runs = close_runs.clone();
    let mut app = Runtime::new();
    let draft = app.setup(|| signal(String::new())).expect("app setup");
    app.declare_windows(move |catalog| {
        catalog.window(
            WindowSpec::new("main")
                .primary()
                .close_policy(ClosePolicy::Confirm),
            move |window| {
                let runs = handler_runs.clone();
                window
                    .on_close(move |_| {
                        runs.set(runs.get() + 1);
                        window
                            .request_show("confirm")
                            .expect("show the confirmation window");
                    })
                    .expect("register the close handler");
                view! {
                    <view>
                        <text-input value={draft} on:input={move |value: String| draft.set(value)} />
                    </view>
                }
            },
        )?;
        catalog.window(
            WindowSpec::new("confirm").confirmation_for("main"),
            |window| {
                let request = window
                    .close_request()
                    .expect("read the pending close")
                    .expect("the confirmation opens during a pending close");
                view! {
                    <view>
                        <text aria:label="draft">{request.epoch().to_string()}</text>
                    </view>
                }
            },
        )?;
        Ok(())
    })
    .expect("declare the confirmation");
    let main = app.show("main").expect("open main");
    app.request_close(main).expect("start the close epoch");
    assert_eq!(close_runs.get(), 1);
    assert_eq!(app.windows().len(), 2);

    app.quit().expect("quit the session");
    assert!(app.windows().is_empty(), "quit releases the whole group");
    assert!(!app.session_alive());
    assert_eq!(
        close_runs.get(),
        1,
        "quit does not run another close handler"
    );
}
