# File drops use one OLE IDropTarget per visible handled window

Windows file drops are delivered as `DropEvent` through `WindowCtx::on_drop`. The native host registers an OLE `IDropTarget` on each visible window that registered a handler, negotiates COPY-only effects for `CF_HDROP`, and queues extracted paths for the UI thread. The runtime gains no drag state machine, no second file list and no implicit task; applications decide whether a drop starts a worker, updates a model or is ignored.

## Decision

### OLE instead of WM_DROPFILES

The host uses `RegisterDragDrop` with an `IDropTarget`, not `DragAcceptFiles` / `WM_DROPFILES`. `WM_DROPFILES` delivers only a completed drop to a window that registered as a drop sink. It cannot negotiate effects during the drag, reject non-file data before the user releases the mouse, or report drag-enter, drag-over and drag-leave separately. `IDropTarget` exposes exactly those callbacks, matches the shell's own OLE drag source, and lets the host accept COPY only while the window is visible and enabled. The target is an ordinary COM object owned by the window entry; no global drop registry is involved.

### OLE initialization supersedes ADR-0008's COM paragraph

The shared UI-thread apartment now initializes with `OleInitialize`. This supersedes the `CoInitializeEx(COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE)` paragraph in [ADR-0008](0008-file-picker-host-tasks.md): `RegisterDragDrop` needs OLE services, and initializing the thread with plain `CoInitializeEx` would make the first registration fail. `OleInitialize` still establishes a single-threaded apartment. Both `S_OK` and `S_FALSE` return successfully and are paired with exactly one `OleUninitialize`; `RPC_E_CHANGED_MODE` means the thread already belongs to another apartment and maps to the drop's `DragDropError::Unsupported` (and the file dialog's `Unsupported`). The host caches initialization failure and never reinitializes the thread.

The file dialogs and drag-and-drop share this apartment. Hidden hosts create no drop target and never initialize OLE for drops; an injected file-dialog presenter remains the CI seam.

### Registration and lifetime

A window owns at most one `DropRegistration`, and only while all of these hold:

- the host is in visible mode;
- the runtime window is visible;
- the activation registered `on_drop` in its factory.

The window entry drops its registration before its native window entry destroys the HWND, so `RevokeDragDrop` always runs on a live handle. Hiding a window, closing it or reopening it revokes the old target; showing it again creates a new target with an empty queue. A queued drop from an old activation is released with that registration and can never migrate to the new one. Non-visible windows do not register, so a hidden window also cannot receive native drops through an old target.

`OleInitialize` is called before the first `RegisterDragDrop`, lazily on the UI thread. Registration failures are host errors; a thread that is already in another COM apartment is the explicit `Unsupported` case.

### Effects and data

`DragEnter` and `DragOver` return `DROPEFFECT_COPY` only when all of these hold:

- the data object offers `CF_HDROP` (`QueryGetData` succeeds);
- the source's allowed effects include `DROPEFFECT_COPY`;
- the HWND exists and is visible and enabled.

The target never offers MOVE or LINK, regardless of modifier keys. Non-file data objects, missing file lists and disabled windows return `DROPEFFECT_NONE`. `Drop` rechecks the window and the negotiated effect, then calls `GetData` for `CF_HDROP`.

Paths are extracted with `DragQueryFileW` in source order, including duplicates. The storage medium is released on every result path, including extraction failure, through `ReleaseStgMedium`. UTF-16 is converted with `OsString::from_wide`, so CJK, non-BMP characters, long paths and unpaired surrogates round-trip without replacement or normalization. Folder paths are ordinary `CF_HDROP` entries; the handler decides what a folder means. Empty or failed extraction returns `DROPEFFECT_NONE` and does not queue an event.

### Callback and delivery boundary

COM callbacks run inside the shell's drag loop. They only inspect the HWND and data object, mutate the per-target queue and, after a successful drop, post a private window message. They never call user code, the runtime or the renderer. The host drains that message on the UI thread after dispatch, calls `Runtime::drop_files` and then settles window requests and repaints.

`Runtime::drop_files` is the validation seam. It rejects a window from another runtime, a stale/closed activation, a hidden window, a window awaiting close confirmation, an empty path list and a window without a handler before user code runs. The state rejections (`Stale`, `Hidden`, `Suspended`, `Empty`, `NoHandler`) are normal outcomes and are silent on the native drain path; `WrongRuntime` indicates a host or caller error. An accepted drop runs the handler once in `Phase::Event`; the handler can write Signals and Stores, queue window requests and start window- or application-owned tasks. A panicking handler is caught as `DropError::HandlerPanicked`, leaving the runtime usable; the native host reports that failure as `HostError::Drop`.

Expected races between the native acceptance and the drain (window hidden, suspended or closed in between) are treated as rejected drops rather than fatal host errors. Genuine handler or refresh failures surface as `HostError::Drop`; a failed `PostMessage` cancels the drop and reports `E_FAIL` to OLE.

The native target checks `IsWindowEnabled`. A modal shell picker disables its owner, and the host disables a parent while a close confirmation is pending; drops over those windows return NONE. The confirmation window used by the demo has no `on_drop` handler and therefore no target. This is why no extra modal-state flag is needed in the drop path.

### DropEvent is not a host task

A file dialog is a host task: it has a `Task<FileDialogOutcome>`, an owner and a pending lifetime. A drop has none of those. `DropEvent` is a one-shot UI event with ordered paths; it is not cancellable, has no receiver and creates no task. If the handler wants work to outlive the window, it explicitly spawns with `window.app_task_scope()`; window-owned work uses `window.task_scope()`. This keeps drop delivery independent of the file-dialog task machinery and makes ownership a caller decision.

## Considered options

- **WM_DROPFILES**: simpler and older, but only reports a completed drop, offers no effect negotiation or pre-drop rejection, and cannot distinguish non-file data during the drag. Rejected.
- **One host-level IDropTarget**: OLE registration is per HWND, and a host-level target would lose the window identity and the per-window enabled/visible rules. Rejected; registration belongs to the window entry.
- **Register on every HWND unconditionally**: hidden windows and windows without handlers would accumulate targets and would need to reject every drop in user-visible ways. Rejected; registration follows visibility plus handler presence.
- **Register and revoke for every drag**: adds churn and races for no benefit; the target is inert until the shell calls it and reads current HWND state on each callback. Rejected.
- **Run the handler inside the COM callback**: would execute user code in the shell drag loop, re-enter the runtime during OLE dispatch and blur the event/delivery boundary. Rejected; callbacks queue and the host drains.
- **Accept MOVE or LINK**: ZenUI does not implement shell move/link semantics or source feedback, and a target that reports COPY while mutating files would be dishonest. Rejected; COPY only.
- **Treat folder paths as an error in the host**: the host cannot know whether an application accepts folders; the handler owns that policy. Rejected; folders are ordinary paths.
- **Convert paths through `String::from_utf16_lossy`**: replaces unpaired surrogates and changes the identity of a valid Windows path. Rejected; `OsString::from_wide` preserves UTF-16.
- **Expose drop as a Task**: implies cancellation and a pending state that a one-shot UI event does not have. Rejected; tasks are opt-in inside the handler.

## Consequences

The native host needs OLE on its UI thread; a thread already in another COM apartment cannot host native drops or file dialogs and reports `Unsupported`. Only visible windows with an `on_drop` handler can receive native drops, and the application must opt into any work beyond the event. Drop delivery preserves order, duplicates, Unicode and long paths, while non-file data and unsupported effects are rejected before the handler.

No custom drag image, drag-over highlight, drop-position hit testing, auto-scroll or programmatic drag is implemented. The target accepts over the whole client area and relies on the shell cursor/effect. Out of scope for this slice: text/URL/image drops, MOVE/LINK effects, multiple simultaneous drags and cross-monitor behavior.

Fake `IDataObject` tests cover effects, extraction, registration policy and medium lifetime; hidden tests cover the runtime validation and lifetime. They are partial evidence. The visible Windows acceptance at 100% and 150% is recorded in [stage-three file drop acceptance](../实现/stage-three-文件拖放-验收.md); the `DropEvent` API and validation rules are described in the [window and task SDK](../SDK/窗口与任务.md).
