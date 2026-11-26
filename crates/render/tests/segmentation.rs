//! Parley 0.13 是否接受应用提供的分词语料：把两个分词 marker 转发给
//! `icu_segmenter::provider::Baked` 的计数 provider 注入布局构建器，确认
//! 分析阶段确实查询了应用数据，而未注入时仍走编译进二进制的数据。

use std::sync::atomic::{AtomicUsize, Ordering};

use icu_provider::{DataError, DataProvider, DataRequest, DataResponse};
use icu_segmenter::provider::{Baked, SegmenterDictionaryAutoV1, SegmenterDictionaryExtendedV1};
use parley::{DictionaryProvider, FontContext, LayoutContext};

/// 记录 Parley 查询次数的 provider；数据本身来自 `Baked`。
#[derive(Default)]
struct CountingDictionary {
    loads: AtomicUsize,
}

impl DataProvider<SegmenterDictionaryAutoV1> for CountingDictionary {
    fn load(
        &self,
        req: DataRequest<'_>,
    ) -> Result<DataResponse<SegmenterDictionaryAutoV1>, DataError> {
        self.loads.fetch_add(1, Ordering::Relaxed);
        Baked.load(req)
    }
}

impl DataProvider<SegmenterDictionaryExtendedV1> for CountingDictionary {
    fn load(
        &self,
        req: DataRequest<'_>,
    ) -> Result<DataResponse<SegmenterDictionaryExtendedV1>, DataError> {
        self.loads.fetch_add(1, Ordering::Relaxed);
        Baked.load(req)
    }
}

/// 编译期确认两个 marker 的实现合起来满足 Parley 的 `DictionaryProvider`。
const _: fn() = || {
    fn assert_provider<P: DictionaryProvider>() {}
    assert_provider::<CountingDictionary>();
};

#[test]
fn an_application_provider_serves_word_and_line_segmentation() {
    const TEXT: &str = "你好世界，你好世界。";
    let provider = CountingDictionary::default();
    let mut fonts = FontContext::new();
    let mut layouts = LayoutContext::<()>::new();

    let mut builder = layouts.ranged_builder(&mut fonts, TEXT, 1.0, true);
    builder.set_dictionary_provider(Some(&provider));
    let mut layout = builder.build(TEXT);
    layout.break_all_lines(Some(40.0));
    assert!(
        layout.lines().count() > 0,
        "the injected provider still produces a layout"
    );
    let injected = provider.loads.load(Ordering::Relaxed);
    assert!(
        injected > 0,
        "Parley queried the application provider during analysis"
    );

    let builder = layouts.ranged_builder(&mut fonts, TEXT, 1.0, true);
    let _ = builder.build(TEXT);
    assert_eq!(
        provider.loads.load(Ordering::Relaxed),
        injected,
        "without an injected provider Parley uses the compiled dictionaries"
    );
}
