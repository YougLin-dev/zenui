//! GPU 渲染层：把 [`Scene`] 通过 `vello_gpu`/wgpu 画进任意 [`TextureView`]。
//!
//! 场景几何是逻辑像素；渲染时按 `Scene::viewport().scale()` 变换成物理像素。
//! 目标视图的格式与尺寸由 [`RenderTargetConfig`] 声明，视图必须匹配。

use std::error::Error;
use std::fmt;

use vello_gpu::color::{AlphaColor, Srgb};
use vello_gpu::kurbo::{Rect as KurboRect, RoundedRect, Shape, Stroke};
use vello_gpu::{
    ClearSettings, RenderOptions, RenderSize, RenderTarget,
    RenderTargetConfig as VelloTargetConfig, TargetInit, TextureBindings,
};
use wgpu::{CommandEncoder, Device, Queue, TextureFormat, TextureView};
use zenui_style::{Color, ColorRole};

use crate::geometry::Rect;
use crate::scene::{DecorationKind, ResolvedStyle, Scene, TextRun, Visual};

/// GPU 层绘制内建视觉与 class 颜色角色时使用的调色板；各字段是 RGBA 字节，
/// 按目标视图的转移函数解释。
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
    /// 指针悬停时的控件底色。
    pub control_hover: [u8; 4],
    /// 指针按下时的控件底色。
    pub control_pressed: [u8; 4],
    /// 默认文本颜色。
    pub text: [u8; 4],
    /// 次要文本颜色。
    pub text_muted: [u8; 4],
    /// 强调色背景上的文本颜色。
    pub text_inverse: [u8; 4],
    /// 禁用文本颜色。
    pub text_disabled: [u8; 4],
    /// 控件边框。
    pub border: [u8; 4],
    /// 选中、按下与 preedit 下划线。
    pub accent: [u8; 4],
    /// 悬停时的强调色。
    pub accent_hover: [u8; 4],
    /// 按下时的强调色。
    pub accent_pressed: [u8; 4],
    /// 危险操作的底色。
    pub danger: [u8; 4],
    /// 悬停时的危险色。
    pub danger_hover: [u8; 4],
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
            control_hover: [212, 212, 216, 255],
            control_pressed: [161, 161, 170, 255],
            text: [24, 24, 27, 255],
            text_muted: [82, 82, 91, 255],
            text_inverse: [255, 255, 255, 255],
            text_disabled: [161, 161, 170, 255],
            border: [161, 161, 170, 255],
            accent: [37, 99, 235, 255],
            accent_hover: [29, 78, 216, 255],
            accent_pressed: [30, 64, 175, 255],
            danger: [220, 38, 38, 255],
            danger_hover: [185, 28, 28, 255],
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

    /// 调整目标尺寸；复用已有的 vello 渲染器与资源，只重建深度视图。
    ///
    /// vello 在每帧准备阶段按目标尺寸更新配置缓冲，因此窗口缩放不需要重建
    /// pipeline 与渲染资源。
    pub fn resize(&mut self, config: RenderTargetConfig) {
        self.depth = self.renderer.create_depth_texture_view(&RenderSize {
            width: config.width,
            height: config.height,
        });
        self.config = config;
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
        let style = element.resolved_style();
        paint_background(target, rect, element, style, palette, scale);
        paint_border(target, rect, element.visual(), style, palette, scale);
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
                fill(
                    target,
                    physical_rect(decoration.rect(), scale),
                    with_opacity(paint, style.opacity()),
                );
            }
        }
        if !element.text().is_empty() {
            let intrinsic = if element.disabled() {
                palette.text_disabled
            } else if matches!(element.visual(), Visual::Button { pressed: true }) {
                palette.text_inverse
            } else {
                palette.text
            };
            let paint = style
                .text_color()
                .map_or(intrinsic, |color| resolve_color(color, palette));
            target.set_paint(color(with_opacity(paint, style.opacity())));
            for run in element.text() {
                glyphs(target, resources, run, scale)?;
            }
        }
    }
    if scene.caret_visible() {
        for decoration in scene.decorations() {
            if decoration.kind() == DecorationKind::Caret {
                let opacity = scene
                    .node(decoration.node())
                    .map_or(1.0, |element| element.resolved_style().opacity());
                fill(
                    target,
                    physical_rect(decoration.rect(), scale),
                    with_opacity(palette.caret, opacity),
                );
            }
        }
    }
    Ok(())
}

/// intrinsic 的背景先铺，class 的 `bg-*` 覆盖它；checkbox 的勾选块始终是强调色。
fn paint_background(
    target: &mut vello_gpu::Scene,
    rect: KurboRect,
    element: &crate::scene::SceneElement,
    style: ResolvedStyle,
    palette: RenderPalette,
    scale: f32,
) {
    let visual = element.visual();
    let control = if element.pointer_pressed() {
        palette.control_pressed
    } else if element.hovered() {
        palette.control_hover
    } else {
        palette.control
    };
    let intrinsic = match visual {
        Visual::Container => Some(palette.surface),
        Visual::Text => None,
        Visual::Button { pressed: true } => Some(if element.pointer_pressed() {
            palette.accent_pressed
        } else if element.hovered() {
            palette.accent_hover
        } else {
            palette.accent
        }),
        Visual::Button { pressed: false } | Visual::Checkbox { .. } | Visual::TextInput => {
            Some(control)
        }
    };
    let background = style
        .background()
        .map(|color| resolve_color(color, palette))
        .or(intrinsic);
    if let Some(paint) = background {
        fill_rounded(target, rect, paint, style, scale);
    }
    if let Visual::Checkbox { checked: true } = visual {
        let inset = 4.0 * f64::from(scale);
        fill(
            target,
            KurboRect::new(
                rect.x0 + inset,
                rect.y0 + inset,
                rect.x1 - inset,
                rect.y1 - inset,
            ),
            with_opacity(palette.accent, style.opacity()),
        );
    }
}

/// intrinsic 的控件边框先铺，class 的 `border` / `border-*` 覆盖宽度或颜色。
fn paint_border(
    target: &mut vello_gpu::Scene,
    rect: KurboRect,
    visual: Visual,
    style: ResolvedStyle,
    palette: RenderPalette,
    scale: f32,
) {
    let width = style.border_width().unwrap_or(match visual {
        Visual::Button { .. } | Visual::Checkbox { .. } | Visual::TextInput => 1.0,
        Visual::Container | Visual::Text => 0.0,
    });
    if width <= 0.0 {
        return;
    }
    let paint = style
        .border_color()
        .map_or(palette.border, |color| resolve_color(color, palette));
    stroke_rounded(target, rect, paint, style, width * scale, scale);
}

fn resolve_color(color: Color, palette: RenderPalette) -> [u8; 4] {
    match color {
        Color::Role(role) => role_color(role, palette),
        Color::Literal(rgba) => rgba,
    }
}

/// 语义角色到当前调色板的分派；暗色调色板只需提供同一组字段。
fn role_color(role: ColorRole, palette: RenderPalette) -> [u8; 4] {
    match role {
        ColorRole::Surface => palette.surface,
        ColorRole::Control => palette.control,
        ColorRole::ControlHover => palette.control_hover,
        ColorRole::ControlPressed => palette.control_pressed,
        ColorRole::Accent => palette.accent,
        ColorRole::AccentHover => palette.accent_hover,
        ColorRole::AccentPressed => palette.accent_pressed,
        ColorRole::Danger => palette.danger,
        ColorRole::DangerHover => palette.danger_hover,
        ColorRole::TextDefault => palette.text,
        ColorRole::TextMuted => palette.text_muted,
        ColorRole::TextInverse => palette.text_inverse,
        ColorRole::TextDisabled => palette.text_disabled,
        ColorRole::BorderDefault => palette.border,
        ColorRole::BorderAccent => palette.accent,
    }
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

fn fill(target: &mut vello_gpu::Scene, rect: KurboRect, paint: [u8; 4]) {
    target.set_paint(color(paint));
    target.fill_rect(&rect);
}

fn fill_rounded(
    target: &mut vello_gpu::Scene,
    rect: KurboRect,
    paint: [u8; 4],
    style: ResolvedStyle,
    scale: f32,
) {
    target.set_paint(color(with_opacity(paint, style.opacity())));
    if style.radius() > 0.0 {
        target.fill_path(&rounded_path(rect, style.radius(), scale));
    } else {
        target.fill_rect(&rect);
    }
}

fn stroke_rounded(
    target: &mut vello_gpu::Scene,
    rect: KurboRect,
    paint: [u8; 4],
    style: ResolvedStyle,
    width: f32,
    scale: f32,
) {
    target.set_paint(color(with_opacity(paint, style.opacity())));
    target.set_stroke(Stroke::new(f64::from(width)));
    if style.radius() > 0.0 {
        target.stroke_path(&rounded_path(rect, style.radius(), scale));
    } else {
        target.stroke_rect(&rect);
    }
}

fn rounded_path(rect: KurboRect, radius: f32, scale: f32) -> vello_gpu::kurbo::BezPath {
    RoundedRect::from_rect(rect, f64::from(radius) * f64::from(scale)).to_path(0.1)
}

fn with_opacity(paint: [u8; 4], opacity: f32) -> [u8; 4] {
    let alpha = (f32::from(paint[3]) * opacity).clamp(0.0, 255.0).round() as u8;
    [paint[0], paint[1], paint[2], alpha]
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
