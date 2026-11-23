# Text edit history is runtime-local and the model stays the value authority

A plain `text-input` now keeps bounded undo/redo history in the headless runtime. Each committed insert, backspace, delete-forward or composition commit records one entry. Undo and redo re-dispatch exactly one `on:input`, so the application model remains the authority for the value; the runtime never silently rewrites the editor's value behind the model. There is no new prop, no host-owned editor state and no widget layer.

## Decision

`Runtime::undo_node` and `Runtime::redo_node` take a `NodeId`; `NodeOperation::Undo` and `NodeOperation::Redo` name the operation in existing typed errors. Every mounted text input owns a `TextHistory` in its local edit state:

- An entry stores the full local state **before** and **after** the edit: committed value, caret, anchor and normalized selection. Redo restores the recorded post-edit state, not the caret position at the moment undo happened; moving the caret between an edit and its undo therefore cannot corrupt redo.
- The edit records the value **actually committed by the handler**. After the edit's refresh, the runtime reads the input's committed `value` attribute, clamps caret/anchor/selection to it, and only then records the entry. Handler-rewritten or truncated values are recorded as committed.
- Entries whose before and after committed values are equal are dropped. Such an edit is not a committed edit: it neither creates history nor clears redo.
- Insert, backspace, delete-forward, composition commit and whole-value replacement each record at most one entry. Caret/selection movement, focus changes and composition start/update/cancel record nothing and dispatch nothing. Composition commit restores the state from before composition started (`base_caret`/`base_anchor`/`base_selection`), not the temporary preedit caret.
- `Runtime::replace_text_node` is a local whole-value edit: it ignores the selection, dispatches one `on:input`, places the caret at the end of the replacement and records one entry when the committed value changes. It preserves the existing history instead of clearing it, because it represents a user edit (for example UIA `ValuePattern.SetValue`), not a model-side external replacement. A replacement identical to the current value is a silent no-op: no `on:input`, no entry.
- Undo and redo are silent `Ok(())` no-ops on an empty stack. A successful application dispatches exactly one `on:input` with the recorded value; a new committed edit clears the redo stack.
- Undo/redo during composition, on disabled inputs, non-text nodes, foreign runtimes, stale nodes, hidden windows or close-suspended windows returns the existing `DispatchError` shapes and never writes the model.

**Exact restoration is promised only for an identity handler** that stores the dispatched value unchanged. A truncating or normalizing handler keeps model authority: the undo/redo dispatch still runs exactly once and the committed value is whatever that handler stores. Tests cover both the identity round trip and truncating/normalizing handlers separately.

## External replacement

Each history records the last committed value observed by refresh. A value change seen at refresh without a local edit in flight is an external replacement and clears both stacks. A local edit marks its own settlement before dispatching, so the handler's model write is not mistaken for an external change.

`Runtime::input(label, value)` is explicitly an external whole-value replacement: it clears both stacks and still dispatches one `on:input`. A shared `Signal` edit in another window changes this input's value externally and therefore invalidates its history; the history never crosses inputs or windows.

## Composition and visibility

A composition commit is one committed edit and one history entry. Starting, updating or cancelling a composition records nothing; cancel still restores the pre-composition state without an `on:input`. Undo/redo during an active composition is rejected with `DispatchError::Rejected { reason: "text input is composing" }`.

Hidden windows keep their activation, tree and history, but dispatch is rejected with `DispatchError::Hidden` until the window is shown again. Per-input histories are independent inside one window and across windows.

## Capacity and unmount

Each input keeps at most 100 entries and at most 256 KiB of snapshot text per stack (undo and redo measured separately). Pushing evicts the oldest entries until both limits hold; a single entry larger than the byte budget is dropped rather than retained. This bounds history memory per input without a global budget or a new public configuration surface.

Unmounting an element clears its undo and redo stacks, the observed value and the local-settlement flag, so a dead element slot retains no history snapshot text or observed value. The element slot itself and its committed `value` attribute (and any composition preedit) remain until the Runtime is dropped; this ADR bounds history memory, not the existing element-table lifetime. The paths that unmount a branch (`release_nodes`, a replaced `Show` branch and removed `For` items) all clear history before disposing the Owner. Remounting creates a new Node ID and a fresh empty history; an old Node ID is `StaleNode`.

## Superseding the SDK revision/origin target

`docs/SDK/样式与输入.md` describes a future editor protocol where `value` carries an origin and revision: a matching origin only confirms synchronization, while external resets arrive under an independent revision, and selection policy is chosen per external replacement reason. That protocol is **not implemented** in this headless slice, and this ADR supersedes it as the current text-history contract:

- History is runtime-local and identified by the input's Node ID, not by an editor revision carried through the model.
- External replacement is detected by comparing the committed value at refresh; there is no origin/revision handshake.
- Exact caret/anchor/selection restoration is promised only for identity handlers; the documented "restore a specified range or collapse to the end per replacement reason" policy remains future work.
- The SDK's "clear, undo, format or remote update is an external replacement" remains true in spirit: any committed value change not caused by this input's local edit clears history.

The verification checklist's Windows-only items about origin/revision traces, consecutive edits before value delivery and external replace during composition remain target design. They must not be read as covered by this ADR. The native adapter can later add revision/origin on top of this runtime seam without changing the model-authority rule.

## Considered options

- **Host-owned or widget-owned history**: the Windows host would need a second editor state and history could not be tested headlessly. Rejected; the runtime owns local edit state and the host only maps messages.
- **History in the application model**: applications would have to opt in with a new prop and implement undo themselves, and the framework could not invalidate histories on shared-Signal changes. Rejected for plain text inputs.
- **Storing only the pre-edit snapshot and inferring redo at undo time**: caret/selection movements between edit and undo would change what redo restores. Rejected in favor of recording both sides at commit time.
- **Unbounded history**: long typing sessions and imported multi-megabyte values would retain arbitrary text. Rejected; bounded stacks evict oldest entries.
- **One history per window or application**: undo would be ambiguous between inputs and could leak text across windows. Rejected; histories are per input and per window.

## Consequences

Undo/redo do not map Ctrl+Z/Ctrl+Y in the Windows host, do not use the clipboard and do not change the demo in this milestone; those are separate host and product steps. The runtime API is headless and deterministic: tests drive edits, composition, external replacements, shared Signals, visibility, caps and unmounting through public runtime entry points, plus one runtime-internal test that inspects a dead slot to prove history text was released.
