# Host owns presentation transients; the scene resolves styles

Hover, pointer-press and input modality change with every mouse move and must not enter the committed presentation snapshot. The Windows host tracks them as presentation transients, passes them to `Scene::build_with` through `SceneEnv`, and re-hit-tests the last pointer position before every scene build so a filtered, reordered or removed node cannot keep a stale target. Buttons and checkboxes activate on release over the same enabled node; capture loss, focus loss and close clear the press.

Style resolution happens once in scene construction: intrinsic defaults, class declarations and inline `style:` declarations are merged by origin, condition rank and source order into `ResolvedStyle`, which layout, text shaping and paint share. `aria:pressed` stays a semantic toggle from the snapshot; `active:` is the host's transient press; `focus-visible:` follows the host's keyboard modality.

Application appearance is the opposite case. It is committed application state: it is reachable from `WindowCtx`, tracked as a memo dependency, and carried on the presentation snapshot so snapshot equality sees a theme change and the renderer selects the light or dark palette. It survives a theme switch together with focus, caret, selection and local text edits.

## Considered Options

- **Put hover/pressed in the snapshot**: every pointer move would change snapshot equality, forcing a runtime round-trip for input that has no model meaning and contradicting the one-way committed-state pipeline.
- **Let the renderer track hover/pressed**: the renderer would need its own hit test and event source, duplicating the host's native input path and breaking the renderer's platform-free boundary.
- **Resolve styles in the runtime**: the macro/runtime would depend on the renderer's layout and palette model; keeping declarations typed in the snapshot and resolving in the scene keeps one authority for each layer.
- **Follow the OS dark-mode setting**: appearance is application state in this milestone; automatic OS tracking is out of scope.

## Consequences

`PresentationSnapshot` stays `Eq + Send + Sync + 'static`; hover and press never appear in it, while appearance does. It is the only published presentation unit and wraps an internal `Arc<SnapshotRoot>`; cloning shares the root, and equality checks `Arc::ptr_eq` and the tree revision before falling back to content comparison, so different revisions are never deep-compared. Host transients are split in two: `SceneEnv` stays `Copy + Eq` and carries the single-node hover/press fallback, modality, scroll, text scale, high contrast and the per-window environment generation; `InteractionPaths` carries the deepest-to-root hover, press and focus paths in shared `Arc<[NodeId]>` slices. `Scene::build` keeps its two-argument convenience form with a default environment and empty paths, while `Scene::build_with(snapshot, viewport, env, paths)` and `Scene::build_cached(..., paths, cache)` are the host paths. `hover:` and `active:` conditions match any element in the respective path, so an ancestor can style a whole row without per-child declarations; the built-in hover/pressed visual still belongs to the deepest node. Scene cache keys are `(snapshot revision, env generation, interaction-path generation)`, so an environment change never advances the committed tree revision yet still invalidates scene and accessibility geometry. CI hidden-window tests cover the native message sequence and read back pixels, but a visible window remains the acceptance path for real hover, pressed, focus-visible and dark contrast (ADR-0005).


## 状态更新（2026-11）

`PresentationSnapshot` 的持久化、delta 与 UIA 发布契约由 [ADR-0026](0026-published-presentation-updates-and-deltas.md) 承接；`SceneEnv` 仍为 `Copy + Eq`，环境代际、交互路径与外观的缓存分工不变。滚动、文本缩放与高对比度不推进树 revision，只有应用外观进入快照。相关门禁见 [性能门禁](../实现/性能门禁.md) 与 [验证重点](../实现/验证重点.md)。
