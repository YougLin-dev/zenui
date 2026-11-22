#![allow(unused_braces)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use zenui::prelude::*;

type ReplyResult = Result<(), WindowError>;
type StaleReply = Rc<RefCell<Option<ReplyResult>>>;
type DoubleReply = Rc<RefCell<Option<(ReplyResult, ReplyResult)>>>;

#[derive(Clone, Copy)]
enum HandlerMode {
    ShowConfirmation,
    AllowImmediately,
    DenyImmediately,
    DoNothing,
    Panic,
    Missing,
}

struct ConfirmHarness {
    app: Runtime,
    main: WindowCtx,
    close_runs: Rc<Cell<u32>>,
    last_request: Rc<RefCell<Option<CloseRequest>>>,
    stale_request: Rc<RefCell<Option<CloseRequest>>>,
    stale_result: StaleReply,
    double_result: DoubleReply,
}

fn open(mode: HandlerMode) -> ConfirmHarness {
    let mut app = Runtime::new();
    let draft = app.setup(|| signal(String::new())).expect("app setup");
    let close_runs = Rc::new(Cell::new(0));
    let last_request: Rc<RefCell<Option<CloseRequest>>> = Rc::new(RefCell::new(None));
    let stale_request: Rc<RefCell<Option<CloseRequest>>> = Rc::new(RefCell::new(None));
    let stale_result: StaleReply = Rc::new(RefCell::new(None));
    let double_result: DoubleReply = Rc::new(RefCell::new(None));

    let main_runs = close_runs.clone();
    let main_last = last_request.clone();
    let confirm_stale = stale_request.clone();
    let confirm_stale_result = stale_result.clone();
    let confirm_double_result = double_result.clone();
    app.declare_windows(move |catalog| {
        catalog.window(
            WindowSpec::new("main")
                .primary()
                .close_policy(ClosePolicy::Confirm),
            move |window| {
                if !matches!(mode, HandlerMode::Missing) {
                    let runs = main_runs.clone();
                    let last = main_last.clone();
                    window
                        .on_close(move |request| {
                            runs.set(runs.get() + 1);
                            *last.borrow_mut() = Some(request);
                            match mode {
                                HandlerMode::ShowConfirmation => window
                                    .request_show("confirm")
                                    .expect("show the confirmation window"),
                                HandlerMode::AllowImmediately => {
                                    request.allow().expect("allow immediately");
                                }
                                HandlerMode::DenyImmediately => {
                                    request.deny().expect("deny immediately");
                                }
                                HandlerMode::Panic => panic!("the close handler failed"),
                                HandlerMode::DoNothing | HandlerMode::Missing => {}
                            }
                        })
                        .expect("register the close handler");
                }
                view! {
                    <view>
                        <text-input
                            class="focus-visible:border-accent"
                            aria:label="main-draft"
                            value={draft}
                            on:input={move |value: String| draft.set(value)}
                        />
                        <button
                            aria:label="main-action"
                            on:click={move || draft.set(String::from("acted"))}
                        >"Act"</button>
                    </view>
                }
            },
        )?;
        catalog.window(
            WindowSpec::new("confirm").confirmation_for("main"),
            move |window| {
                let request = window
                    .close_request()
                    .expect("read the pending close request")
                    .expect("the confirmation opens during a pending close");
                let note = signal(String::new());
                let stale = confirm_stale.clone();
                let stale_result = confirm_stale_result.clone();
                let double_result = confirm_double_result.clone();
                view! {
                    <view>
                        <text-input
                            aria:label="confirm-note"
                            value={note}
                            on:input={move |value: String| note.set(value)}
                        />
                        <button
                            aria:label="Allow"
                            on:click={move || {
                                request.allow().expect("allow the close");
                            }}
                        >"Allow"</button>
                        <button
                            aria:label="Deny"
                            on:click={move || {
                                request.deny().expect("deny the close");
                            }}
                        >"Deny"</button>
                        <button
                            aria:label="Escape"
                            on:click={move || {
                                window
                                    .request_close()
                                    .expect("the confirmation close request denies");
                            }}
                        >"Escape"</button>
                        <button
                            aria:label="Double allow"
                            on:click={move || {
                                let first = request.allow();
                                *double_result.borrow_mut() = Some((first, request.allow()));
                            }}
                        >"Double"</button>
                        <button
                            aria:label="Stale allow"
                            on:click={move || {
                                if let Some(stale) = *stale.borrow() {
                                    *stale_result.borrow_mut() = Some(stale.allow());
                                }
                            }}
                        >"Stale"</button>
                    </view>
                }
            },
        )?;
        Ok(())
    })
    .expect("declare the confirmation catalog");
    let main = app.show("main").expect("show main");
    ConfirmHarness {
        app,
        main,
        close_runs,
        last_request,
        stale_request,
        stale_result,
        double_result,
    }
}

impl ConfirmHarness {
    fn confirmation(&self) -> Option<WindowCtx> {
        self.app
            .windows()
            .into_iter()
            .find(|info| info.spec.key() == "confirm")
            .map(|info| info.window)
    }

    fn open_confirmation(&mut self) -> WindowCtx {
        self.app
            .request_close(self.main)
            .expect("request the close");
        self.confirmation().expect("the confirmation is open")
    }

    fn request(&self) -> CloseRequest {
        self.last_request
            .borrow()
            .expect("the handler received a request")
    }
}

#[test]
fn catalog_rejects_invalid_confirmation_bindings() {
    fn declare(
        declare: impl FnOnce(&mut WindowCatalog) -> Result<(), CatalogError>,
    ) -> Result<(), CatalogError> {
        Runtime::new().declare_windows(declare)
    }

    let error = declare(|catalog| {
        catalog.window(
            WindowSpec::new("main")
                .primary()
                .close_policy(ClosePolicy::Confirm),
            |_| view! { <view /> },
        )?;
        catalog.window(
            WindowSpec::new("confirm").confirmation_for("missing"),
            |_| {
                view! { <view /> }
            },
        )
    })
    .expect_err("the parent must exist");
    assert_eq!(error, CatalogError::UnknownConfirmationParent("confirm"));

    let error = declare(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| {
            view! { <view /> }
        })?;
        catalog.window(WindowSpec::new("confirm").confirmation_for("main"), |_| {
            view! { <view /> }
        })
    })
    .expect_err("the parent must opt into confirmation");
    assert_eq!(error, CatalogError::ConfirmationParentNotConfirm("main"));

    let error = declare(|catalog| {
        catalog.window(
            WindowSpec::new("main")
                .primary()
                .close_policy(ClosePolicy::Confirm)
                .confirmation_for("main"),
            |_| view! { <view /> },
        )
    })
    .expect_err("a window cannot confirm itself");
    assert_eq!(error, CatalogError::ConfirmationParentIsSelf("main"));

    let error = declare(|catalog| {
        catalog.window(
            WindowSpec::new("main")
                .primary()
                .close_policy(ClosePolicy::Confirm),
            |_| view! { <view /> },
        )?;
        catalog.window(WindowSpec::new("a").confirmation_for("main"), |_| {
            view! { <view /> }
        })?;
        catalog.window(WindowSpec::new("b").confirmation_for("main"), |_| {
            view! { <view /> }
        })
    })
    .expect_err("a parent has one confirmation window");
    assert_eq!(error, CatalogError::DuplicateConfirmation("main"));

    let error = declare(|catalog| {
        catalog.window(
            WindowSpec::new("main")
                .primary()
                .close_policy(ClosePolicy::Confirm),
            |_| view! { <view /> },
        )?;
        catalog.window(
            WindowSpec::new("confirm")
                .confirmation_for("main")
                .close_policy(ClosePolicy::Confirm),
            |_| view! { <view /> },
        )
    })
    .expect_err("a confirmation window cannot nest another confirmation");
    assert_eq!(error, CatalogError::ConfirmationMustBeImmediate("confirm"));
}

#[test]
fn allow_closes_the_parent_and_confirmation_as_a_group() {
    let mut harness = open(HandlerMode::ShowConfirmation);
    let confirmation = harness.open_confirmation();
    assert_eq!(harness.close_runs.get(), 1);
    assert!(harness.app.window_alive(harness.main));
    assert!(harness.app.window_visible(confirmation));
    assert!(!harness.app.window_accepts_input(harness.main));
    assert!(harness.app.window_accepts_input(confirmation));
    assert_eq!(harness.app.focused_window(), Some(confirmation));
    harness.app.click_label("Allow").expect("click Allow");
    harness
        .app
        .apply_window_requests()
        .expect("settle the reply");
    assert!(harness.app.windows().is_empty());
    assert_eq!(
        harness.app.live_signals(),
        1,
        "only the app-owned draft signal survives the group release"
    );
    assert_eq!(harness.close_runs.get(), 1);
}

#[test]
fn deny_restores_parent_input_focus_and_caret() {
    let mut harness = open(HandlerMode::ShowConfirmation);
    harness.app.focus("main-draft").expect("focus the draft");
    assert!(harness.app.set_caret("main-draft", 2));
    let input = harness
        .app
        .presentation_of(harness.main)
        .expect("main snapshot")
        .find_label("main-draft")
        .expect("draft input")
        .id();

    let confirmation = harness.open_confirmation();
    assert_eq!(
        harness.app.input("main-draft", "blocked"),
        Err(MountError::Component("target window input is suspended"))
    );
    assert_eq!(
        harness.app.click_label("main-action"),
        Err(MountError::Component("target window input is suspended"))
    );

    harness.app.click_label("Deny").expect("click Deny");
    harness
        .app
        .apply_window_requests()
        .expect("settle the deny");
    assert!(!harness.app.window_alive(confirmation));
    assert!(harness.app.window_alive(harness.main));
    assert!(harness.app.window_accepts_input(harness.main));
    assert_eq!(harness.app.focused_window(), Some(harness.main));
    let snapshot = harness.app.presentation_of(harness.main).expect("snapshot");
    assert_eq!(snapshot.focus(), Some(input), "the prior focus is restored");
    assert_eq!(
        snapshot
            .find_label("main-draft")
            .expect("draft")
            .text_value(),
        Some("")
    );
    assert_eq!(harness.app.caret("main-draft"), Some(2));
    harness
        .app
        .input("main-draft", "resumed")
        .expect("input is enabled again");
    assert_eq!(
        harness
            .app
            .presentation_of(harness.main)
            .expect("snapshot")
            .find_label("main-draft")
            .expect("draft")
            .text_value(),
        Some("resumed")
    );
    assert_eq!(harness.close_runs.get(), 1);
}

/// X routes through the same runtime close request as the native WM_CLOSE path.
#[test]
fn x_on_the_confirmation_denies_the_pending_epoch() {
    let mut harness = open(HandlerMode::ShowConfirmation);
    let confirmation = harness.open_confirmation();
    harness
        .app
        .request_close(confirmation)
        .expect("X on the confirmation denies");
    assert!(!harness.app.window_alive(confirmation));
    assert!(harness.app.window_alive(harness.main));
    assert!(harness.app.window_accepts_input(harness.main));
    assert_eq!(harness.app.focused_window(), Some(harness.main));
    assert_eq!(harness.close_runs.get(), 1);
}

#[test]
fn escape_on_the_confirmation_denies_the_pending_epoch() {
    let mut harness = open(HandlerMode::ShowConfirmation);
    let confirmation = harness.open_confirmation();
    harness
        .app
        .click_label("Escape")
        .expect("the confirmation close request denies");
    harness
        .app
        .apply_window_requests()
        .expect("settle the close request");
    assert!(!harness.app.window_alive(confirmation));
    assert!(harness.app.window_alive(harness.main));
    assert!(harness.app.window_accepts_input(harness.main));
}

#[test]
fn repeated_close_focuses_the_confirmation_without_a_new_epoch() {
    let mut harness = open(HandlerMode::ShowConfirmation);
    let confirmation = harness.open_confirmation();
    let first_epoch = harness.request().epoch();
    harness.app.show("main").expect("focus the main window");
    assert_eq!(harness.app.focused_window(), Some(harness.main));

    harness
        .app
        .request_close(harness.main)
        .expect("request the close again");
    assert_eq!(
        harness.close_runs.get(),
        1,
        "the handler runs once per epoch"
    );
    assert_eq!(harness.request().epoch(), first_epoch);
    assert_eq!(harness.app.focused_window(), Some(confirmation));
    assert_eq!(harness.app.windows().len(), 2);
}

#[test]
fn stale_replies_are_rejected_after_a_new_epoch() {
    let mut harness = open(HandlerMode::ShowConfirmation);
    let first = harness.open_confirmation();
    let first_request = harness.request();
    harness
        .app
        .click_label("Deny")
        .expect("deny the first epoch");
    harness
        .app
        .apply_window_requests()
        .expect("settle the deny");
    assert!(!harness.app.window_alive(first));
    assert!(harness.app.window_alive(harness.main));

    *harness.stale_request.borrow_mut() = Some(first_request);
    let second = harness.open_confirmation();
    assert_ne!(harness.request().epoch(), first_request.epoch());
    harness
        .app
        .click_label("Stale allow")
        .expect("click the stale handle");
    harness
        .app
        .apply_window_requests()
        .expect("the stale reply applies nothing");
    assert_eq!(
        *harness.stale_result.borrow(),
        Some(Err(WindowError::Stale))
    );
    assert!(harness.app.window_alive(harness.main));
    assert!(harness.app.window_alive(second));

    harness
        .app
        .click_label("Deny")
        .expect("deny the second epoch");
    harness
        .app
        .apply_window_requests()
        .expect("settle the deny");
    assert!(harness.app.window_alive(harness.main));
}

#[test]
fn only_the_first_reply_for_an_epoch_wins() {
    let mut harness = open(HandlerMode::ShowConfirmation);
    harness.open_confirmation();
    harness
        .app
        .click_label("Double allow")
        .expect("click Allow twice");
    let double = harness.double_result.borrow();
    let results = double.as_ref().expect("the double reply ran");
    assert_eq!(results.0, Ok(()));
    assert_eq!(results.1, Err(WindowError::Stale));
    harness
        .app
        .apply_window_requests()
        .expect("settle the first reply");
    assert!(harness.app.windows().is_empty());
}

#[test]
fn a_missing_or_silent_handler_never_allows_the_close() {
    let mut missing = open(HandlerMode::Missing);
    assert_eq!(
        missing.app.request_close(missing.main),
        Err(WindowError::Close(CloseError::MissingHandler))
    );
    assert!(missing.app.window_alive(missing.main));
    assert!(missing.app.window_accepts_input(missing.main));
    assert!(missing.confirmation().is_none());

    let mut silent = open(HandlerMode::DoNothing);
    assert_eq!(
        silent.app.request_close(silent.main),
        Err(WindowError::Close(CloseError::ConfirmationNotShown))
    );
    assert_eq!(silent.close_runs.get(), 1);
    assert!(silent.app.window_alive(silent.main));
    assert!(silent.app.window_accepts_input(silent.main));
    assert!(silent.confirmation().is_none());
    silent
        .app
        .input("main-draft", "still usable")
        .expect("the parent stays usable");
}

#[test]
fn hiding_a_pending_parent_denies_and_keeps_its_local_state() {
    let mut harness = open(HandlerMode::ShowConfirmation);
    harness.app.focus("main-draft").expect("focus the draft");
    assert!(harness.app.set_caret("main-draft", 3));
    harness.app.input("main-draft", "draft").expect("draft");
    let confirmation = harness.open_confirmation();

    harness.app.hide(harness.main).expect("hide the parent");
    assert!(!harness.app.window_alive(confirmation));
    assert!(harness.app.window_alive(harness.main));
    assert!(!harness.app.window_visible(harness.main));
    harness.app.show("main").expect("show the parent again");
    assert!(harness.app.window_accepts_input(harness.main));
    let snapshot = harness.app.presentation_of(harness.main).expect("snapshot");
    assert_eq!(
        snapshot
            .find_label("main-draft")
            .expect("draft")
            .text_value(),
        Some("draft")
    );
    assert_eq!(harness.app.caret("main-draft"), Some(5));
}

#[test]
fn hiding_the_confirmation_denies_and_restores_the_parent() {
    let mut harness = open(HandlerMode::ShowConfirmation);
    let confirmation = harness.open_confirmation();
    harness.app.hide(confirmation).expect("hide the prompt");
    assert!(!harness.app.window_alive(confirmation));
    assert!(harness.app.window_alive(harness.main));
    assert!(harness.app.window_accepts_input(harness.main));
    assert_eq!(harness.app.focused_window(), Some(harness.main));
}

#[test]
fn closing_the_parent_by_another_path_resolves_the_epoch() {
    let mut harness = open(HandlerMode::ShowConfirmation);
    harness.open_confirmation();
    harness
        .app
        .close_window(harness.main)
        .expect("close the parent unconditionally");
    assert!(harness.app.windows().is_empty());
}

#[test]
fn immediate_policy_closes_without_invoking_the_handler() {
    let mut app = Runtime::new();
    let runs = Rc::new(Cell::new(0));
    let counter = runs.clone();
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |window| {
            let counter = counter.clone();
            window
                .on_close(move |_| counter.set(counter.get() + 1))
                .expect("register the close handler");
            view! { <view /> }
        })?;
        Ok(())
    })
    .expect("catalog");
    let main = app.show("main").expect("show main");
    app.request_close(main).expect("close immediately");
    assert!(app.windows().is_empty());
    assert_eq!(runs.get(), 0, "Immediate ignores the close handler");
}

#[test]
fn a_confirm_window_without_a_declared_confirmation_reports_it() {
    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(
            WindowSpec::new("main")
                .primary()
                .close_policy(ClosePolicy::Confirm),
            |window| {
                window.on_close(|_| {}).expect("register the close handler");
                view! { <view /> }
            },
        )?;
        Ok(())
    })
    .expect("catalog");
    let main = app.show("main").expect("show main");
    assert_eq!(
        app.request_close(main),
        Err(WindowError::Close(CloseError::ConfirmationNotDeclared))
    );
    assert!(app.window_alive(main));
    assert!(app.window_accepts_input(main));
}

#[test]
fn a_handler_can_allow_or_deny_without_showing_a_confirmation() {
    let mut allow = open(HandlerMode::AllowImmediately);
    allow
        .app
        .request_close(allow.main)
        .expect("allow the close");
    assert!(allow.app.windows().is_empty());
    assert_eq!(allow.close_runs.get(), 1);
    assert!(allow.confirmation().is_none());

    let mut deny = open(HandlerMode::DenyImmediately);
    deny.app.request_close(deny.main).expect("deny the close");
    assert!(deny.app.window_alive(deny.main));
    assert!(deny.app.window_accepts_input(deny.main));
    assert!(deny.confirmation().is_none());
    assert_eq!(deny.close_runs.get(), 1);
}

#[test]
fn a_panicking_handler_leaves_the_parent_open() {
    let mut harness = open(HandlerMode::Panic);
    assert_eq!(
        harness.app.request_close(harness.main),
        Err(WindowError::Close(CloseError::HandlerPanicked))
    );
    assert_eq!(harness.close_runs.get(), 1);
    assert!(harness.app.window_alive(harness.main));
    assert!(harness.app.window_accepts_input(harness.main));
    assert!(harness.confirmation().is_none());
}

#[test]
fn a_window_registers_one_close_handler_per_activation() {
    let mut app = Runtime::new();
    let second: Rc<RefCell<Option<Result<(), WindowError>>>> = Rc::new(RefCell::new(None));
    let probe = second.clone();
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |window| {
            window.on_close(|_| {}).expect("first registration");
            *probe.borrow_mut() = Some(window.on_close(|_| {}));
            view! { <view /> }
        })?;
        Ok(())
    })
    .expect("catalog");
    app.show("main").expect("show main");
    assert!(matches!(
        second.borrow().as_ref(),
        Some(Err(WindowError::State(_)))
    ));
}

#[test]
fn a_hidden_confirm_window_is_restored_before_prompting() {
    let mut harness = open(HandlerMode::ShowConfirmation);
    harness.app.hide(harness.main).expect("hide main");
    harness
        .app
        .request_close(harness.main)
        .expect("request the close");
    assert!(
        harness.app.window_visible(harness.main),
        "a hidden Confirm window becomes visible for the close protocol"
    );
    let confirmation = harness.confirmation().expect("the prompt is visible");
    assert!(!harness.app.window_accepts_input(harness.main));
    harness.app.click_label("Deny").expect("deny the close");
    harness
        .app
        .apply_window_requests()
        .expect("settle the deny");
    assert!(harness.app.window_alive(harness.main));
    assert!(harness.app.window_visible(harness.main));
    assert!(harness.app.window_accepts_input(harness.main));
    assert!(!harness.app.window_alive(confirmation));
}

#[test]
fn a_reply_after_another_path_closed_the_parent_is_stale() {
    let mut app = Runtime::new();
    let pending: Rc<RefCell<Option<CloseRequest>>> = Rc::new(RefCell::new(None));
    let stale: Rc<RefCell<Option<CloseRequest>>> = Rc::new(RefCell::new(None));
    let stale_result: Rc<RefCell<Option<Result<(), WindowError>>>> = Rc::new(RefCell::new(None));

    let main_pending = pending.clone();
    let helper_stale = stale.clone();
    let helper_result = stale_result.clone();
    app.declare_windows(move |catalog| {
        catalog.window(
            WindowSpec::new("main")
                .primary()
                .close_policy(ClosePolicy::Confirm),
            move |window| {
                let pending = main_pending.clone();
                window
                    .on_close(move |request| {
                        *pending.borrow_mut() = Some(request);
                        window
                            .request_show("confirm")
                            .expect("show the confirmation window");
                    })
                    .expect("register the close handler");
                view! { <view /> }
            },
        )?;
        catalog.window(
            WindowSpec::new("confirm").confirmation_for("main"),
            |window| {
                window
                    .close_request()
                    .expect("read the close request")
                    .expect("a pending epoch");
                view! { <view /> }
            },
        )?;
        catalog.window(WindowSpec::new("helper"), move |_| {
            let stale = helper_stale.clone();
            let result = helper_result.clone();
            view! {
                <view>
                    <button
                        aria:label="Stale allow"
                        on:click={move || {
                            if let Some(request) = *stale.borrow() {
                                *result.borrow_mut() = Some(request.allow());
                            }
                        }}
                    >"Stale"</button>
                </view>
            }
        })?;
        Ok(())
    })
    .expect("catalog");

    let main = app.show("main").expect("show main");
    app.show("helper").expect("show the helper");
    app.request_close(main).expect("request the close");
    let request = pending.borrow().expect("the handler saw the request");
    app.close_window(main)
        .expect("close the parent by another path");
    assert_eq!(app.windows().len(), 1, "only the helper remains");
    *stale.borrow_mut() = Some(request);

    app.click_label("Stale allow")
        .expect("click the stale reply");
    app.apply_window_requests()
        .expect("the stale reply applies nothing");
    assert_eq!(
        *stale_result.borrow(),
        Some(Err(WindowError::Stale)),
        "a reply after the parent closed by another path is rejected"
    );
    assert_eq!(app.windows().len(), 1);
}
