# Scroll is host-owned presentation state over the full snapshot

Scrolling never changes what is mounted. The runtime keeps mounting every row, `PresentationSnapshot` keeps every mounted element, and `Scene` keeps one `SceneElement` per snapshot element. A scroll is one integer logical-pixel offset supplied by the host in `SceneEnv`; the scene translates the emitted geometry by that offset and reports the content extent, the applied offset and the maximum offset. The renderer never filters the snapshot down to visible rows, never remounts rows and never owns a second scroll model. True virtualization — mounting only a committed window of rows — is a different contract, reserved as ADR-0011 and not part of this milestone.

## Ownership and clamping

- The offset is an `i32` logical-pixel value on `SceneEnv`, whose default is zero; a positive offset moves content up (toward smaller y). `SceneEnv` remains `Clone + Copy + PartialEq + Eq`. A scroll-only change is an environment change, not a snapshot change: it does not touch snapshot equality, Node IDs, local edit state or the committed tree.
- The offset is per window activation and is a **presentation environment value**: durable host presentation state, distinct from ADR-0006's per-event hover, press and keyboard-modality transients. It extends `SceneEnv` without being cleared with pointer transients, and it is not a per-event Glossary "presentation transient". It is not application state and not a snapshot attribute. Overview wording that calls scroll a renderer-owned transient describes frame scheduling and application of the value, not a second stored offset: the host value carried in `SceneEnv` is authoritative, and any future renderer-side scroll animation only proposes intermediate deltas to it.
- The host asks for an offset, the scene clamps it to `[0, max]` and reports the applied offset. The host then adopts the applied offset as its stored value; every later relative wheel or key step starts from that stored, already-clamped value. This prevents a boundary over-scroll or a content shrink from swallowing the first reverse step: at the maximum, repeated down notches cannot accumulate a phantom offset, and one up notch moves immediately.

## Content extent and geometry

- All scene geometry stays in logical pixels. Content coordinates are the coordinates produced by layout at offset zero. The content extent is the synthetic Taffy layout root's border box captured before translation: `x = 0`, `y = 0`, `width = viewport width`, `height = synthetic root height`. The synthetic root has a minimum height equal to the viewport height, so the content extent is never shorter than the viewport. It includes the inter-root gap and any padding or border of the emitted top-level elements; it is not the rect of the emitted `role="main"` element, which is one child of that synthetic root. Horizontal overflow is clipped and not scrollable in this milestone.
- The maximum offset is `max(0, floor(content_height - viewport_height))`. The floor is deliberate: an integer offset must never scroll past the end and expose an empty strip below the content, so the last subpixel of a fractional content height may remain unreachable. When the content fits, the maximum is zero.
- Scene coordinates are content coordinates translated by `-applied_offset`. Every emitted `SceneElement::rect`, glyph position, caret stop, caret, selection and preedit decoration uses scene coordinates. The applied offset is baked into the scene exactly once: layout is offset-independent, and paint, hit-testing, caret, IME and accessibility consume the same translated scene rather than applying the offset again. This follows the existing single-geometry rule.
- The implementation overviews also describe a future published `GeometrySnapshot` with its own scroll-offset, transform and clip fields. For this milestone `Scene` is the single published geometry. If that snapshot is introduced later, it must choose one form: untranslated content rects plus one `applied_offset` that every consumer applies exactly once (its transform excludes scroll), or already-translated rects with no scroll field. It must never expose both a translated rect and an offset that a consumer could apply again.
- The viewport clip is the half-open rectangle `[0, width) × [0, height)` in scene coordinates. An element is visible only when its translated border box and the viewport clip both have positive area and intersect. A zero-width or zero-height viewport produces no visible elements, glyphs or caret stops. Visibility never removes the element: offscreen elements keep their translated rect, remain in `Scene::elements()` and remain reachable through `Scene::node(id)`.
- The viewport clip is authoritative for culling and hit-testing. The host presents a render target that covers exactly the logical viewport at the window scale, and the backend discards paint outside its target. A target larger than the viewport does not extend the visible area: if one is used, the backend clips scene paint to the viewport rather than revealing content outside it.

## Text, caret and decorations

Visibility culling is per shaping unit, not per glyph. A text leaf or text input shapes when its translated box intersects the viewport; a partially visible input shapes its whole display text, and the clip removes glyphs and decorations that fall outside. A fully offscreen input produces no glyph runs, no caret stops and no caret, selection or preedit decorations, while its rect stays queryable. This keeps shaping, caret stops and selection geometry derived from one layout instead of from a viewport-dependent partial shape.

Caret and decoration geometry follows the same translation as the input that owns it. `Scene::caret`, `caret_at_value`, `caret_index` and `decorations()` are all in scene coordinates. `caret_index` remains usable for a partially visible input because the host only passes points inside the viewport.

Node IDs, element count and snapshot equality are offset-invariant. Shaping and caret stops are identical for every element that is shaped at a given width, but whether a given element is shaped is intentionally viewport- and offset-dependent.

## Hit testing

Hit testing is defined only inside the viewport. A point outside the viewport clip returns `None`, even when translated content geometry extends under that point; this prevents a scrolled root or header from being hit above the window. Inside the viewport:

- candidates are scanned in reverse paint order, so the topmost element wins;
- an element is hit when its translated border box contains the point under half-open edges: `x <= px < right` and `y <= py < bottom`; zero-area elements never hit;
- partially visible elements participate with their translated, unmodified rect; a click in the visible part hits them, a click outside the viewport does not;
- containers are hit-testable. Every emitted top-level element and ordinary `view`/`form` container receives clicks in its padding and in gaps no child covers. The `role="main"` padding is part of that emitted element's box and scrolls with it;
- blank space inside a container's box hits that container. The synthetic layout root is never an element, so the inter-root gap and any trailing blank area after the last top-level element return `None`, as does every point outside the viewport clip. There are no vacated mounted rows to hit because scroll never filters the snapshot.

After a snapshot change shrinks the content, the next scene reports the smaller extent; the host adopts the re-clamped applied offset before using hit-test, caret or paint geometry. A stale scene is never used for input.

## Scope: whole-page root scrolling

The first milestone has exactly one scroll offset for the whole window root. The toolbar, header, filters and list all translate together and scroll away. A pinned toolbar or any nested scroll container is deferred: it needs a viewport-fixed layer or a per-container clip and content extent, its own offset identity and nested wheel targeting, which is a behavior change rather than a refinement of this offset. Pointer hit-testing, paint, caret and IME geometry already share the translated scene, so adding containers later must extend the same single geometry authority rather than adding paint-only translation.

## Wheel

Wheel input is converted by the host into whole logical-pixel steps; the renderer only sees the resulting `i32` offset.

- The OS mouse-wheel routing setting decides delivery. Under-pointer or hybrid routing sends the event to the ZenUI window under the cursor; if the window under the cursor is not ZenUI, that window keeps the event. Focus routing sends it to the focused ZenUI window regardless of what is under the cursor; if no ZenUI window has focus, the event is dropped. A ZenUI window only scrolls for wheel input it actually received.
- Each activation keeps a signed `i64` fixed-point wheel accumulator and a signed `i64` anchor, private to the host. `lines_per_notch` is read once per activation (default 3). For a line-scrolling setting, one notch (`WHEEL_DELTA`, 120 units) is `lines_per_notch × 24` logical pixels; for the OS page-scroll sentinel (`WHEEL_PAGESCROLL`), one notch is one `PageDown`/`PageUp` step; a setting of zero disables wheel movement and resets the accumulator. On each event, add `delta_units × notch_pixels` to the accumulator, take `pixels = accumulator / 120` (truncating toward zero), compute `delta = pixels - anchor`, request `applied_offset - delta`, and set the anchor to `pixels`. Positive wheel units (forward, away from the user) therefore decrease the offset. The sub-pixel residue stays in the accumulator; truncating division makes sub-notch distances accumulate symmetrically in either direction.
- The accumulator and anchor reset to zero whenever the scene clamps a wheel request, and whenever the offset changes outside wheel input (key scroll, reveal, resize, snapshot or content re-clamp, hide/show), so unapplied wheel distance is discarded rather than banked and the first reverse notch moves immediately. They are discarded on close/reopen, never shared between windows, and reset when `lines_per_notch` changes.
- Platforms that report pixel-precision wheel deltas convert them to logical pixels in a separate signed `i64` fixed-point buffer on the same activation with the same delta-and-reset rule; the two accumulator units are never mixed.
- At an offset boundary the clamped wheel distance is discarded and the event is consumed; scroll does not chain to a parent container and does not overscroll.

## Keyboard

Scroll commands are presentation actions: they start no runtime transaction, refresh no presentation snapshot and change no focus, caret or selection.

- `PageDown`/`PageUp` scroll by `max(1, floor(viewport_height - 24.0) as i32)` logical pixels, keeping one 24-pixel overlap line. `Home`/`End` go to zero and the maximum.
- Focus traversal and focused-control key handling win. Tab/Shift+Tab and arrow-key focus traversal keep their documented behavior; a focused editable text input consumes arrows, Home/End, Backspace, Delete and Ctrl+A for editing. `PageDown`/`PageUp` scroll the page unless an IME composition or the focused control consumes them, and `Home`/`End` scroll only when no editable text input has focus.
- Each key repeat is an independent step. When the scene reports a zero maximum, scroll keys are not consumed. Horizontal keys, horizontal wheel and horizontal scrolling are out of scope.

## IME and focus reveal

- An active IME composition is never committed or cancelled by scrolling. The candidate window is positioned from the translated caret or preedit geometry after every scene rebuild; for a multi-line preedit its anchor is the composition-start caret rect, clamped into the window work area.
- Candidate visibility follows the translated caret/preedit rect, not the composing input's whole box. If the composing input is fully offscreen its caret/preedit decoration is absent; absence counts as not intersecting and hides the candidate window. The same rule applies after a scroll-induced or snapshot-induced re-clamp, so the host never positions the candidate at a negative or off-client point left over from an earlier scene. The exact Win32 hide call is verified by visible IME acceptance.
- Focus and caret reveal are host policy with at most two passes. Pass one locates the focused element: it uses the caret rect when caret geometry exists, otherwise the focused element's translated border box. If that rect is above the viewport it adds `floor(rect.top)` to the stored applied offset; if it is below, it adds `ceil(rect.bottom - viewport_height)`; if the rect is taller than the viewport and both hold, the top wins. It clamps once, rebuilds and adopts the result. Pass two runs only when pass one used the border box and the rebuild produced caret geometry, repeating the same rule for the caret. There is never a third pass, and any residual outside the viewport is accepted.
- Reveal is vertical only; a horizontally clipped caret stays horizontally clipped because horizontal scrolling is out of scope. A pointer click normally lands inside the viewport and needs no reveal, but caret placement after the click runs the same check, so a click in the visible slice of a partially visible input still reveals a caret that lands outside. Explicit scrolling never moves focus or the caret and never triggers reveal, including during composition.

## Rebuild and input ordering

A scroll, resize, snapshot change or focus change that alters the offset rebuilds and stores the scene for the new `SceneEnv` before the next pointer or keyboard input is dispatched. Paints may be coalesced or deferred, but input is never hit-tested against a scene built for an older offset.

After a rebuild, the host re-hit-tests the last pointer position against the new translated scene and, if the hovered node changed, rebuilds once with the corrected hovered value. Pointer press is capture-based and is not re-evaluated by scrolling; it still activates only on release over the same node.

## Offset lifetime

The offset is per window activation and lives in the host.

- Hide keeps the activation, focus, caret and local edit state; it also keeps the stored applied offset. Show restores it after clamping to the current content.
- Close releases the activation and its offset. Reopening the same catalog key creates a new activation at offset zero.
- Window resize, surface recreation, render-target rebuild, DPI change and simulated device loss preserve the logical offset; the scene is rebuilt and re-clamped, and the logical offset is not rescaled by Window scale. A real device-loss gap remains tracked by #63.
- A snapshot change (insert, remove, filter, reorder) preserves the stored applied offset and re-clamps it. This milestone has no scroll anchoring or follow-the-end behavior: content that shrinks and later grows leaves the offset at the last applied value, not at the old or new maximum. Keeping a particular row at the same viewport position is a later contract.
- A scroll-only update rebuilds the scene from the same snapshot with a new `SceneEnv`; no transaction runs and `presentation()` returns the same snapshot.

## Scrollbar ownership

No scrollbar is drawn in the first milestone. When one is added, the default is a host/renderer overlay over the scroll root, not a snapshot element: it reflects the applied offset and content extent and adds no NodeId. Accessibility is the platform scroll pattern (UIA `ScrollPattern` on the scrollable window/root), whose `SetScrollPercent` and `Scroll` actions feed the same host offset.

If a later milestone needs a focusable, styleable scrollbar element, it must be a real committed element allocated by the runtime like any other NodeId, placed in a viewport-fixed layer outside the translated root — the same deferred layer a pinned toolbar needs — so the root offset never scrolls it away. A renderer-invented synthetic NodeId is rejected. In every case Node IDs stay stable while scrolling, and the offset is never encoded as a snapshot attribute.

## Evidence

Headless scene tests prove translation, clamping, extent reporting, culling, hit-testing, caret/decoration geometry, multi-root gaps, zero-size viewports, stable Node IDs and unchanged offset-zero geometry. Hidden WARP smoke can show that a translated scene paints and reads back offscreen. Both are partial evidence.

Acceptance is a visible Windows 11 run on a single screen at 100% and 150% scaling: real wheel/trackpad notches including high-resolution accumulation, boundary clamping and reversal, keyboard paging, IME candidate alignment while scrolled and hiding when the caret leaves the viewport, focus/caret reveal including tabbing to an offscreen input, hover refresh after scroll, hide/show, close/reopen, and simulated device loss. Mixed-DPI multi-monitor behavior and real hardware device loss remain outside this ADR and stay with #63.

Offset-zero regression evidence compares a scene built with the default environment against the same snapshot and viewport through the same code path; it is not a portable golden of Parley glyph IDs or font-derived heights. Maximum-offset assertions are derived from the scene-reported extent rather than hard-coded font measurements.

## Considered Options

- **A floating-point offset (`f32`)**: fractional scroll would make the offset part of frame equality, produce platform-dependent accumulation and make "every rect moved by exactly `-o`" unverifiable. The host converts wheel units to integer pixels instead. Rejected.
- **Renderer-side snapshot filtering ("keep only visible rows")**: it changes element count and Node IDs with the scroll position, breaks snapshot/scene correspondence and hides rows from focus, IME and hit-testing. Rejected; culling only skips shaping and paint, never emission.
- **Application-owned scroll state (a `Signal`/`Store` value) in the first milestone**: every wheel notch would become a model transaction, mark reactive dependents dirty and couple presentation cadence to business state. A future renderer-driven scroll animation may propose deltas, but the authoritative value is not application state. Rejected.
- **Per-container or nested offsets now**: the first milestone deliberately freezes entire-root scrolling; nested clips and offsets need separate identity and hit-testing rules. Deferred.
- **Paint-only translation**: hit-testing, caret and IME geometry would disagree with what is drawn, violating the single-geometry rule. Rejected.
- **A scrollbar NodeId or an offset snapshot attribute**: both would invent a second identity/state path before the accessibility contract exists. A committed scrollbar is deferred to the viewport-fixed layer; the offset stays out of the snapshot.
- **Resetting the offset on hide or device loss**: hide is not close, and a recreated surface is not a new activation. Rejected; only close/reopen resets.
- **Scrolling by re-layout with negative margins or a translated Taffy root**: layout must stay independent of the scroll position, otherwise every scroll step remeasures text and changes layout cache identity. The offset is applied after layout. Rejected.
- **Keeping the requested offset instead of the applied one**: boundary notches would accumulate a phantom offset and the first reverse step would appear dead. Rejected; the host adopts the scene's applied value.
- **Scroll anchoring or follow-the-end in this milestone**: both require row identity and content-change policy beyond translation. Deferred.

## Consequences

`SceneEnv` stays small and `Copy`; scroll-only changes do not clone or mutate the snapshot. `Scene` reports the content extent, applied offset and maximum offset so the host can clamp, drive reveal and later paint a scrollbar. Node IDs, element count and snapshot equality are identical at every offset.

Layout still runs over the full mounted tree on every scene build. The viewport culling introduced for long imported lists skips shaping, glyph runs, caret stops and paint for offscreen elements, but it does not skip Taffy layout; a 5000-row list therefore remains O(rows) per scroll rebuild until ADR-0011 defines a committed window. The scene benchmark is extended with scroll rows at 100, 500, 1000 and 5000 rows and reports build time plus element and visible counts; it remains a measurement, not a wall-clock assertion.
