# Application segmentation data enters Parley through its dictionary provider

Parley 0.13 line-breaks and selects within CJK, Thai, Lao, Khmer and Myanmar text with ICU4X dictionary-based word and line segmentation. `zenui-render` enables Parley's `complex-scripts` feature, so the compiled dictionaries are always present and an application cannot substitute its own data. #279 asks the renderer to accept segmentation data together with its text assets; the spike recorded here decides the seam before the `TextAssets` shape is fixed in #284.

## Decision

An application supplies segmentation data through Parley's own `DictionaryProvider` trait. The renderer will carry an optional provider with the text assets; when it is present, Parley uses only that provider for word and line segmentation, and when it is absent the compiled `complex-scripts` data stays in use. No locale field is added and no ZenUI-owned dictionary format is defined.

The public seam is `RangedBuilder::set_dictionary_provider(Option<&dyn parley::DictionaryProvider>)`, available on the ranged, tree and style-run builders. `parley::DictionaryProvider` is a blanket trait over `icu_provider::DataProvider<SegmenterDictionaryAutoV1> + DataProvider<SegmenterDictionaryExtendedV1>`, so an application implements no ZenUI trait and depends directly on `icu_segmenter` and `icu_provider`. Parley consumes the provider when it builds its word and line segmenters during analysis, before shaping.

`crates/render/tests/segmentation.rs` proves the seam: a counting provider forwards both dictionary markers to `icu_segmenter::provider::Baked`, the layout builder receives it, and constructing a CJK layout observes at least one provider load. The test also pins the version contract: the crate's `icu_segmenter` requirement must stay compatible with Parley's, or the trait implementations no longer match.

## Considered options

- **A renderer-parsed segmentation blob**: `TextAssets` would carry bytes that the renderer deserializes into ICU4X data. Rejected: it adds `icu_provider_blob` and a public blob schema, and Parley warns that buffer deserialization happens per layout unless the application builds its own caching provider. The provider trait already gives the application that caching choice.
- **A segmentation locale field**: Parley selects dictionary data by script, not by locale; its only locale-shaped input is an internal fixed Japanese content locale for strictness options. Rejected: the field would promise control the engine does not expose.
- **A custom word list**: ICU4X dictionaries are compiled data, not user word lists. Rejected: implementing it would mean forking the segmentation path; per-application word lists remain a stage-four boundary.
- **The line-break override callback**: `set_line_break_override` adjusts line-break opportunities but not word segmentation or dictionary data. Rejected as a substitute; an application would have to reimplement segmentation.

## Consequences

When an external provider is configured, Parley never falls back to the compiled dictionaries, even for scripts the provider does not cover. A provider that covers only some complex scripts degrades the others, so the SDK text (#284) must state the coverage requirement rather than imply per-locale data.

Provider failures other than `IdentifierNotFound` panic during layout: Parley unwraps the segmenter load and documents this in the trait. Cache construction can therefore validate only that a provider was supplied, not the data behind it; #284's "unsupported segmentation" diagnostic becomes a documented application contract instead of a typed constructor error.

The application's `icu_segmenter` and `icu_provider` versions must stay compatible with Parley's (`~2.3.0` and `2.3.0` in Parley 0.13). A Parley or ICU4X upgrade that changes the markers is a breaking change for applications that pass a provider, which is why the text-asset contract uses Parley's trait instead of defining a parallel ZenUI one.

The compiled dictionaries stay pinned by the workspace lockfile; without a provider, the renderer behaves exactly as before this decision.
