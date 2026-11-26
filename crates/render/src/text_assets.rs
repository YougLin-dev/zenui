//! 应用提供的文本资产：固定字体、主家族、回退链、系统回退与分词语料。
//!
//! 资产在 [`crate::SceneCache::with_text_assets`] 构造时注册并校验；错误不会
//! 推迟到绘制。没有配置资产时，缓存沿用系统字体集合（[`crate::SceneCache::new`]）。

use std::borrow::Cow;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

use parley::fontique::{Blob, Collection, CollectionOptions};
use parley::{DictionaryProvider, FontContext, FontFamilyName};

/// 随应用分发的一份字体字节。
///
/// 字节原样保存在内存中；同一份 [`FontAsset`] 克隆后共享同一份数据。
#[derive(Clone, Debug)]
pub struct FontAsset {
    data: Blob<u8>,
}

impl FontAsset {
    /// 从字体文件字节构造资产；解析与注册推迟到缓存构造。
    #[must_use]
    pub fn new(data: impl AsRef<[u8]>) -> Self {
        Self {
            data: Blob::new(Arc::new(data.as_ref().to_vec())),
        }
    }
}

/// 一次缓存构造使用的字体与分词语料。
///
/// 主家族与有序回退家族构成传给 Parley 的家族列表；`system_fallback` 默认关闭，
/// 此时集合只含配置的字体，已覆盖的文本不会回退到系统字体。分词语料通过
/// Parley 的 [`DictionaryProvider`] 提供；配置后编译进二进制的 ICU4X 词典不再
/// 参与分词，因此 provider 必须覆盖应用用到的复杂脚本（[ADR-0020]）。
///
/// [ADR-0020]: https://github.com/ZenUIOrg/zenui/blob/main/docs/adr/0020-application-segmentation-data.md
#[derive(Clone)]
pub struct TextAssets {
    fonts: Vec<FontAsset>,
    primary: String,
    fallbacks: Vec<String>,
    system_fallback: bool,
    dictionary: Option<Arc<dyn DictionaryProvider>>,
}

impl fmt::Debug for TextAssets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextAssets")
            .field("fonts", &self.fonts)
            .field("primary", &self.primary)
            .field("fallbacks", &self.fallbacks)
            .field("system_fallback", &self.system_fallback)
            .field("dictionary", &self.dictionary.is_some())
            .finish()
    }
}

impl TextAssets {
    /// 以主家族名开始构造；至少还需要一个 [`Self::with_font`]。
    #[must_use]
    pub fn new(primary: impl Into<String>) -> Self {
        Self {
            fonts: Vec::new(),
            primary: primary.into(),
            fallbacks: Vec::new(),
            system_fallback: false,
            dictionary: None,
        }
    }

    /// 追加一份字体资产；注册顺序不影响家族选择。
    #[must_use]
    pub fn with_font(mut self, font: FontAsset) -> Self {
        self.fonts.push(font);
        self
    }

    /// 追加一个回退家族；按调用顺序排在主家族之后。
    #[must_use]
    pub fn with_fallback_family(mut self, family: impl Into<String>) -> Self {
        self.fallbacks.push(family.into());
        self
    }

    /// 是否允许回退到系统字体；默认关闭。
    #[must_use]
    pub fn with_system_fallback(mut self, enabled: bool) -> Self {
        self.system_fallback = enabled;
        self
    }

    /// 用应用提供的分词语料替换编译进二进制的 ICU4X 词典。
    ///
    /// provider 必须同时实现自动与扩展两个词典 marker；它的版本要与 Parley 的
    /// `icu_segmenter` 兼容。除 `IdentifierNotFound` 外的加载错误会在布局期间
    /// panic，因此覆盖应用用到的复杂脚本是调用方的责任（[ADR-0020]）。
    ///
    /// [ADR-0020]: https://github.com/ZenUIOrg/zenui/blob/main/docs/adr/0020-application-segmentation-data.md
    #[must_use]
    pub fn with_dictionary_provider(mut self, provider: Arc<dyn DictionaryProvider>) -> Self {
        self.dictionary = Some(provider);
        self
    }
}

/// 文本资产在缓存构造期的显式失败。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextAssetError {
    /// 没有提供任何字体资产。
    NoFonts,
    /// 第 `index` 份资产没有字节。
    MissingFont {
        /// 资产在构造 [`TextAssets`] 时的顺序。
        index: usize,
    },
    /// 第 `index` 份资产不是受支持的 sfnt、OpenType 或字体集合容器。
    UnsupportedFont {
        /// 资产在构造 [`TextAssets`] 时的顺序。
        index: usize,
    },
    /// 第 `index` 份资产没有注册出任何可用家族；容器可能损坏或缺少名称表。
    NoUsableFamily {
        /// 资产在构造 [`TextAssets`] 时的顺序。
        index: usize,
    },
    /// 主家族没有出现在已注册的家族里。
    UnknownPrimaryFamily {
        /// 配置的主家族名。
        family: String,
    },
    /// 回退家族没有出现在已注册的家族里。
    UnknownFallbackFamily {
        /// 配置的回退家族名。
        family: String,
    },
}

impl fmt::Display for TextAssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoFonts => f.write_str("text assets configure no font"),
            Self::MissingFont { index } => write!(f, "font asset {index} has no bytes"),
            Self::UnsupportedFont { index } => {
                write!(
                    f,
                    "font asset {index} is not a supported sfnt, OpenType or collection file"
                )
            }
            Self::NoUsableFamily { index } => {
                write!(f, "font asset {index} exposes no usable font family")
            }
            Self::UnknownPrimaryFamily { family } => {
                write!(f, "the primary font family {family:?} is not registered")
            }
            Self::UnknownFallbackFamily { family } => {
                write!(f, "the fallback font family {family:?} is not registered")
            }
        }
    }
}

impl Error for TextAssetError {}

/// 资产构造出的字体上下文；由 `TextShaper` 持有。
pub(crate) struct TextFonts {
    pub(crate) context: FontContext,
    pub(crate) families: Vec<FontFamilyName<'static>>,
    pub(crate) dictionary: Option<Arc<dyn DictionaryProvider>>,
}

impl TextAssets {
    /// 注册字体、校验家族并构造字体上下文。
    pub(crate) fn into_fonts(self) -> Result<TextFonts, TextAssetError> {
        if self.fonts.is_empty() {
            return Err(TextAssetError::NoFonts);
        }
        let mut collection = Collection::new(CollectionOptions {
            // 集合只在创建缓存的线程上使用；共享层没有收益。
            shared: false,
            system_fonts: self.system_fallback,
        });
        for (index, font) in self.fonts.iter().enumerate() {
            let bytes = font.data.as_ref();
            if bytes.is_empty() {
                return Err(TextAssetError::MissingFont { index });
            }
            if !has_supported_signature(bytes) {
                return Err(TextAssetError::UnsupportedFont { index });
            }
            if collection
                .register_fonts(font.data.clone(), None)
                .is_empty()
            {
                return Err(TextAssetError::NoUsableFamily { index });
            }
        }

        let available: Vec<String> = collection.family_names().map(str::to_owned).collect();
        if !available.iter().any(|name| name == &self.primary) {
            return Err(TextAssetError::UnknownPrimaryFamily {
                family: self.primary,
            });
        }
        for family in &self.fallbacks {
            if !available.iter().any(|name| name == family) {
                return Err(TextAssetError::UnknownFallbackFamily {
                    family: family.clone(),
                });
            }
        }

        let mut families = Vec::with_capacity(1 + self.fallbacks.len());
        families.push(FontFamilyName::Named(Cow::Owned(self.primary)));
        families.extend(
            self.fallbacks
                .into_iter()
                .map(|name| FontFamilyName::Named(Cow::Owned(name))),
        );

        Ok(TextFonts {
            context: FontContext {
                collection,
                source_cache: Default::default(),
            },
            families,
            dictionary: self.dictionary,
        })
    }
}

/// 受支持的 sfnt 容器签名；字体集合也以 `ttcf` 开头。
const FONT_SIGNATURES: [[u8; 4]; 5] = [
    [0x00, 0x01, 0x00, 0x00],
    *b"OTTO",
    *b"true",
    *b"typ1",
    *b"ttcf",
];

/// 字节是否以受支持的容器签名开头；签名之后的内容由字体扫描器判断。
fn has_supported_signature(bytes: &[u8]) -> bool {
    let Some(signature) = bytes.get(..4).and_then(|head| head.try_into().ok()) else {
        return false;
    };
    FONT_SIGNATURES.contains(&signature)
}
