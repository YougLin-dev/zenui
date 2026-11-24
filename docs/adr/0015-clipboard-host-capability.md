# Clipboard is a host capability over the committed selection and edit reducer

Ctrl+C, Ctrl+X and Ctrl+V on a focused enabled text input are Windows host services. The runtime gains no clipboard API, no second editor state and no application clipboard model. Copy reads the committed selection from the presentation snapshot; cut writes first and only then asks the existing backspace reducer to delete; paste filters the clipboard text and feeds the existing insert reducer. Each operation either dispatches no `on:input` (copy) or exactly one (cut, paste).

## Decision

### Provider seam

`ClipboardProvider` is the host seam:

```rust
pub trait ClipboardProvider {
    fn read_text(&mut self) -> Result<Option<String>, ClipboardError>;
    fn write_text(&mut self, text: &str) -> Result<(), ClipboardError>;
}
```

`Host::set_clipboard_provider` installs an implementation. A visible host installs the native `CF_UNICODETEXT` provider by default; a hidden host has none, so hidden tests and CI never read or write the system clipboard. The provider has no completion callback, no async handle and no retry loop of its own. Its methods run on the UI thread during input dispatch. A provider panic is contained with `catch_unwind` and treated exactly like an error; failures are silent no-ops for the user and never become `HostError`.

### Copy and cut

Copy takes the focused input's committed value and normalized selection from the snapshot captured for that key event. It slices the value by Unicode scalars, not UTF-16 units, and writes exactly those characters. A collapsed selection, missing focus, disabled input, active composition, missing provider or failed write returns before any edit; copy never dispatches `on:input`.

Cut reuses the same write path. Only `Ok(())` from `write_text` allows the host to call `Runtime::backspace_node`; that reducer already deletes a non-empty selection, leaves the caret at the old range start, records one history entry and dispatches one `on:input`. Error, panic or a missing provider therefore leaves text, selection and caret untouched, and deleting before a successful write is structurally impossible. There is no new runtime method for cut.

### Paste

Paste reads `CF_UNICODETEXT`, filters every Unicode control character (including NUL, CR, LF and tab), and calls the existing `Runtime::insert_text_node` once. A non-empty selection is replaced; a collapsed selection inserts at the caret; the caret ends after the inserted text. A successful paste is one edit, one `on:input` and one history entry. Empty filtered text, a non-text clipboard, a missing provider, an error or a panic returns before the reducer, so the model, selection and history are unchanged. A failed paste never deletes a selection.

### Composition, modifiers and focus

The host enters the editor shortcut path only when a focused enabled text input exists and no composition is active. Composition keeps priority over Ctrl+C/X/V just as it does over Ctrl+A/Z/Y. AltGr is reported as Ctrl+Alt on Windows; the host masks the editor shortcut path when Alt is down, so AltGr-generated characters are not consumed by copy, cut or paste. The operations do not move focus. Between windows, one provider serves the OS-global clipboard, but each paste is applied to the input focused in the window that received the key.

### Windows ownership and lifetime

The native provider uses `OpenClipboard` with at most five attempts and a 10 ms pause between attempts. A guard closes the clipboard on every return path and during unwind. Read locks the `HGLOBAL`, copies UTF-16 units into a local buffer before unlocking, and closes; the system-owned handle is never freed. Write encodes UTF-16 with a terminating NUL, empties the clipboard, allocates `GMEM_MOVEABLE`, locks, copies, unlocks and calls `SetClipboardData`. On success the system owns the allocation; on `GlobalLock` or `SetClipboardData` failure the provider frees its allocation and returns an error. Clipboard access does not use the OLE apartment that the file dialogs need.

### Scope

The native provider exchanges `CF_UNICODETEXT` only. Opening/HTML/RTF/image/file-drop formats, clipboard history, delayed rendering, cross-process format negotiation, programmatic clipboard watching and rich-text paste are out of scope. Text inputs are single-line in this milestone, which is why all control characters are dropped rather than only line breaks. Applications that need another format or another behavior can inject their own provider; the core SDK does not promise a clipboard API.

## Considered options

- **Runtime or SDK clipboard API**: the headless core would depend on an OS service, tests would need the system clipboard, and the value-authority model would gain a second source of truth. Rejected; clipboard is a host capability and the runtime only sees its existing edit reducers.
- **OLE clipboard and rich formats**: adds COM apartment and format-negotiation state for formats the single-line text input cannot represent. Rejected for this milestone; the provider seam can carry a richer implementation later.
- **Deleting the selection before writing (or concurrently)**: a clipboard failure would destroy text. Rejected; write success gates deletion.
- **A host-owned second text buffer or clipboard history**: duplicates editor state and creates a second authority for value, caret and selection. Rejected; copy reads the snapshot, cut and paste use the reducers.
- **Pasting raw text including line breaks**: a single-line input cannot represent them, and silent normalization elsewhere would hide the loss. Rejected; drop all control characters before the reducer.
- **Treating AltGr as Ctrl**: breaks characters produced with AltGr layouts by consuming them as shortcuts. Rejected; Alt masks the shortcut path.
- **Asynchronous or timer-driven clipboard access**: clipboard ownership is short and UI-thread-bound; a completion turn would make a keystroke outlive its input event and complicate focus. Rejected; the bounded synchronous retry is the whole policy.
- **Per-window native clipboard objects**: the clipboard is OS-global, not per activation. Rejected; one provider per host, with I/O gated by the focused input.

## Consequences

`shortcuts::editor_shortcut` remains the only policy seam: it maps Ctrl+C/X/V to `Copy`/`Cut`/`Paste`; the host owns the Windows work. Hidden tests inject an in-memory provider and cover success, failure, missing provider, no-op states, surrogate pairs, AltGr and cross-window routing. Visible Windows acceptance is recorded in [stage-three 剪贴板验收](../实现/stage-three-剪贴板-验收.md) with Notepad interop, in-app cut/paste and failure no-ops. Cut and paste reuse [ADR-0014](0014-text-edit-history.md) history entries; clipboard operations add no history semantics of their own.
