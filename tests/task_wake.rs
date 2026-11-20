#![allow(unused_braces)]

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, mpsc};
use std::thread::ThreadId;
use std::time::Duration;

use zenui::prelude::*;

struct WakeProbe {
    wakes: Arc<AtomicUsize>,
    off_main: Option<Arc<AtomicBool>>,
    rx: mpsc::Receiver<()>,
}

impl WakeProbe {
    fn install(app: &mut Runtime) -> Self {
        Self::install_tracking(app, None)
    }

    /// 可选地记录回调是否运行在 `main_thread` 之外。
    fn install_tracking(app: &mut Runtime, main_thread: Option<ThreadId>) -> Self {
        let wakes = Arc::new(AtomicUsize::new(0));
        let off_main = main_thread.map(|_| Arc::new(AtomicBool::new(false)));
        let (wake_tx, rx) = mpsc::channel();
        app.set_wake_callback({
            let wakes = Arc::clone(&wakes);
            let observed = off_main.clone();
            move || {
                wakes.fetch_add(1, Ordering::SeqCst);
                if let (Some(observed), Some(main)) = (&observed, main_thread) {
                    observed.store(std::thread::current().id() != main, Ordering::SeqCst);
                }
                let _ = wake_tx.send(());
            }
        });
        Self {
            wakes,
            off_main,
            rx,
        }
    }

    fn wait(&self) {
        self.rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the worker woke the host");
    }

    fn count(&self) -> usize {
        self.wakes.load(Ordering::SeqCst)
    }

    fn ran_off_main(&self) -> bool {
        self.off_main
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::SeqCst))
    }
}

#[test]
fn a_worker_result_wakes_from_its_worker_thread_and_delivers_once() {
    let main_thread = std::thread::current().id();
    let mut app = Runtime::new();
    let probe = WakeProbe::install_tracking(&mut app, Some(main_thread));

    let task = app.spawn_task(app.task_scope(), |_| 42).expect("spawn");
    app.task_done(task, |value| {
        assert_eq!(value, 42);
        Ok(())
    })
    .expect("continuation");

    probe.wait();
    assert_eq!(probe.count(), 1, "one result wakes once");
    assert!(
        probe.ran_off_main(),
        "the callback ran on the worker thread"
    );

    let report = app.drive_tasks().expect("deliver");
    assert_eq!(report.delivered, 1);
    assert_eq!(report.discarded, 0);
    assert_eq!(app.task_status(task), Some(TaskStatus::Delivered));
    assert_eq!(
        app.drive_tasks().expect("nothing left"),
        DeliveryReport::default(),
        "the result is delivered exactly once"
    );
}

#[test]
fn the_runtime_does_not_coalesce_wakes() {
    let mut app = Runtime::new();
    let probe = WakeProbe::install(&mut app);

    let first = app.spawn_task(app.task_scope(), |_| 1).expect("first");
    let second = app.spawn_task(app.task_scope(), |_| 2).expect("second");
    app.task_done(first, |_| Ok(())).expect("first done");
    app.task_done(second, |_| Ok(())).expect("second done");

    for _ in 0..2 {
        probe.wait();
    }
    assert_eq!(probe.count(), 2);
    assert_eq!(app.drive_tasks().expect("deliver").delivered, 2);
}

#[component]
fn BlockedTask(
    window: WindowCtx,
    release: Arc<Barrier>,
    delivered: Rc<Cell<u32>>,
) -> MountResult<MountPlan> {
    let scope = window.task_scope();
    let task = scope
        .spawn(move |_| {
            release.wait();
            String::from("late")
        })
        .map_err(|_| MountError::Delivery("blocked task spawn failed"))?;
    task.on_done(move |_| {
        delivered.set(delivered.get() + 1);
        Ok(())
    })
    .map_err(|_| MountError::Delivery("blocked continuation failed"))?;

    view! {
        <view>
            <text>"blocked"</text>
        </view>
    }
}

#[test]
fn closing_the_activation_prevents_stale_wakes_and_delivery() {
    let release = Arc::new(Barrier::new(2));
    let delivered = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    let probe = WakeProbe::install(&mut app);

    let window = app
        .open_window("main", {
            let release = release.clone();
            let delivered = delivered.clone();
            move |window| {
                let release = release.clone();
                let delivered = delivered.clone();
                view! { <BlockedTask window={window} release={release} delivered={delivered} /> }
            }
        })
        .expect("open window");

    app.close_window(window).expect("close");
    release.wait();
    // wait_for_tasks joins the worker, so the post-send cancellation check has run.
    app.wait_for_tasks();
    assert_eq!(
        probe.count(),
        0,
        "a result for a closed activation does not wake the host"
    );

    let _fresh = app
        .open_window("main", |_| view! { <text>"fresh"</text> })
        .expect("reopen");
    let app_task = app.spawn_task(app.task_scope(), |_| 2).expect("app task");
    app.task_done(app_task, |_| Ok(()))
        .expect("app continuation");
    probe.wait();
    assert_eq!(probe.count(), 1, "only live results wake");

    let report = app.drive_tasks().expect("deliver");
    assert_eq!(report.discarded, 1, "the stale window result is discarded");
    assert_eq!(report.delivered, 1);
    assert_eq!(delivered.get(), 0, "the stale continuation never runs");
    assert_eq!(
        app.texts(),
        vec![String::from("fresh")],
        "a reopened activation sees none of the stale state"
    );
    assert_eq!(
        app.drive_tasks().expect("nothing pending"),
        DeliveryReport::default()
    );
    assert_eq!(delivered.get(), 0);
}

#[test]
fn a_panicking_wake_callback_does_not_lose_the_result() {
    let (wake_tx, wake_rx) = mpsc::channel();
    let delivered = Rc::new(Cell::new(String::new()));

    let mut app = Runtime::new();
    app.set_wake_callback(move || {
        let _ = wake_tx.send(());
        panic!("wake callback panicked");
    });

    let task = app
        .spawn_task(app.task_scope(), |_| String::from("survives"))
        .expect("spawn");
    app.task_done(task, {
        let delivered = delivered.clone();
        move |value| {
            delivered.set(value);
            Ok(())
        }
    })
    .expect("continuation");

    wake_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the callback fired before it panicked");
    let report = app.drive_tasks().expect("deliver");
    assert_eq!(report.delivered, 1);
    assert_eq!(delivered.take(), "survives");
    assert_eq!(app.task_status(task), Some(TaskStatus::Delivered));
}

#[test]
fn a_panicking_worker_does_not_wake_and_reports_panicked() {
    let mut app = Runtime::new();
    let probe = WakeProbe::install(&mut app);

    let task = app
        .spawn_task(app.task_scope(), |_| -> u32 { panic!("job panicked") })
        .expect("spawn");

    assert_eq!(
        app.settle_tasks().expect("settle"),
        DeliveryReport::default()
    );
    assert_eq!(app.task_status(task), Some(TaskStatus::Panicked));
    assert_eq!(probe.count(), 0, "a panic is not a deliverable result");
}
