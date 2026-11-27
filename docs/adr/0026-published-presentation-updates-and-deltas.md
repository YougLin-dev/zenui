# ADR-0026: Published presentation updates and renderer-side delta application

Consumers of `PresentationSnapshot` currently read a complete root whenever the tree revision moves, and `Scene::build` performs a full layout from that root. The presentation plan (§19) requires the runtime to publish a `PresentationUpdate { snapshot, delta }` instead, with the renderer owning an `applied_revision` cursor and falling back to a full root on any gap. This ADR records the publication, delta and cursor contract shipped for that step; persistent chunked snapshots and incremental layout/damage that consume the same keys are the remainder of the same decision.

## Decision

- **One publication unit.** `PresentationUpdate` carries the current `PresentationSnapshot` and an optional `PresentationDelta`. `delta: None` means a complete root. `Runtime::presentation_update()` publishes the active tree; `presentation_update_of(window)` publishes one visible window. `presentation_of` remains a read-only convenience for UIA, tests and queries: it neither publishes nor advances the baseline.
- **Baseline per tree.** The runtime stores the last successfully published snapshot on the tree. A delta is computed from that published root to the current root, so multiple commits in one turn accumulate into one delta; a consumer that lagged still receives a contiguous range from the runtime's last publication. Hidden windows have no publication (`Hidden`); hiding clears the baseline, so the first publication after `show` is a full root and is never guessed from the consumer's cursor.
- **Keyed nodes.** Every element and text node in the snapshot carries a stable `PresentationKey`. The delta lists changed nodes with all ancestor paths, every inserted node, every removed node, removed subtree roots for fast eviction, and the topmost `layout_roots`. Flags cover focus, appearance and row-band changes. The delta never carries old values, parent chains or new values; consumers read nodes from the published root.
- **Renderer cursor.** `zenui-render::PresentationView { snapshot, applied_revision }` is the per-tree consumer view. `apply(update)` accepts a delta only when `delta.from_revision() == applied_revision` and `delta.to_revision() == snapshot.revision()`; the scene always builds from the published root. In every other case (no delta, a skipped publication, a mismatched pair) the delta is discarded and the view replaces its snapshot from the latest root. The full rebuild is a legal fallback, not an error.
- **UIA does not consume deltas.** The accessibility publisher records `last_published_revision` next to the tree it publishes. It has no `applied_revision`: delta lacks parents, child order and old values, so UIA keeps publishing whole trees and comparing adjacent publications (ADR-0012).
- **Publisher, not renderer, computes the diff.** The renderer never diffs snapshots. Publishing a delta is the runtime's responsibility; until persistent chunked snapshots land, the runtime computes the delta by walking the published and current roots, which is O(nodes) per publication rather than O(changes).

## Rejected alternatives

- **Renderer-side diffing.** It would give every consumer its own change interpretation and a second authority for structural comparison.
- **Delta carrying old/new values.** It duplicates snapshot data, grows with attribute payloads, and lets consumers observe a stale mix of root and delta; the root is authoritative.
- **UIA consuming deltas.** UIA needs full parent/child and old-value knowledge for structure and property events; delta cannot replace adjacent-tree comparison.
- **Advancing the publication baseline on reads.** Tests and UIA read snapshots frequently without rendering; publishing on those reads would move the renderer's baseline behind deltas it never applied.

## Consequences

- The runtime retains one extra snapshot root per tree (the last publication) in addition to the build cache; both are `Arc` clones of the same immutable roots.
- Delta correctness depends on every snapshot-affecting mutation advancing the tree revision. Existing host and caret/revision tests already enforce that invariant.
- Scene construction still performs a full layout; the delta is a cache-invalidation contract for later incremental layout, damage and shaping caches. The renderer view exposes `applied_revision` and the accepted delta so those caches can attach without changing the protocol.
- Persistent chunked snapshots can replace the O(nodes) diff with path-copied change journals while keeping this public contract unchanged.
