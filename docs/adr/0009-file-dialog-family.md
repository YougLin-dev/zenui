# File dialogs are one host-task family with kind-specific fields

Opening, saving and choosing a folder are the same host service: the runtime owns the request and task identity, the platform host presents one modal shell dialog anchored to a live window, and the application owns whatever happens to the returned path. Extending `pick_file` in place would leave the entry-point name and the options type describing only open-file selection, so this milestone renames it to `WindowCtx::request_file_dialog` and keeps one `FileDialogOptions` value whose fields are interpreted by `FileDialogKind`. The rename is a deliberate pre-1.0 break; open behavior is otherwise unchanged. The host-task machinery from ADR-0008 — anchored requests, explicit `TaskScope`, typed `Picked`/`Cancelled`/`Failed` outcomes, exactly-once completion and app-owned continuations — is reused without a second platform path.

## Kind-by-field matrix

| Field | `OpenFile` | `OpenFiles` | `SaveFile` | `PickFolder` |
| --- | --- | --- | --- | --- |
| Title | yes | yes | yes | yes |
| Filters | yes | yes | yes | ignored |
| Suggested name | ignored | ignored | yes | ignored |
| Default extension | ignored | ignored | yes | ignored |
| Overwrite prompt | no | no | yes | no |
| Multiple paths | no | yes | no | no |

`allows_multiple()` is true only for `OpenFiles`. `suggested_name` is a bare file name, not an initial-directory API: path separators pass through to the shell unchanged, and invalid file-name characters are the shell's decision. A leading dot on the default extension is normalized the same way as filter extensions; an empty value means absent. The options value carries every field so the builder stays one type, but a kind ignores fields outside its row and the accessors report what the native mapper will use.

`FileDialogOutcome::Picked(Vec<PathBuf>)` is kept because open-multi already needs a list. Save and folder truthfulness is enforced by the Windows host before completion: a `Picked` with the wrong arity becomes `Failed(DialogFailed(...))` — exactly one path for open-single, save and folder, at least one for open-multi. The runtime completion API stays kind-agnostic and trusts its backend, so the validation does not add an outcome variant or a second completion path.

## Native Windows mapping

- Open single and multiple keep the ADR-0008 mapping: `IFileOpenDialog` with `FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST | FOS_PATHMUSTEXIST`, plus `FOS_ALLOWMULTISELECT` for multiple.
- Save uses a fresh `IFileSaveDialog` and adds `FOS_FORCEFILESYSTEM | FOS_OVERWRITEPROMPT | FOS_PATHMUSTEXIST`. It never adds `FOS_FILEMUSTEXIST` or `FOS_ALLOWMULTISELECT`.
- Folder selection uses `IFileOpenDialog` with `FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST`. It configures no filters, type index, suggested name or default extension.
- Flags are ORed onto `GetOptions()`. The planner asserts the bits it adds, not an assumed empty baseline: the shell may already set defaults such as `FOS_NOCHANGEDIR` or `FOS_NOREADONLYRETURN`, and those defaults are not cleared.
- Configuration order is options, title, filters with the 1-based type index, suggested file name, then default extension. The shell may update the default extension when the selected type changes; the selected filter type takes precedence when they conflict. Save and folder read one path with `GetResult`; open-multi reads `GetResults`.
- `FOS_NOTESTFILECREATE` is deliberately not set. The shell's creation test can reject an unwritable destination as a dialog failure before a path is returned. The application still handles races, disk-full and other post-dialog write failures.
- `ERROR_CANCELLED` maps to `Cancelled` for every kind; other `Show` errors map to `Failed(DialogFailed(...))`. Path extraction through `SIGDN_FILESYSPATH`, `OsString::from_wide`, `CoTaskMemFree`, the STA COM guard and the one-dialog-at-a-time host drain are unchanged.

## Ownership and evidence

The anchor decides where the dialog appears; the `TaskScope` decides how long the selection lives. `window.app_task_scope()` lets the result survive the anchor and is what the demo uses for import, save and folder actions. A window-owned request is cancelled with its activation, and the runtime settles queued requests without delivering into a closed or reopened window. The application owns the write after a save dialog returns; the host never writes application data.

Hidden hosts complete `Unsupported` without a presenter. An injected `FileDialogPresenter` still runs in hidden mode and is the CI seam; it receives the full options value and dispatches on kind. Hidden and injected-presenter tests cover routing, options, arity, cancel, failure, FIFO and host usability, but they are partial evidence. The visible Windows run is the native acceptance record; hidden smoke and WARP are never presented as native evidence.

## Considered Options

- **Separate open/save/folder option types**: more precise construction, but three parallel builders and entry points for one host service. Rejected; one builder documents the kind-by-field matrix.
- **A second outcome variant for single-path kinds**: would duplicate completion and continuation paths. Rejected; the host arity check plus injected-presenter tests enforce the shipped contract.
- **Clearing shell default flags**: risks removing useful shell behavior for no visible gain. Rejected; the planner documents and tests its add-mask.
- **Applying folder filters or a type index**: folders are not file types and the shell picker ignores the distinction. Rejected; folder planning emits none of these fields.
- **Hiding unrelated files in a save dialog**: save filters select the type and extension, not the visible file set. Rejected.
- **A framework default extension or initial directory**: both are application decisions, and `suggested_name` is not an initial-directory API. Rejected.
- **A dedicated STA dialog thread**: adds cross-thread owner, focus, COM lifetime and `Host` borrow failure modes without changing the user-visible contract. Rejected for this milestone; programmatic cancellation remains a follow-up.
- **Letting the host write the file**: couples platform code to application formats and breaks the job-owner split. Rejected.

## Consequences

One entry point now covers open, save and folder, and the public surface documents which fields each kind uses. Save and folder results are single paths by contract; a backend bug becomes a typed failure instead of an ambiguous list. The shell can reject an unwritable save destination before the application sees a path, while the application's write still reports post-dialog failures. The demo's save and folder actions use app-owned work so their results can arrive after the main window closes, and both render the shared notice in the main and Details windows. Atomic or transactional writes, folder multi-select, initial-directory/MRU policy, save-as format conversion, native dialog automation in CI and cross-platform hosts remain out of scope.
