# ADR-0011: Windowed `For` mounting over a declared row extent

ADR-0010 kept every row mounted and relied on viewport culling for paint, shaping and hit-testing. That contract cannot hold for large lists: full mounting is O(rows) in snapshot size, snapshot clone and Taffy layout on every scroll rebuild. The extended benchmark proves it. At 900×700 on the reference machine, a full list builds a 10,000-row scene in ≈294–363 ms, clones a 50,030-element snapshot in ≈11 ms and spends ≈15 ms building the snapshot itself; 5,000 rows already cost ≈126–139 ms per scroll rebuild. Scrolling at that cost is not viable, so this ADR introduces a committed visible-row window for `For`. It supersedes ADR-0010 only for For nodes that opt in; every other element keeps the full-mounting contract.

## Decision

A `For` may declare a uniform `row-height` in logical points. A `For` without `row-height` keeps mounting every row exactly as ADR-0010 describes. A `For` with `row-height` is windowed:

- The declared `row-height` is the outer height of one row slot, and the For's parent must be a zero-gap vertical container. The For inserts the mounted rows between two spacer `view` elements: the top spacer is `first_row × row-height` tall, the bottom spacer is `(total_rows − first_row − mounted_rows) × row-height` tall. Layout therefore reports the same content extent `total_rows × row-height` for every window, and the scrollbar thumb, maximum offset and reveal geometry derive from that declared extent rather than from the mounted subset.
- The snapshot contains the mounted rows and the two spacers only. `PresentationSnapshot` gains row-band metadata for each windowed For: an opaque stable band key, the top spacer's `NodeId`, the declared row height, the first mounted row index and the total row count. The snapshot stays `Eq + Send + Sync + 'static`; the row height is carried as a resolved `Length`.
- `Scene::row_bands()` reports the band geometry from the same layout pass: content top, row height, first row and total rows. The host uses it to translate its scroll offset and viewport height into a row range. This is the structural seam: **the host/renderer never filters a snapshot**; it computes `(first, count)` from scene geometry and passes it to `Runtime::set_row_window(window, band, first, count)`. The runtime is the only party that mounts or unmounts rows.

## Window updates

- A window is `(first, count)` clamped to the band's total. `Runtime::set_row_window(window, band, first, count)` applies it immediately with one refresh of that window's tree, mounts entering rows, unmounts leaving rows and resizes the spacers; it returns whether the request changed and bumps the presentation revision so the host rebuilds the snapshot once. An unchanged request is free: it neither refreshes nor changes the revision.
- Window sync adds at most one scene build to a frame: the first scene supplies the band's content top and extent, the host derives the target window, and when it differs from the runtime's current window the host applies it and rebuilds once. The band's top spacer is anchored at the list's content top regardless of the mounted range, so the second pass is exact; a third window pass is never needed. (Reveal keeps its own ADR-0010 budget of at most two rebuilds and runs before window sync.)
- Scroll within one row changes only `SceneEnv`'s offset. The mounted window is unchanged and no snapshot is rebuilt. Crossing a row boundary changes the window and rebuilds the snapshot once; the cost is O(visible rows), independent of the total. The full keyed order is cached against the row-set projection and only remapped when that projection changes, so a window update does not rescan the model.
- A windowed For mounts rows even when no host window has been reported yet: it falls back to a fixed default window from row 0. The first host pass corrects it. Headless `Runtime` users that never report a window keep the default window, not full mounting.

## Identity and lifecycle

- Rows are keyed by the `For` key. While a row stays inside the window it keeps its `NodeId`, owner, once values, caret, selection, composition and history.
- A row that leaves the window is unmounted with its owner and local state. When it re-enters it is mounted fresh: a new `NodeId`, a new owner, fresh once values and fresh local edit state; the previous `NodeId` reports `Stale`.
- Reorder, filter and removal compute the full keyed order from the row set before windowing, so duplicate-key rejection is unchanged and a mounted key can never appear twice. Filtering only changes which rows are mounted; it never deletes store data. The key selector is an identity projection: while the row-set projection is unchanged its mapped order is reused, so it must not depend on mutable row fields.
- `Scene::elements()` and `NodeStatus` are window-scoped for a windowed For: only mounted rows and the two spacers are committed elements. Rows outside the window have no scene element and no live Node ID. This is the deliberate difference from ADR-0010.

## Focus, Tab and IME

The focused row is kept mounted while it is inside the window plus one row of overscan. If a scroll would leave it more than one row outside, the host blurs it and cancels any composition before applying the new window; the candidate window is hidden by the ordinary focus-loss path. This is the frozen policy:

- scrolling the focused row out of the window with at most one row of grace keeps it mounted, and its caret and composition survive;
- scrolling further blurs it, cancels composition and commits nothing;
- Tab traverses only mounted, focusable elements in paint order; at the last mounted row Tab leaves the list for the next focusable element. Rows outside the window are not traversable until scrolling mounts them.

The runtime enforces the grace window as a backstop: when it computes a new window it expands it by at most one row on either side to keep a focused row mounted; if the focused row is still outside, the refresh clears the tree's focus instead of settling it to a mounted neighbour, so headless callers and stale hosts cannot turn a scroll into an implicit focus jump.

## Rejected alternatives

- **Renderer-side snapshot filtering.** The renderer cannot mount or unmount rows, cannot own row identity and cannot answer Node queries for unmounted rows. Culling remains a paint/hit accelerator over the committed snapshot (ADR-0010); windowing belongs to the runtime.
- **App-provided visible ranges.** Applications do not own the scroll offset or the viewport (ADR-0010); asking them to compute row ranges would create a second scroll model.
- **Per-row variable heights.** Variable extents require measured row caches and a different offset-to-index mapping. The milestone freezes uniform `row-height`; variable rows are future work.
- **Pinning the focused row at any distance.** A pinned far row cannot be laid out between spacers without mounting every intervening row, which defeats the window. The frozen policy above keeps the focus only within one row of grace.

## Consequences

- Long lists become O(visible rows) in snapshot size, layout and paint while keeping exact content extent, scrollbar and reveal geometry.
- The snapshot gains two `role="spacer"` `view` elements per windowed For, carrying inline `Height` declarations; scenes must accept zero-height elements and keep them hit-testable only outside the viewport, and tests that enumerate committed elements see the spacers.
- `Scene::elements()`, `NodeStatus`, focus traversal and IME candidate anchoring become window-scoped for windowed For nodes; tests and the UIA/scroll acceptance must assert that contract instead of the old full-mounting one.
- `scene_bench` reports snapshot build, snapshot clone, scene build, window-update cost, visible window size and element counts for 100, 1,000 and 10,000 rows; it remains a measurement, not a wall-clock assertion. The visible Windows acceptance at 100% and 150% covers a 10,000-row windowed list scrolling, editing, reordering and filtering (see `docs/实现/stage-three-窗口化-for-验收.md`).
