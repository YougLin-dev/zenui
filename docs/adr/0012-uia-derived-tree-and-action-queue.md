# ADR-0012: The UIA tree is derived per HWND and writes cross one action queue

Windows accessibility support grew across several slices: a read-only tree, Invoke/Toggle/Value/SetFocus, focus/state/structure events, a minimal `TextPattern`, and a window-root `ScrollPattern`. This ADR records the contract those slices share so later work does not introduce a second retained tree, a second scroll model or provider-thread mutation of the runtime.

## Decision

Each published window answers `WM_GETOBJECT` with its own UIA fragment root. The root and every element provider resolve the window's **current** published tree; they never keep a copy of interface state or mutate the runtime. Reading is served from immutable publications, and every write becomes a provider action on the window's single typed-command queue, stamped with the window's runtime-global `ActivationId` and drained by the UI thread.

## Per-HWND fragment roots

- A window publishes a tree only when it has a committed scene, a non-zero physical client area, and is neither hidden nor closing. The host answers only `WM_GETOBJECT` with `lParam == UiaRootObjectId`; other object identifiers continue to `DefWindowProc`. `IRawElementProviderFragmentRoot` is returned, and the provider is a server-side provider (`ProviderOptions_ServerSideProvider`).
- Every HWND has an independent root. `ZenUI Todo`, `ZenUI Details` and the close-confirmation window each answer their own `UiaRootObjectId`; there is no process-wide accessibility tree.
- The provider holds `Weak<WindowAccessibility>` and a provider key (window root or a `NodeId`). It upgrades the weak reference and reads the publication on each call. A destroyed window, a hidden window, a closing window or a zero-client-area window fails the upgrade/publication lookup with `UIA_E_ELEMENTNOTAVAILABLE`.
- `HostRawElementProvider` delegates to `UiaHostProviderFromHwnd` for the window root, so native chrome keeps the host provider. The derived tree only describes the client content.

## Tree derivation

`build_tree` runs when the host publishes a scene from a presentation snapshot. It reads the snapshot and the same `Scene` used by paint and hit-testing; it never lays out again.

- Index 0 is the window node: `ControlType.Window`, name = window title, empty AutomationId, tag/ClassName `ZenUIHostWindow`, bounds = the client area `(0, 0, width, height)` in logical pixels. Scroll never moves the window node.
- Element nodes follow snapshot order (pre-order). A node exists only while the snapshot element has a scene element; a missing scene node flattens its children into the nearest accessible ancestor. `role="spacer"` views are transparent: they create no node and their children attach to the nearest ancestor.
- Node identity is the snapshot `NodeId`. The provider-local runtime ID is allocated per NodeId and stays stable across publications; an unloaded node's local runtime ID is retired and never reassigned. The window root returns no local runtime ID and uses the host window's UIA runtime ID.
- Bounds are the translated scene rect converted to physical screen coordinates with the window client origin and the window scale. They follow scroll exactly once; paint, hit-testing, caret/selection and accessibility consume the same translated scene.
- `Windowed For` is window-scoped by ADR-0011: only mounted rows have scene elements, NodeIds and tree nodes. A row that leaves the window is removed from the tree; a row that enters is a new NodeId and a new provider-local runtime ID.

## Names, roles and states

- Name: trimmed `aria:label` when non-empty, otherwise trimmed visible text. A node with neither has an empty name, never a synthesized label from a class or memory address.
- AutomationId: the `id` attribute. ClassName: the tag (`button`, `text-input`, ...). FrameworkId: `ZenUI`.
- Control types: `button` → Button, `checkbox` → CheckBox, `text-input` → Edit, `text` → Text, `form` → Group; `view` with `role="toolbar"` → ToolBar, `role="list"` → List, `role="listitem"` → ListItem, other roles → Group; every other tag → Custom. The mapping is a pure function of the committed tag and role.
- IsEnabled combines the element's `disabled` state with the current native window enabled state (`IsWindowEnabled`). IsKeyboardFocusable is the snapshot's focusable semantic and enabled state. HasKeyboardFocus requires the snapshot focus, enabled state and the window being foreground; a background window never reports a focus it does not own.
- ToggleState comes from `checked` and `aria:pressed`; both share one state, and the Toggle pattern is exposed only when the snapshot also carries the `Change` action.
- IsOffscreen is true when the translated rect has no positive-area intersection with the viewport. Offscreen nodes stay in the tree and keep their NodeId and runtime ID until unmounted by a windowed For or removed from the snapshot.
- Edit nodes carry the committed value (preedit excluded), the document/selection/caret geometry, and whether a composition is active. Composing or disabled inputs stay readable but reject writes.

## Provider threading

- A publication is an immutable `AccessibilityTree` swapped atomically under `RwLock<Option<Arc<_>>>`. Provider methods may be called from arbitrary COM/UIA threads and only read the upgraded publication plus thread-safe Win32 queries (client origin, `IsWindowEnabled`, foreground window). They never lock the runtime, never build a snapshot and never mutate the tree.
- The provider types are `Send + Sync`; shared mutable state is limited to the publication slot and the action queue, each behind its own lock. UIA can call back into the provider synchronously while an event is being raised, so events are raised after the publication lock is released.
- Stale caches resolve to the current publication; if the tree was cleared, every property, pattern and navigation call returns `UIA_E_ELEMENTNOTAVAILABLE` instead of reading a freed publication.

## Action queue

- Every activation carries a runtime-global `ActivationId { slot, generation }`. Slots are allocated monotonically by the runtime and never reused; the generation is the catalog key's open count. `WindowCtx::runtime_id()`, `WindowCtx::activation_id()` and `NodeId::runtime_id()` expose the runtime identity to the host and to providers; `WindowCtx` itself never enters a cross-thread queue.
- Writes never run on the provider thread. `Invoke`, `Toggle`, `ReplaceValue`, `SetSelection`, `SetFocus` and `Scroll` push a `ProviderAction` onto that window's FIFO queue and post `WM_APP + 3` (`UIA_ACTION`). The post only wakes the UI thread; the order is the queue order. The queue is created with the window and stores the injected `runtime` and `ActivationId` on every request as `UiActionRequest` (Node) or `ScrollRequest` (Scroll, which carries no NodeId).
- Node and Scroll requests share one FIFO, the 1024-entry limit, the order and the clearing rules; Scroll is not a second queue. A full queue or a failed post rejects the action and reports `UIA_E_ELEMENTNOTAVAILABLE`; no partial entry remains.
- The UI thread drains the queue inside its normal message turn. Each action first matches the window's injected identity; a foreign runtime or an old activation is a race and is dropped. Node actions then revalidate against the latest presentation snapshot: the NodeId must belong to this runtime and still exist, the element must remain enabled, and the window must be visible, not close-suspended and not natively disabled. `ReplaceValue` and `SetSelection` additionally reject while the input is composing; SetFocus requires the node to be focusable and the window to be foreground; Scroll requires the window to accept input and a positive maximum offset. A rejected action is a normal race and is dropped without a host error and without a retry.
- Node actions are typed commands, not routed intents: `Invoke` → `click_node`, `Toggle` → `change_node`, `ReplaceValue` → `replace_text_node` (one `on:input`, caret at the end, one undo entry, silent when unchanged), `SetSelection` → `set_selection_node` (no `on:input`) and `SetFocus` → `focus_node`. They do not run ancestor handlers, submit defaults, menu commands or capture/bubble phases. Scroll calls the host's explicit scroll path and changes only the presentation offset; it never reads a runtime snapshot.
- Clearing a publication (hide, close, zero client area) also clears pending actions, including unexecuted Scroll requests. Hiding and closing clear the queue by value, so a provider that still holds the publication unit cannot run an action queued before the clear.

## Patterns

Pattern presence is derived from snapshot semantics, not from names or labels:

- `Invoke` requires the `Click` action; `Toggle` requires `Change` plus a toggle state; `Value` and `TextPattern` are exposed on Edit nodes, with the committed text as the value (empty when none; `Value` read-only state is derived from `Input`, disable and composition); `SetFocus` follows `IRawElementProviderFragment::SetFocus` and the focusable snapshot semantic; `ScrollPattern` only on the window root.
- Unsupported patterns return an empty `GetPatternProvider` result. Unsupported `ITextRangeProvider` methods return `UIA_E_NOTSUPPORTED`; unsupported or stale write targets return `UIA_E_ELEMENTNOTAVAILABLE`.

### Root ScrollPattern

The window root is the only scrollable UIA element. It reports `VerticallyScrollable = max_scroll_offset > 0`, `VerticalViewSize = viewport / content × 100` (clamped to 0..=100), and `VerticalScrollPercent` from the applied offset, clamped to 0..=100. With no scrollable content the pattern still exists, reports false, and writes return `UIA_E_INVALIDOPERATION`.

The horizontal surface is fixed: `HorizontallyScrollable` is false, `HorizontalScrollPercent` and `HorizontalViewSize` are `UIA_ScrollPatternNoScroll` (-1), and only `NoAmount`/`NoScroll` are accepted for the horizontal arguments. `Scroll` maps `NoAmount` to a no-op, large steps to the keyboard page step `max(1, floor(viewport_height − 24))`, and small steps to the wheel line step (`lines_per_notch × 24`, with the page-scroll sentinel and zero/disabled settings following the wheel rules). `SetScrollPercent` maps 0..100 to `round(max × percent / 100)` — nearest integer logical pixel, half away from zero — and clamps to `[0, max]`; the reported percent comes from the applied offset.

The scrollbar is a host overlay, not an accessibility element. No scrollbar NodeId, `ScrollItemPattern`, horizontal scroll container or nested scroll provider exists; all scroll writes feed the single ADR-0010 offset.

## Events

- Events are derived by comparing two adjacent publications. Identity is the provider-local runtime ID; a node present in both publications is the same element.
- Structure changes are raised at one level: `ChildAdded` on the new child, `ChildRemoved` on the parent carrying the removed child's runtime ID, `ChildrenReordered` on the parent when keyed order changes. A newly added subtree reports only its top-level addition.
- `IsEnabled`, ToggleState and Value property changes are compared only for nodes present in both publications. The window root's IsEnabled is raised by the host window provider path, not duplicated by the planner.
- Focus is raised only when the publishing window is foreground. A focus change while the window is background produces no event, and later activation does not replay it; a cleared focus has no event, and clients observe `GetFocus` as empty.
- The fixed order is structure, property, focus. Events are raised after the publication lock is released; one failed raise does not stop later events.
- Hover, press, caret blink, scroll geometry and other presentation transients do not change the tree and raise no event. There is no `TextSelectionChanged` event in this contract; clients read `TextPattern.GetSelection`.

## Hidden and suspended windows

- Hidden, closing and zero-client-area windows publish no tree and clear pending actions. Re-showing publishes again on the first committed scene.
- A close confirmation keeps the parent published but reports it disabled: the tree retains its nodes, elements and root return IsEnabled=false and IsKeyboardFocusable=false, and writes are rejected. The confirmation window has its own root.
- A natively disabled window combines its disabled state with the tree's; actions report `UIA_E_ELEMENTNOTAVAILABLE`. A window that is merely background keeps its tree and reads normally, but focus properties and focus events follow the foreground rules above.

## Deferrals

- An MSAA/`IAccessible` bridge over the derived tree is not implemented. Legacy clients see the host window provider, not ZenUI's elements, unless a UIA client maps them.
- Selection, SelectionItem, ExpandCollapse, RangeValue and grid/table patterns are not exposed. Toggle state is available even where the Toggle pattern itself is absent.
- Virtualized item patterns are not exposed for windowed `For`: only mounted rows exist in the tree. There is no placeholder element for an unmounted row.
- Horizontal scrolling, nested scroll containers, a scrollbar NodeId and scroll events are out of scope. The only scroll surface is the window-root vertical ScrollPattern.
- Menu, tray and any native-chrome accessibility beyond the host provider remain future work.
- Mixed-DPI/multi-monitor behavior is not part of the current acceptance; the recorded visible evidence is single-screen.

## Consequences

- Accessibility reads are O(1) lookups into the current publication; a scroll or model commit publishes a new immutable tree, and provider identity survives it.
- Every UIA write is asynchronous from the client's point of view: the pattern call reports that the action was queued, while the UI thread may still reject it against newer state. Clients observe the result through the tree, bounds and properties on the next publication.
- The tree can never drift from paint or hit-testing because all three consume the same scene, and the only scroll offset is ADR-0010's host value.
- Hidden smoke and unit tests cover the provider/queue contract; visible process-external UIA clients, Inspect and Narrator remain the acceptance evidence. The per-slice pages and the consolidated contract page record the exact states and their limits.
