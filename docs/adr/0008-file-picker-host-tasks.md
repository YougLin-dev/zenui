# File pickers are host tasks with explicit job ownership

> Superseded in part by [ADR-0009](0009-file-dialog-family.md): the entry point is now `WindowCtx::request_file_dialog`, and the family covers open, save and folder selection. This page remains the historical record of the open-file slice and its host-task contract.
>
> The COM initialization paragraph is superseded by [ADR-0016](0016-ole-file-drop-target.md): the shared UI-thread apartment now uses `OleInitialize` so file dialogs and OLE drag-and-drop share one initialization, still pairing every successful call with one `OleUninitialize`.

ZenUI needs a native open-file dialog and a real import in the Todo demo. A file picker is a host service: the runtime owns the request and the task identity, and the platform host presents the dialog. The anchor window decides where the dialog appears, not how long the selected work lives. `WindowCtx::pick_file(options, scope)` returns a normal `Task<FileDialogOutcome>`; the caller passes `window.task_scope()` to cancel the selection with the activation, or `window.app_task_scope()` to let an app-owned import continue after the anchor closes. The anchor never silently promotes or demotes the owner.

The `FileDialogOptions` builder owns the dialog kind, title and filters. `FileDialogOutcome` distinguishes `Picked(Vec<PathBuf>)`, `Cancelled` and `Failed(FileDialogError)`; a user cancel is a value, not an error. `WindowCtx::pick_file` accepts only `Event`/`Delivery` turns, the exact live and visible anchor, a non-suspended parent, and a scope from the same runtime (a window scope must name this anchor). Requests are queued only after every check, so a rejection leaves no half-created task.

## Host tasks

A file dialog is not a worker. `TaskRecord` carries `TaskKind::{Worker, FileDialog}`. A host task has `receiver: None`, so `settle_tasks`, `wait_for_tasks` and dead-channel detection ignore it; reusing `begin_task` would drop its sender and mark the pending picker `Panicked`. Completion stores a typed value and flips the record to `Completed`; the existing delivery machinery then runs the continuation in a `Delivery` turn with the owning owner. The backend completes through `Runtime::complete_file_dialog_request`, which checks the runtime id, kind and `Running` status, so completion is exactly-once and generation-checked even without an installed runtime context.

`Runtime::take_file_dialog_requests` drains the queue after window requests. It revalidates the exact anchor (live, visible and accepting input) and settles ineligible entries before returning them: a window-owned task is `Cancelled`, an app-owned task is `Completed` with `Failed(AnchorClosed)` so its continuation still runs. Queued requests are likewise settled when a window closes, a mount fails, or the runtime is dropped. Pending pickers therefore never keep a closed activation alive and never deliver into a reopened one.

## Windows host

The Host presents at most one native dialog at a time, FIFO, at message-loop level and only while no `&mut Host` borrow is held. It takes a request, resolves the exact anchor, moves the presenter out of the Host, calls it outside the borrow, restores the presenter, and re-borrows to complete the request, drive tasks, sync windows, focus and repaint. A panicking presenter is caught and cannot leave the Host without a presenter or unwind across the FFI boundary. Hidden hosts never open a shell dialog: with no injected presenter they complete `Unsupported`; an injected presenter still runs in hidden mode as the CI seam. The default visible presenter is the native `IFileOpenDialog`.

COM is initialized lazily on the UI thread with `CoInitializeEx(COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE)`. `S_OK` and `S_FALSE` are paired with exactly one `CoUninitialize`; `RPC_E_CHANGED_MODE` completes `Unsupported` instead of reinitializing the thread. The native mapper ORs `FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST | FOS_PATHMUSTEXIST` (plus `FOS_ALLOWMULTISELECT`) onto `GetOptions()`, keeps wide filter buffers alive through `SetFileTypes`, sets the 1-based type index, and maps `ERROR_CANCELLED` to `Cancelled`. Paths come from `SIGDN_FILESYSPATH`, `OsString::from_wide` and `CoTaskMemFree`, so long, Unicode and non-BMP paths round-trip.

## Considered Options

- **Reuse `begin_task` for host tasks**: its result channel would drop immediately and the pending picker would be reported as a panicked worker. Rejected; host tasks use a receiver-less record.
- **Complete through a request-handle method**: it would need an installed runtime context or keep a dropped runtime alive. Rejected; completion is a `Runtime` method.
- **Run the dialog on a dedicated STA thread**: the shell dialog is modal over the owner HWND, and cross-thread owner, focus, COM lifetime and Host-borrow rules add failure modes without changing the user-visible contract. Rejected for this milestone; programmatic cancellation is the follow-up.
- **Let the anchor implicitly own the job**: callers would not be able to choose whether a selection survives its window. Rejected; scope is explicit and validated.
- **Stack multiple dialogs or add a framework `.txt` default**: both hide application decisions or recurse the shell modal loop. Rejected; one dialog at a time and the application supplies filters.

## Consequences

A pending picker survives `drive_tasks` and `settle_tasks` and completes only through its backend. Dialog failures are values; the task model never panics for them. The shell dialog is modal on the UI thread: closing the last window while it is open waits for the user to dismiss it, and programmatic cancellation is not part of this slice. Save dialogs, folder pickers and their native mapping are recorded in [ADR-0009](0009-file-dialog-family.md); drag-and-drop is recorded in [ADR-0016](0016-ole-file-drop-target.md). Initial-directory/MRU policy and programmatic cancellation remain out of scope.

Hidden and injected-presenter tests exercise request routing, exactly-once completion, nested message pumping and Host usability, but they are not native acceptance. The visible Windows run is recorded in [stage-three file picker acceptance](../实现/stage-three-文件对话框-验收.md); ADR-0005's rule that hidden smoke and WARP are partial evidence still applies. The API and behavior are described in [window and task SDK](../SDK/窗口与任务.md).
