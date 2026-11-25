# Text edit lineage classifies settlements, stale values and external resets

Every mounted text input owns a local edit lineage: an opaque origin from its mount generation and a monotonic revision advanced by every local edit. At each refresh the committed `value` is classified as the settlement of the pending edit (including a handler-normalized or truncated value), a stale settlement of an older revision that must be ignored, or an independent external reset that is applied and clears history. The app-facing `value` stays a plain `String`; the Windows host stays a message mapper; the model remains the value authority. This records the contract delivered by #280 and #281 and supersedes the deferral in [ADR-0014](0014-text-edit-history.md#revisionorigin-protocol). The visible acceptance is recorded in [stage-three 外部替换与组合验收](../实现/stage-three-外部替换与组合-验收.md).

## Decision

### Edit lineage

- Every mounted text input owns an `EditLineage`. The **origin** is the Node ID generation minted at mount. A same-key remount creates a new Node ID and therefore a new origin, a fresh revision and empty history; the old Node ID remains `StaleNode`, so a delivery from the previous mount cannot write into the new one.
- A monotonic **revision** advances once per local edit. Insert, backspace, delete-forward, whole-value replacement, composition commit, undo/redo and the host clipboard edits all dispatch through one path: the editor adopts the new text, advances the revision, records the dispatched digest in the in-flight window, and then calls `on:input` exactly once.
- The lineage keeps the last observed committed value, a **settled watermark** and a bounded window of the latest 256 dispatched edits `{revision, digest}`. The watermark guarantees a settled revision can never become pending again while an older record is still inside the window.
- The value binding contributes a **turn epoch** captured before the handler runs: for a `Signal` it is the write count, which advances even for an equal write; for a `Memo` it is the value-change revision; static and `once` bindings have no epoch.

### Classification at refresh

The input's committed `value` attribute is read at every refresh; the first refresh only initializes the observed value. For each later refresh:

- **No pending edit** (the newest revision is settled): an equal value confirms synchronization and records nothing; a value whose digest matches an older in-flight record is a stale settlement of that revision; any other value is an external reset.
- **A pending edit**: a matching digest is that edit's **settlement**, including a handler-normalized or truncated value; the committed value is recorded in history and the caret and selection clamp to it. A digest of an older revision is a **stale settlement**: the newest local text is kept, the model is not written and history is not cleared. Another value settles the edit when the handler wrote the value binding in that turn or the observed value changed in the same turn; without a binding write, an unchanged committed value is delivery lag (the editor keeps its local text) and a changed value observed in a later turn is an external reset.
- After a settlement the watermark advances past that revision; a later delivery of an older value is a stale settlement, never a rollback of the newest text.
- Settlement does not rewrite the model: a handler that normalizes, truncates or refuses still decides the committed value, and history entries store that committed value.

### External reset

A value that is not produced by the current or a previous local edit is applied: both history stacks and the in-flight window are cleared, the committed value is adopted, and the selection policy below runs. `Runtime::input` remains the explicit whole-value replacement path and keeps its history-clearing behavior.

- An empty value sets caret and anchor to 0 and removes the selection.
- A non-empty value clamps caret and anchor to the character length.
- A selection wholly inside the new value is preserved; a selection extending past the end collapses to the end.
- Offsets stay character-based and never split UTF-8: CJK and non-BMP scalars count as one character each.

### Composition

Composition start, update and cancel remain non-edits: they advance no revision, record no history and dispatch no `on:input`. A composition commit is one local edit and one history entry. An external reset during composition cancels the composition: the preedit is discarded without `on:input`, the new value is applied, the caret moves to the end and the selection is cleared. The trace records whether the reset cancelled a composition.

### Reducer trace

A bounded internal ring of 64 entries records `LocalEdit`, `Settlement { normalized }`, `StaleSettlement { current, stale }`, `ExternalReset`, `CompositionCancelledByReset` and `Remount`. It is a runtime-internal test seam with no public accessor; observable behaviour remains the value, caret, anchor, selection, history and the existing typed dispatch errors.

### Host boundary

The Windows host is a message mapper: it owns no origin, revision, second value tree or second selection model. Keyboard, IME, clipboard and UIA paths all reach the same runtime reducer. When an external reset cancels the runtime composition while the OS IME still holds a composition string, the host cancels the OS composition (`ImmNotifyIME`/`CPS_CANCEL`) before presenting the next changed frame. Without that, the next `WM_IME_COMPOSITION` update would resurrect the preedit from the cancelled composition and a later candidate confirmation would commit it.

### Known limitations

- A handler that normalizes a value through a `Memo` back to the value observed before the edit is indistinguishable from a deferred delivery, because a `Memo` revision advances only on a changed result. Equal-value `Signal` writes are not affected: the write count advances. Only a changed `Memo` value settles an edit through a `Memo` binding.
- With a plain `String` binding, an external value identical to a digest still retained in the in-flight window is classified as a stale settlement, not an external reset. `Runtime::input` is the explicit path for such a replacement.

## Considered options

- **Host-owned origin and revision**: the Windows host would need a second editor state and the classification could not be tested headlessly. Rejected; the runtime owns local edit state and the host maps messages.
- **A public `TextValue`/origin/revision type**: applications would have to adopt a new value protocol and existing `String` bindings would break. Rejected; the origin and revision are runtime-local and the app-facing value stays `String`.
- **Revision carried through the application model**: every handler would be responsible for echoing a revision, and a handler that normalizes or refuses a value could not express settlement. Rejected; the committed value at refresh is the authority.
- **Roll back to the newest committed value on every delivery**: a lagging model would erase newer typing. Rejected; the newest local text is kept until the current or a newer revision settles.
- **Clearing history on every value change**: the handler's own model write would clear history after each edit. Rejected; settlement and stale values are distinguished from external resets.
- **Unbounded in-flight window**: a long session or an imported value would retain arbitrary text. Rejected; the window keeps the latest 256 digests, and older records fall back to the documented external-reset rule.
- **A public trace API**: applications would couple to internal classification steps. Rejected; the trace is a bounded internal test seam.

## Consequences

The origin/revision protocol is delivered entirely in the headless runtime plus the existing host mapping. SDK applications keep binding `String` and keep writing the model from `on:input`; the editor no longer loses newer text when the model lags, no longer treats its own settlement as an external reset, and applies genuine external resets deterministically, including during composition. History rules from [ADR-0014](0014-text-edit-history.md) are unchanged: entries store committed values, runs and capacity limits stay as recorded, and exact caret/anchor/selection restoration is still promised only for identity handlers.

Application-selected selection policies keyed by replacement reason, detecting a same-value external reset through a plain `String` binding, cross-input or application-level undo, rich and multi-line text, CRDTs and remote cursors remain out of scope. The runtime API is headless and deterministic: the lineage tests drive settlements, normalization, stale deliveries, deferred Memo writes, remounts and composition cancellation through public runtime entry points and inspect the bounded trace.
