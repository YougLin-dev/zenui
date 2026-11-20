//! GPU 渲染层：把 [`Scene`] 通过 `vello_gpu`/wgpu 画进任意 [`TextureView`]。
//!
//! 场景几何是逻辑像素；渲染时按 `Scene::viewport().scale()` 变换成物理像素。
//! 目标视图的格式与尺寸由 [`RenderTargetConfig`] 声明，视图必须匹配。

use std::error::Error;
use std::fmt;

use vello_gpu::color::{AlphaColor, Srgb};
use vello_gpu::kurbo::{Rect as KurboRect, Stroke};
use vello_gpu::{
    ClearSettings, RenderOptions, RenderSize, RenderTarget,
    RenderTargetConfig as VelloTargetConfig, TargetInit, TextureBindings,
};
use wgpu::{CommandEncoder, Device, Queue, TextureFormat, TextureView};

use crate::geometry::Rect;
use crate::scene::{DecorationKind, Scene, TextRun, Visual};

/// GPU 层绘制内建视觉时使用的颜色；各字段是 RGBA 字节，按目标视图的转移函数解释。
///
/// 纯色（0 或 255）在两种 BGRA 格式下写出相同的字节；中间调在 sRGB 视图上
/// 会再经过一次传输编码。显示用途应请求非 sRGB 视图；只有 sRGB 视图时需要
/// 显式的转换或 blit。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderPalette {
    /// 目标清屏色。
    pub background: [u8; 4],
    /// `view` / `form` 容器底色。
    pub surface: [u8; 4],
    /// 按钮、输入框与复选框底色。
    pub control: [u8; 4],
    /// 文本颜色。
    pub text: [u8; 4],
    /// 控件边框。
    pub border: [u8; 4],
    /// 选中、按下与 preedit 下划线。
    pub accent: [u8; 4],
    /// selection 高亮。
    pub selection: [u8; 4],
    /// caret。
    pub caret: [u8; 4],
}

impl Default for RenderPalette {
    fn default() -> Self {
        Self {
            background: [244, 244, 245, 255],
            surface: [255, 255, 255, 255],
            control: [228, 228, 231, 255],
            text: [24, 24, 27, 255],
            border: [161, 161, 170, 255],
            accent: [37, 99, 235, 255],
            selection: [37, 99, 235, 80],
            caret: [24, 24, 27, 255],
        }
    }
}

/// 渲染目标的物理像素格式与尺寸。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderTargetConfig {
    format: TextureFormat,
    width: u16,
    height: u16,
}

impl RenderTargetConfig {
    /// 只接受 `Bgra8Unorm` 与 `Bgra8UnormSrgb`，并要求非零尺寸。
    pub fn new(format: TextureFormat, width: u16, height: u16) -> Result<Self, RenderError> {
        if !matches!(
            format,
            TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb
        ) {
            return Err(RenderError::UnsupportedFormat { format });
        }
        if width == 0 || height == 0 {
            return Err(RenderError::InvalidTargetSize { width, height });
        }
        Ok(Self {
            format,
            width,
            height,
        })
    }

    /// 目标视图的纹理格式。
    /// 目标视图必须匹配的纹理格式。
    pub const fn format(self) -> TextureFormat {
        self.format
    }

    /// 目标视图必须匹配的物理宽度。
    pub const fn width(self) -> u16 {
        self.width
    }

    /// 目标视图必须匹配的物理高度。
    pub const fn height(self) -> u16 {
        self.height
    }

    fn vello(self) -> VelloTargetConfig {
        VelloTargetConfig {
            format: self.format,
            width: self.width,
            height: self.height,
        }
    }
}

/// 一个 wgpu 设备与队列；适配器或设备不可用时返回明确错误。
pub struct GpuContext {
    device: Device,
    queue: Queue,
}

impl GpuContext {
    /// 请求默认适配器与设备。没有可用适配器时返回 [`RenderError::NoAdapter`]。
    pub fn new() -> Result<Self, RenderError> {
        pollster::block_on(async {
            let adapter = wgpu::Instance::default()
                .request_adapter(&Default::default())
                .await
                .map_err(|_| RenderError::NoAdapter)?;
            let (device, queue) =
                adapter
                    .request_device(&Default::default())
                    .await
                    .map_err(|error| RenderError::NoDevice {
                        message: error.to_string(),
                    })?;
            Ok(Self { device, queue })
        })
    }

    /// 创建设备时使用的 wgpu device。
    pub const fn device(&self) -> &Device {
        &self.device
    }

    /// 与设备配套的 wgpu queue。
    pub const fn queue(&self) -> &Queue {
        &self.queue
    }
}

/// 持有 vello 资源与目标配置的渲染器；每次渲染完整重放 [`Scene`]。
pub struct GpuRenderer {
    renderer: vello_gpu::Renderer,
    resources: vello_gpu::Resources,
    depth: TextureView,
    config: RenderTargetConfig,
    palette: RenderPalette,
}

impl GpuRenderer {
    /// 为目标配置创建渲染器；视图必须与该配置的格式和尺寸一致。
    pub fn new(context: &GpuContext, config: RenderTargetConfig, palette: RenderPalette) -> Self {
        Self::from_parts(context.device(), context.queue(), config, palette)
    }

    /// 用已有的 device/queue 创建渲染器；surface 宿主可复用它的适配器。
    pub fn from_parts(
        device: &Device,
        queue: &Queue,
        config: RenderTargetConfig,
        palette: RenderPalette,
    ) -> Self {
        let (renderer, resources) = vello_gpu::Renderer::new(device, queue, &config.vello());
        let depth = renderer.create_depth_texture_view(&RenderSize {
            width: config.width,
            height: config.height,
        });
        Self {
            renderer,
            resources,
            depth,
            config,
            palette,
        }
    }

    /// 把场景编码进 `encoder`；调用方负责提交。
    pub fn render(
        &mut self,
        scene: &Scene,
        target: &TextureView,
        encoder: &mut CommandEncoder,
    ) -> Result<(), RenderError> {
        let palette = self.palette;
        let mut vello_scene = vello_gpu::Scene::new(self.config.width, self.config.height);
        paint(scene, &mut vello_scene, &mut self.resources, palette)?;
        let mut options = RenderOptions::default();
        options.target_init = TargetInit::Clear(ClearSettings::Viewport {
            color: color(palette.background),
        });
        self.renderer
            .render(
                &vello_scene,
                &mut self.resources,
                encoder,
                RenderTarget {
                    view: target,
                    depth_view: Some(&self.depth),
                },
                &TextureBindings::new(),
                options,
            )
            .map_err(|error| RenderError::Render {
                message: error.to_string(),
            })
    }
}

/// GPU 渲染失败；没有 panics。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderError {
    /// 没有可用的 wgpu 适配器。
    NoAdapter,
    /// 适配器拒绝创建设备。
    NoDevice {
        /// 底层错误描述。
        message: String,
    },
    /// 目标格式不是受支持的 BGRA 格式。
    UnsupportedFormat {
        /// 被拒绝的格式。
        format: TextureFormat,
    },
    /// 目标尺寸为零。
    InvalidTargetSize {
        /// 请求的物理宽度。
        width: u16,
        /// 请求的物理高度。
        height: u16,
    },
    /// vello 或 glifo 渲染失败。
    Render {
        /// 底层错误描述。
        message: String,
    },
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAdapter => f.write_str("no usable wgpu adapter"),
            Self::NoDevice { message } => write!(f, "wgpu device creation failed: {message}"),
            Self::UnsupportedFormat { format } => {
                write!(f, "unsupported render target format {format:?}")
            }
            Self::InvalidTargetSize { width, height } => {
                write!(
                    f,
                    "render target size must be non-zero, got {width}x{height}"
                )
            }
            Self::Render { message } => write!(f, "GPU rendering failed: {message}"),
        }
    }
}

impl Error for RenderError {}

fn paint(
    scene: &Scene,
    target: &mut vello_gpu::Scene,
    resources: &mut vello_gpu::Resources,
    palette: RenderPalette,
) -> Result<(), RenderError> {
    let scale = scene.viewport().scale();
    for element in scene.elements() {
        let rect = physical_rect(element.rect(), scale);
        match element.visual() {
            Visual::Container => fill(target, rect, palette.surface),
            Visual::Text => {}
            Visual::Button { pressed } => {
                if pressed {
                    fill(target, rect, palette.accent);
                    outline(target, rect, palette.border, scale);
                } else {
                    control_box(target, rect, palette, scale);
                }
            }
            Visual::Checkbox { checked } => {
                control_box(target, rect, palette, scale);
                if checked {
                    let inset = 4.0 * f64::from(scale);
                    fill(
                        target,
                        KurboRect::new(
                            rect.x0 + inset,
                            rect.y0 + inset,
                            rect.x1 - inset,
                            rect.y1 - inset,
                        ),
                        palette.accent,
                    );
                }
            }
            Visual::TextInput => control_box(target, rect, palette, scale),
        }
        for decoration in scene.decorations() {
            if decoration.node() != element.id() {
                continue;
            }
            let paint = match decoration.kind() {
                DecorationKind::Selection => Some(palette.selection),
                DecorationKind::PreeditUnderline => Some(palette.accent),
                DecorationKind::Caret => None,
            };
            if let Some(paint) = paint {
                fill(target, physical_rect(decoration.rect(), scale), paint);
            }
        }
        if !element.text().is_empty() {
            target.set_paint(color(palette.text));
            for run in element.text() {
                glyphs(target, resources, run, scale)?;
            }
        }
    }
    for decoration in scene.decorations() {
        if decoration.kind() == DecorationKind::Caret {
            fill(
                target,
                physical_rect(decoration.rect(), scale),
                palette.caret,
            );
        }
    }
    Ok(())
}

fn glyphs(
    target: &mut vello_gpu::Scene,
    resources: &mut vello_gpu::Resources,
    run: &TextRun,
    scale: f32,
) -> Result<(), RenderError> {
    let glyphs = run.glyphs().iter().map(|glyph| glifo::Glyph {
        id: glyph.id,
        x: glyph.x * scale,
        y: glyph.y * scale,
    });
    target
        .glyph_run(resources, run.font())
        .font_size(run.font_size() * scale)
        .normalized_coords(run.normalized_coords())
        .atlas_cache(false)
        .fill_glyphs(glyphs)
        .map_err(|error| RenderError::Render {
            message: error.to_string(),
        })
}

fn control_box(target: &mut vello_gpu::Scene, rect: KurboRect, palette: RenderPalette, scale: f32) {
    fill(target, rect, palette.control);
    outline(target, rect, palette.border, scale);
}

fn fill(target: &mut vello_gpu::Scene, rect: KurboRect, paint: [u8; 4]) {
    target.set_paint(color(paint));
    target.fill_rect(&rect);
}

fn outline(target: &mut vello_gpu::Scene, rect: KurboRect, paint: [u8; 4], scale: f32) {
    target.set_paint(color(paint));
    target.set_stroke(Stroke::new(f64::from(scale)));
    target.stroke_rect(&rect);
}

fn physical_rect(rect: Rect, scale: f32) -> KurboRect {
    let scale = f64::from(scale);
    KurboRect::new(
        f64::from(rect.x) * scale,
        f64::from(rect.y) * scale,
        f64::from(rect.right()) * scale,
        f64::from(rect.bottom()) * scale,
    )
}

fn color(rgba: [u8; 4]) -> AlphaColor<Srgb> {
    AlphaColor::from_rgba8(rgba[0], rgba[1], rgba[2], rgba[3])
}
