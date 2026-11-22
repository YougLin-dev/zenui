#![cfg(windows)]
#![allow(unused_braces)]

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, PostMessageW, TranslateMessage, WM_CHAR,
    WM_LBUTTONDOWN, WM_LBUTTONUP,
};
use zenui::prelude::*;
use zenui_windows::{Host, HostConfig};

const MAIN: &str = "main";

type Picks = Rc<RefCell<Vec<Task<FileDialogOutcome>>>>;
type Outcomes = Rc<RefCell<Vec<FileDialogOutcome>>>;
type Calls = Rc<RefCell<Vec<(isize, FileDialogOptions)>>>;

#[component]
fn PickerProbe(window: WindowCtx, picks: Picks, outcomes: Outcomes) -> MountResult<MountPlan> {
    let input = signal(String::new());
    let notice = signal(String::new());

    let pick_picks = picks.clone();
    let pick_outcomes = outcomes.clone();
    let pick = move || {
        register(
            window.request_file_dialog(options(), window.app_task_scope()),
            &pick_picks,
            &pick_outcomes,
        );
    };

    let save_picks = picks.clone();
    let save_outcomes = outcomes.clone();
    let save = move || {
        let options = FileDialogOptions::save_file()
            .titled("Save tasks")
            .filter("Text files", ["txt"])
            .with_suggested_name("tasks.txt")
            .with_default_extension(".txt");
        register(
            window.request_file_dialog(options, window.app_task_scope()),
            &save_picks,
            &save_outcomes,
        );
    };

    let folder_picks = picks.clone();
    let folder_outcomes = outcomes.clone();
    let folder = move || {
        let options = FileDialogOptions::pick_folder().titled("Pick a folder");
        register(
            window.request_file_dialog(options, window.app_task_scope()),
            &folder_picks,
            &folder_outcomes,
        );
    };

    let close_picks = picks.clone();
    let close_outcomes = outcomes.clone();
    let close_and_pick = move || {
        let _ = window.request_close();
        register(
            window.request_file_dialog(options(), window.task_scope()),
            &close_picks,
            &close_outcomes,
        );
    };

    let start_task = move || {
        let result = (|| -> Result<(), TaskError> {
            let task = window
                .app_task_scope()
                .spawn(|_| String::from("task done"))?;
            task.on_done(move |text| {
                notice.set(text);
                Ok(())
            })
        })();
        if let Err(error) = result {
            notice.set(error.to_string());
        }
    };

    view! {
        <view role="main">
            <button aria:label="Pick" on:click={move || pick()}>"Pick"</button>
            <button aria:label="Save" on:click={move || save()}>"Save"</button>
            <button aria:label="Folder" on:click={move || folder()}>"Folder"</button>
            <button aria:label="Close and pick" on:click={move || close_and_pick()}>
                "Close and pick"
            </button>
            <button aria:label="Start task" on:click={move || start_task()}>"Start task"</button>
            <text aria:label="Notice">{notice}</text>
            <text-input
                aria:label="Probe input"
                value={input}
                on:input={move |value: String| input.set(value)}
            />
        </view>
    }
}

fn register(
    result: Result<Task<FileDialogOutcome>, WindowError>,
    picks: &Picks,
    outcomes: &Outcomes,
) {
    let Ok(task) = result else {
        return;
    };
    let slot = outcomes.clone();
    if task
        .on_done(move |outcome| {
            slot.borrow_mut().push(outcome);
            Ok(())
        })
        .is_ok()
    {
        picks.borrow_mut().push(task);
    }
}

fn options() -> FileDialogOptions {
    FileDialogOptions::open_files()
        .titled("Import tasks")
        .filter("Text files", ["txt"])
}

struct Harness {
    host: Host,
    picks: Picks,
    outcomes: Outcomes,
}

fn open() -> Harness {
    let mut runtime = Runtime::new();
    let picks: Picks = Rc::new(RefCell::new(Vec::new()));
    let outcomes: Outcomes = Rc::new(RefCell::new(Vec::new()));
    let factory_picks = picks.clone();
    let factory_outcomes = outcomes.clone();
    runtime
        .declare_windows(move |catalog| {
            catalog.window(
                WindowSpec::new(MAIN).primary().sized(640, 480),
                move |window| {
                    let picks = factory_picks.clone();
                    let outcomes = factory_outcomes.clone();
                    view! { <PickerProbe window={window} picks={picks} outcomes={outcomes} /> }
                },
            )?;
            Ok(())
        })
        .expect("declare the picker catalog");
    let mut host = Host::open(runtime, HostConfig::hidden()).expect("open the hidden host");
    host.render().expect("render the primary window");
    Harness {
        host,
        picks,
        outcomes,
    }
}

fn center_label(host: &Host, label: &str) -> (f32, f32, f32) {
    let id = host
        .presentation(MAIN)
        .expect("a window snapshot")
        .find_label(label)
        .expect("a labelled element")
        .id();
    let scene = host.scene(MAIN).expect("a rendered scene");
    let rect = scene.node(id).expect("a scene element").rect();
    (
        rect.x + rect.width / 2.0,
        rect.y + rect.height / 2.0,
        scene.viewport().scale(),
    )
}

fn pointer_lparam(x: f32, y: f32, scale: f32) -> LPARAM {
    let x = (x * scale).round() as i16;
    let y = (y * scale).round() as i16;
    LPARAM((x as isize & 0xffff) | ((y as isize & 0xffff) << 16))
}

fn post_click(host: &Host, label: &str) {
    let (x, y, scale) = center_label(host, label);
    let hwnd = host.hwnd(MAIN).expect("a live window");
    for message in [WM_LBUTTONDOWN, WM_LBUTTONUP] {
        // 安全：HWND 由 Host 持有；消息只排队。
        unsafe {
            PostMessageW(Some(hwnd), message, WPARAM(0), pointer_lparam(x, y, scale))
                .expect("post the pointer message");
        }
    }
}

fn post_char(host: &Host, character: char) {
    let hwnd = host.hwnd(MAIN).expect("a live window");
    // 安全：HWND 由 Host 持有；消息只排队。
    unsafe {
        PostMessageW(Some(hwnd), WM_CHAR, WPARAM(character as usize), LPARAM(0))
            .expect("post the character");
    }
}

fn input_value(harness: &Harness) -> Option<String> {
    harness
        .host
        .presentation(MAIN)
        .expect("a window snapshot")
        .find_label("Probe input")
        .expect("the probe input")
        .text_value()
        .map(str::to_owned)
}

fn notice_text(harness: &Harness) -> String {
    harness
        .host
        .presentation(MAIN)
        .expect("a window snapshot")
        .find_label("Notice")
        .expect("the notice")
        .text()
        .to_owned()
}

fn install_recording(harness: &mut Harness, calls: &Calls, outcome: FileDialogOutcome) {
    let slot = calls.clone();
    harness
        .host
        .set_file_dialog_presenter(move |anchor: HWND, options: &FileDialogOptions| {
            slot.borrow_mut().push((anchor.0 as isize, options.clone()));
            outcome.clone()
        })
        .expect("install the presenter");
}

#[test]
fn a_presenter_receives_the_anchor_and_options_and_the_host_remains_usable() {
    let mut harness = open();
    let calls: Calls = Rc::new(RefCell::new(Vec::new()));
    install_recording(
        &mut harness,
        &calls,
        FileDialogOutcome::Picked(vec![PathBuf::from("picked.txt")]),
    );

    post_click(&harness.host, "Pick");
    assert!(!harness.host.pump().expect("click and present the picker"));

    let calls = calls.borrow();
    assert_eq!(calls.len(), 1, "the presenter is called once");
    assert_eq!(
        calls[0].0,
        harness.host.hwnd(MAIN).expect("the main HWND").0 as isize,
        "the presenter receives the exact anchor"
    );
    assert_eq!(calls[0].1.title(), "Import tasks");
    assert_eq!(calls[0].1.filters()[0].name(), "Text files");
    assert_eq!(
        calls[0].1.filters()[0]
            .extensions()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["txt"]
    );
    assert_eq!(
        harness.outcomes.borrow().as_slice(),
        [FileDialogOutcome::Picked(vec![PathBuf::from("picked.txt")])]
    );

    post_click(&harness.host, "Probe input");
    assert!(!harness.host.pump().expect("focus the input"));
    post_char(&harness.host, 'X');
    assert!(!harness.host.pump().expect("type after the dialog"));
    assert_eq!(input_value(&harness).as_deref(), Some("X"));

    harness.host.resize(MAIN, 500, 400).expect("resize");
    let frame = harness.host.read_frame(MAIN).expect("read a frame");
    assert!(frame.width() > 0 && frame.height() > 0);
}

#[test]
fn a_presenter_can_pump_nested_messages_without_holding_the_host() {
    let mut harness = open();
    post_click(&harness.host, "Probe input");
    assert!(!harness.host.pump().expect("focus the input"));

    let calls: Calls = Rc::new(RefCell::new(Vec::new()));
    let slot = calls.clone();
    harness
        .host
        .set_file_dialog_presenter(move |anchor: HWND, options: &FileDialogOptions| {
            slot.borrow_mut().push((anchor.0 as isize, options.clone()));
            // 安全：post 只排队；嵌套循环在当前 UI 线程派发回窗口过程。
            unsafe {
                PostMessageW(Some(anchor), WM_CHAR, WPARAM('X' as usize), LPARAM(0))
                    .expect("post the first character");
                PostMessageW(Some(anchor), WM_CHAR, WPARAM('Y' as usize), LPARAM(0))
                    .expect("post the second character");
                let mut message = MSG::default();
                for _ in 0..64 {
                    if PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).0 == 0 {
                        break;
                    }
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            FileDialogOutcome::Cancelled
        })
        .expect("install the nested presenter");

    post_click(&harness.host, "Pick");
    assert!(
        !harness
            .host
            .pump()
            .expect("present and pump nested messages")
    );

    assert_eq!(calls.borrow().len(), 1);
    assert_eq!(
        input_value(&harness).as_deref(),
        Some("XY"),
        "nested messages reached the host while the dialog was open"
    );
    assert_eq!(
        harness.outcomes.borrow().as_slice(),
        [FileDialogOutcome::Cancelled]
    );
}

#[test]
fn a_worker_wake_is_delivered_while_the_dialog_is_open() {
    let mut harness = open();
    post_click(&harness.host, "Start task");
    post_click(&harness.host, "Pick");

    let calls: Calls = Rc::new(RefCell::new(Vec::new()));
    let slot = calls.clone();
    harness
        .host
        .set_file_dialog_presenter(move |anchor: HWND, options: &FileDialogOptions| {
            slot.borrow_mut().push((anchor.0 as isize, options.clone()));
            std::thread::sleep(std::time::Duration::from_millis(50));
            // 安全：派发本线程队列；不得持有 Host 引用。
            unsafe {
                let mut message = MSG::default();
                for _ in 0..64 {
                    if PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).0 == 0 {
                        break;
                    }
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            FileDialogOutcome::Cancelled
        })
        .expect("install the waiting presenter");

    assert!(!harness.host.pump().expect("deliver both turns"));
    assert_eq!(notice_text(&harness), "task done");
    assert_eq!(calls.borrow().len(), 1);
    assert_eq!(
        harness.outcomes.borrow().as_slice(),
        [FileDialogOutcome::Cancelled]
    );
}

#[test]
fn two_requests_are_served_fifo_and_never_nested() {
    let mut harness = open();
    post_click(&harness.host, "Pick");
    post_click(&harness.host, "Pick");

    let calls: Calls = Rc::new(RefCell::new(Vec::new()));
    let slot = calls.clone();
    let index = Rc::new(Cell::new(0usize));
    let active = Rc::new(Cell::new(false));
    let nested = Rc::new(Cell::new(false));
    let slot_active = active.clone();
    let slot_nested = nested.clone();
    let slot_index = index.clone();
    harness
        .host
        .set_file_dialog_presenter(move |anchor: HWND, options: &FileDialogOptions| {
            if slot_active.replace(true) {
                slot_nested.set(true);
            }
            slot.borrow_mut().push((anchor.0 as isize, options.clone()));
            let current = slot_index.get();
            slot_index.set(current + 1);
            slot_active.set(false);
            FileDialogOutcome::Picked(vec![PathBuf::from(format!("picked-{current}.txt"))])
        })
        .expect("install the counting presenter");

    assert!(!harness.host.pump().expect("serve both requests"));
    assert!(!nested.get(), "dialogs are never stacked");
    assert_eq!(calls.borrow().len(), 2);
    assert_eq!(
        harness.outcomes.borrow().as_slice(),
        [
            FileDialogOutcome::Picked(vec![PathBuf::from("picked-0.txt")]),
            FileDialogOutcome::Picked(vec![PathBuf::from("picked-1.txt")]),
        ]
    );
}

#[test]
fn save_and_folder_requests_complete_unsupported_before_the_presenter() {
    let mut harness = open();
    let calls: Calls = Rc::new(RefCell::new(Vec::new()));
    install_recording(
        &mut harness,
        &calls,
        FileDialogOutcome::Picked(vec![PathBuf::from("picked.txt")]),
    );

    post_click(&harness.host, "Save");
    post_click(&harness.host, "Folder");
    assert!(!harness.host.pump().expect("complete both unsupported"));

    assert!(
        calls.borrow().is_empty(),
        "an open-file presenter is never called for save or folder"
    );
    assert_eq!(
        harness.outcomes.borrow().as_slice(),
        [
            FileDialogOutcome::Failed(FileDialogError::Unsupported),
            FileDialogOutcome::Failed(FileDialogError::Unsupported),
        ]
    );

    post_click(&harness.host, "Pick");
    assert!(
        !harness
            .host
            .pump()
            .expect("present an open request after the rejections")
    );
    assert_eq!(
        calls.borrow().len(),
        1,
        "the presenter slot survives the rejected requests"
    );
    assert_eq!(
        harness.outcomes.borrow().last(),
        Some(&FileDialogOutcome::Picked(vec![PathBuf::from(
            "picked.txt"
        )])),
        "open behavior is unchanged after save and folder are rejected"
    );
}

#[test]
fn a_hidden_host_without_a_presenter_completes_unsupported() {
    let mut harness = open();
    post_click(&harness.host, "Pick");
    assert!(!harness.host.pump().expect("complete unsupported"));
    assert_eq!(
        harness.outcomes.borrow().as_slice(),
        [FileDialogOutcome::Failed(FileDialogError::Unsupported)]
    );
}

#[test]
fn a_closed_anchor_is_not_presented() {
    let mut harness = open();
    let calls: Calls = Rc::new(RefCell::new(Vec::new()));
    install_recording(&mut harness, &calls, FileDialogOutcome::Cancelled);

    post_click(&harness.host, "Close and pick");
    assert!(
        harness.host.pump().expect("close the window"),
        "the last window closes without a dialog"
    );
    assert!(
        calls.borrow().is_empty(),
        "the traced presenter is not called"
    );
    assert!(
        harness.outcomes.borrow().is_empty(),
        "a window-owned picker is cancelled with its activation"
    );
    assert_eq!(harness.picks.borrow().len(), 1);
}
