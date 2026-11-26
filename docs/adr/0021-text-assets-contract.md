# Text assets are validated at cache construction and default to no system fallback

Scene construction currently always shapes with Parley's system font collection. An application that ships its own font has no way to register it, cannot state a fallback order, and gets no diagnostic when an asset is missing or corrupt: text silently falls back to whatever the machine provides, so measurements, wrapping and caret geometry differ between machines. #279 asks for a renderer-level text-asset contract, and #284 implements it.

## Decision

`TextAssets` carries one or more `FontAsset` blobs, a primary family name, an ordered fallback-family list, a system-fallback flag (default `false`) and an optional segmentation provider ([ADR-0020](0020-application-segmentation-data.md)). `SceneCache::with_text_assets` registers the blobs into an unshared `fontique::Collection` and validates the configuration before the cache exists:

- no font blobs is `NoFonts`;
- a zero-length blob is `MissingFont { index }`;
- bytes that do not start with a supported sfnt, OpenType or collection signature are `UnsupportedFont { index }`;
- a blob that registers no family is `NoUsableFamily { index }`;
- a primary or fallback family absent from the collection is `UnknownPrimaryFamily` or `UnknownFallbackFamily`.

The signature check separates clearly unreadable files from containers that parse without a usable family. Deeper corruption inside a valid container also surfaces as `NoUsableFamily`, because `fontique::Collection::register_fonts` reports one signal for both cases.

Each layout pushes `FontFamily::List(primary + fallbacks)` in order. With `system_fallback = false` the collection contains only the configured blobs, so covered text cannot silently use a machine family. With `true`, system families join the collection and may serve through the same list and Parley's fallback selection. Assets are immutable for the cache; changing them means constructing a new cache, not reconfiguring a live one.

`SceneCache::new()` and `Default` keep the system-font path and the collection's default family. `Scene::build` and `Scene::build_with` still create a default cache per call; applications that configure assets pass their cache to `Scene::build_cached`.

## Considered options

- **System fonts plus explicitly named families**: no blobs and a family list only. Rejected: the application cannot guarantee a family on a clean machine, and a missing family is exactly what the contract must diagnose.
- **Deferring validation to the first paint**: rejected; it turns a startup configuration error into a rendering failure after a window is visible.
- **A ZenUI font wrapper over `fontique`**: rejected; blobs and family names are the whole contract, and Parley already exposes the resulting `FontData` on each run.
- **Per-scene assets**: rejected; shaping caches are cache-scoped, and swapping fonts per scene would defeat them and silently mix metrics.
- **A renderer-managed font file path**: rejected; file IO and packaging belong to the application, which decides between `include_bytes!` and a runtime path.

## Consequences

A cache built with assets is not interchangeable with a default cache: metrics and glyphs differ, and applications must keep one cache per configured asset set. `TextRun::font()` exposes the underlying `FontData`, so tests assert which blob shaped a run instead of inferring it from geometry.

The template and release path that consume this contract, the shipped font's license and coverage, and the clean-machine and missing-asset evidence are recorded separately in #286. The segmentation-provider field carries the coverage and version contract from ADR-0020. The SDK renderer page documents the asset API; the quick-start, template README and release pages gain the concrete asset path when #286 ships it.
