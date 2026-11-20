use std::error::Error;
use std::fmt;
use std::num::NonZeroIsize;

use windows::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, GetDpiForWindow,
    SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRect, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW,
    DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW, IDC_ARROW, LoadCursorW, MSG,
    PM_REMOVE, PeekMessageW, PostMessageW, PostQuitMessage, RegisterClassExW, SW_HIDE, SW_SHOW,
    SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, ShowWindow, TranslateMessage, WINDOW_EX_STYLE,
    WM_CLOSE, WM_DESTROY, WM_DPICHANGED, WM_LBUTTONDOWN, WM_PAINT, WM_SIZE, WNDCLASSEXW,
    WS_OVERLAPPEDWINDOW,
};
use windows::core::{HSTRING, w};
use zenui::prelude::*;
use zenui_render::{
    GpuRenderer, Point, RenderPalette, RenderTargetConfig, Scene, Viewport, Visual, wgpu,
};

/// 打开宿主窗口的选项。
#[derive(Clone, Copy, Debug)]
pub struct HostConfig {
    /// 窗口标题。
    pub title: &'static str,
    /// 是否在 `run` 时显示窗口；隐藏模式供 CI smoke 使用。
    pub visible: bool,
    /// 客户区的初始逻辑宽度。
    pub width: u32,
    /// 客户区的初始逻辑高度。
    pub height: u32,
}

impl HostConfig {
    /// 一个可见的 900×700 窗口。
    pub fn visible(title: &'static str) -> Self {
        Self {
            title,
            visible: true,
            width: 900,
            height: 700,
        }
    }

    /// 一个不显示的窗口；surface 仍会创建、配置并渲染一帧。
    pub fn hidden(title: &'static str) -> Self {
        Self {
            title,
            visible: false,
            width: 900,
            height: 700,
        }
    }
}

/// 一帧 BGRA8 像素；行已紧凑排列。
#[derive(Clone, Debug)]
pub struct Frame {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Frame {
    /// 物理像素宽度。
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// 物理像素高度。
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// 紧凑的 BGRA 字节。
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// 读取一个 BGRA 像素。
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = (y as usize * self.width as usize + x as usize) * 4;
        self.pixels
            .get(index..index + 4)
            .and_then(|pixel| pixel.try_into().ok())
    }
}

/// Win32 宿主：一个 HWND、一个 activation、一个 DX12 surface。
pub struct Host {
    runtime: Runtime,
    window_ctx: WindowCtx,
    _instance: wgpu::Instance,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    renderer: GpuRenderer,
    window: Window,
    scene: Option<Scene>,
}

impl Host {
    /// 创建窗口、挂载静态 Todo，并配置 DX12 surface；优先硬件适配器，失败时回退 WARP。
    pub fn open(config: HostConfig) -> Result<Self, HostError> {
        // 安全：进程级 DPI 设置；重复设置失败时保持已有策略。
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        let mut runtime = Runtime::new();
        let window_ctx = runtime
            .open_window("main", |_window| view! { <TodoScreen /> })
            .map_err(|error| HostError::Window(error.to_string()))?;

        let hinstance = module_handle()?;
        register_class(hinstance)?;
        let window = Window::new(config, hinstance)?;
        let hwnd = window.hwnd;

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let raw_window = raw_window_handle(hwnd, hinstance)?;
        // 安全：HWND 由 `window` 持有，销毁推迟到 Window::drop；surface 字段在
        // window 之前声明，因此 surface 先于 HWND 释放。
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(raw_display_handle()),
                raw_window_handle: raw_window,
            })
        }
        .map_err(|error| HostError::Surface(error.to_string()))?;

        let (width, height) = client_size(hwnd)?;
        let (adapter, device, queue) = request_backend(&instance, &surface)?;

        let capabilities = surface.get_capabilities(&adapter);
        // RenderPalette 的字节是 sRGB 编码值；只有非 sRGB 视图能原样保存它们。
        // sRGB 视图会再做一次编码，因此这里明确拒绝而不是静默产出错误颜色。
        if !capabilities
            .formats
            .contains(&wgpu::TextureFormat::Bgra8Unorm)
        {
            return Err(HostError::Surface(String::from(
                "the DX12 surface offers no non-sRGB BGRA format; an sRGB-only surface needs an explicit conversion",
            )));
        }
        let format = wgpu::TextureFormat::Bgra8Unorm;
        let mut surface_config = surface
            .get_default_config(&adapter, width.max(1), height.max(1))
            .ok_or_else(|| {
                HostError::Surface(String::from("the surface has no default configuration"))
            })?;
        surface_config.format = format;
        surface_config.present_mode = wgpu::PresentMode::Fifo;
        surface.configure(&device, &surface_config);
        let renderer = new_renderer(
            &device,
            &queue,
            format,
            surface_config.width,
            surface_config.height,
        )?;

        Ok(Self {
            runtime,
            window_ctx,
            _instance: instance,
            device,
            queue,
            surface,
            surface_config,
            renderer,
            window,
            scene: None,
        })
    }

    /// 窗口句柄；测试与平台集成用它在原生消息循环里投递消息。
    pub const fn hwnd(&self) -> HWND {
        self.window.hwnd
    }

    /// 最近一次呈现的场景；指针命中与几何查询使用。
    pub const fn scene(&self) -> Option<&Scene> {
        self.scene.as_ref()
    }

    /// 显示窗口（若配置为可见）并进入消息循环，直到窗口关闭。
    pub fn run(&mut self) -> Result<(), HostError> {
        if self.window.visible {
            // 安全：HWND 在 Host 生命周期内有效。
            unsafe {
                let _ = ShowWindow(self.window.hwnd, SW_SHOW);
            }
        }
        self.render()?;
        loop {
            let mut message = MSG::default();
            // 安全：标准 Win32 消息循环；窗口由 Host 持有。
            let result = unsafe { GetMessageW(&mut message, None, 0, 0).0 };
            if result == -1 {
                return Err(HostError::MessageLoop);
            }
            if result == 0 {
                break;
            }
            if self.process_message(&message)? {
                break;
            }
        }
        Ok(())
    }

    /// 处理所有已排队的窗口消息后立即返回；返回是否收到 `WM_CLOSE`。
    ///
    /// 把宿主嵌进别的消息循环，或在测试里驱动一次输入而不阻塞时使用。
    pub fn pump(&mut self) -> Result<bool, HostError> {
        loop {
            let mut message = MSG::default();
            // 安全：只取本线程队列里的消息；窗口由 Host 持有。
            let result = unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) };
            if result.0 == 0 {
                return Ok(false);
            }
            if self.process_message(&message)? {
                return Ok(true);
            }
        }
    }

    /// 分派一条消息；返回是否收到 `WM_CLOSE`。
    fn process_message(&mut self, message: &MSG) -> Result<bool, HostError> {
        // 安全：消息来自本线程消息队列，目标窗口仍有效。
        unsafe {
            let _ = TranslateMessage(message);
            DispatchMessageW(message);
        }
        if message.message == WM_LBUTTONDOWN {
            self.pointer_pressed(message.lParam)?;
            self.render()?;
        } else if message.message == WM_CLOSE {
            // 窗口的 HWND 仍然有效；Window::drop 在 surface 释放后才销毁它。
            self.runtime
                .close_window(self.window_ctx)
                .map_err(HostError::Close)?;
            return Ok(true);
        } else if matches!(message.message, WM_SIZE | WM_PAINT | WM_DPICHANGED) {
            self.render()?;
        }
        Ok(false)
    }

    /// 请求关闭窗口；消息循环会在处理 WM_CLOSE 后释放 activation。
    pub fn request_close(&self) -> Result<(), HostError> {
        // 安全：HWND 在 Host 生命周期内有效。
        unsafe { PostMessageW(Some(self.window.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }
            .map_err(|error| HostError::Window(error.to_string()))
    }

    /// 向 surface 渲染并呈现一帧；最小化或零尺寸时什么也不做。
    pub fn render(&mut self) -> Result<(), HostError> {
        self.ensure_surface()?;
        let (width, height) = client_size(self.window.hwnd)?;
        if width == 0 || height == 0 {
            return Ok(());
        }
        let scene = self.current_scene(width, height)?;
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.surface_config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Validation => {
                return Err(HostError::Surface(String::from(
                    "the surface texture could not be acquired",
                )));
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.renderer
            .render(&scene, &view, &mut encoder)
            .map_err(HostError::Render)?;
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        self.scene = Some(scene);
        Ok(())
    }

    /// 一次左键按下：命中测试后按元素类型聚焦、放置 caret 或派发控件事件。
    ///
    /// `lparam` 是客户区物理像素坐标；场景几何是逻辑像素，按最近一次呈现的 scale 换算。
    fn pointer_pressed(&mut self, lparam: LPARAM) -> Result<(), HostError> {
        let scale = self
            .scene
            .as_ref()
            .map_or(1.0, |scene| scene.viewport().scale());
        let point = pointer_point(lparam, scale);

        let Some((node, visual, caret, disabled)) = self.scene.as_ref().and_then(|scene| {
            let node = scene.hit_test(point)?;
            let element = scene.node(node)?;
            Some((
                node,
                element.visual(),
                element.caret_index(point),
                element.disabled(),
            ))
        }) else {
            return self.runtime.blur().map_err(HostError::Runtime);
        };

        // disabled 控件不聚焦、不激活，也不改变焦点。
        if disabled {
            return Ok(());
        }

        match visual {
            Visual::TextInput => {
                pointer_dispatch(self.runtime.focus_node(node))?;
                if let Some(offset) = caret {
                    pointer_dispatch(self.runtime.set_caret_node(node, offset))?;
                }
            }
            Visual::Checkbox { checked } => {
                pointer_dispatch(self.runtime.change_node(node, !checked))?;
            }
            Visual::Button { .. } => pointer_dispatch(self.runtime.click_node(node))?,
            Visual::Container | Visual::Text => {
                self.runtime.blur().map_err(HostError::Runtime)?;
            }
        }
        Ok(())
    }

    /// 用同一场景渲染到离屏纹理并读回 BGRA 像素；隐藏 CI smoke 使用。
    pub fn read_frame(&mut self) -> Result<Frame, HostError> {
        self.ensure_surface()?;
        let (width, height) = client_size(self.window.hwnd)?;
        if width == 0 || height == 0 {
            return Err(HostError::Readback(String::from(
                "cannot read a frame from an empty client area",
            )));
        }
        let scene = self.current_scene(width, height)?;
        let format = self.surface_config.format;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("zenui host readback"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.renderer
            .render(&scene, &view, &mut encoder)
            .map_err(HostError::Render)?;

        let row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("zenui host readback buffer"),
            size: u64::from(row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, |_result| {});
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| HostError::Readback(error.to_string()))?;
        let mapped = buffer
            .slice(..)
            .get_mapped_range()
            .map_err(|error| HostError::Readback(error.to_string()))?;
        let width_px = width as usize;
        let row = row as usize;
        let mut pixels = Vec::with_capacity(width_px * height as usize * 4);
        for y in 0..height as usize {
            let start = y * row;
            let end = start + width_px * 4;
            pixels.extend_from_slice(&mapped[start..end]);
        }
        drop(mapped);
        buffer.unmap();
        self.scene = Some(scene);
        Ok(Frame {
            width,
            height,
            pixels,
        })
    }

    fn ensure_surface(&mut self) -> Result<(), HostError> {
        let (width, height) = client_size(self.window.hwnd)?;
        if width == 0 || height == 0 {
            return Ok(());
        }
        if self.surface_config.width == width && self.surface_config.height == height {
            return Ok(());
        }
        self.renderer = new_renderer(
            &self.device,
            &self.queue,
            self.surface_config.format,
            width,
            height,
        )?;
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
        Ok(())
    }

    fn current_scene(&self, width: u32, height: u32) -> Result<Scene, HostError> {
        // 安全：HWND 在 Host 生命周期内有效。
        let dpi = unsafe { GetDpiForWindow(self.window.hwnd) };
        let scale = if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 };
        let viewport = Viewport::new(width as f32 / scale, height as f32 / scale, scale)
            .map_err(HostError::Scene)?;
        let snapshot = self.runtime.presentation().map_err(HostError::Runtime)?;
        Scene::build(&snapshot, viewport).map_err(HostError::Scene)
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        // Drop 不能返回错误；未通过 run 关闭的宿主在这里尽力释放 activation。
        let _ = self.runtime.close_window(self.window_ctx);
    }
}

/// 宿主的一帧失败；没有 panics。
#[derive(Debug)]
pub enum HostError {
    /// 运行时挂载或快照失败。
    Runtime(MountError),
    /// 窗口类注册、创建或查询失败。
    Window(String),
    /// surface 创建、配置或呈现失败。
    Surface(String),
    /// 离屏读回失败（零尺寸、映射或轮询错误）。
    Readback(String),
    /// 没有可用的 DX12 适配器（包括 WARP 回退）。
    NoAdapter,
    /// 设备创建失败。
    Device(String),
    /// 关闭窗口并释放 activation 失败。
    Close(WindowError),
    /// 场景构建失败。
    Scene(zenui_render::SceneError),
    /// GPU 渲染失败。
    Render(zenui_render::RenderError),
    /// 指针命中后的运行时派发失败。
    Input(DispatchError),
    /// 消息循环失败。
    MessageLoop,
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Runtime(error) => write!(f, "runtime failed: {error}"),
            Self::Window(message) => write!(f, "window creation failed: {message}"),
            Self::Surface(message) => write!(f, "surface failed: {message}"),
            Self::Readback(message) => write!(f, "frame readback failed: {message}"),
            Self::NoAdapter => f.write_str("no usable DX12 adapter, including WARP"),
            Self::Device(message) => write!(f, "device creation failed: {message}"),
            Self::Close(error) => write!(f, "closing the window failed: {error}"),
            Self::Scene(error) => write!(f, "scene failed: {error}"),
            Self::Render(error) => write!(f, "render failed: {error}"),
            Self::Input(error) => write!(f, "pointer dispatch failed: {error}"),
            Self::MessageLoop => f.write_str("the Win32 message loop failed"),
        }
    }
}

impl Error for HostError {}

struct Window {
    hwnd: HWND,
    visible: bool,
}

impl Window {
    fn new(config: HostConfig, hinstance: HINSTANCE) -> Result<Self, HostError> {
        let title: HSTRING = config.title.into();
        let style = WS_OVERLAPPEDWINDOW;
        // 配置尺寸是逻辑像素；按系统 DPI 换算成初始物理客户区。
        // 安全：进程 DPI awareness 已在 Host::open 设置。
        let dpi = unsafe { GetDpiForSystem() };
        let scale = if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 };
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: (config.width as f32 * scale) as i32,
            bottom: (config.height as f32 * scale) as i32,
        };
        // 安全：rect 与 style 来自本地值；bmenu=false。
        unsafe { AdjustWindowRect(&mut rect, style, false) }
            .map_err(|error| HostError::Window(error.to_string()))?;
        // 安全：类名已在 register_class 注册；标题与实例句柄在调用期间有效。
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("ZenUIHostWindow"),
                &title,
                style,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                rect.right - rect.left,
                rect.bottom - rect.top,
                None,
                None,
                Some(hinstance),
                None,
            )
        }
        .map_err(|error| HostError::Window(error.to_string()))?;
        Ok(Self {
            hwnd,
            visible: config.visible,
        })
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        // 安全：HWND 由本结构独占；重复销毁只返回错误。
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn module_handle() -> Result<HINSTANCE, HostError> {
    // 安全：请求当前进程的可执行模块句柄。
    unsafe { GetModuleHandleW(None) }
        .map(|module| HINSTANCE(module.0))
        .map_err(|error| HostError::Window(error.to_string()))
}

fn register_class(hinstance: HINSTANCE) -> Result<(), HostError> {
    // 安全：加载系统光标；失败时退回默认光标。
    let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default();
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc),
        hInstance: hinstance,
        hCursor: cursor,
        lpszClassName: w!("ZenUIHostWindow"),
        ..Default::default()
    };
    // 安全：class 在调用期间有效。
    let atom = unsafe { RegisterClassExW(&class) };
    if atom == 0 {
        let error = unsafe { GetLastError() };
        if error != ERROR_CLASS_ALREADY_EXISTS {
            return Err(HostError::Window(format!(
                "RegisterClassExW failed: {error:?}"
            )));
        }
    }
    Ok(())
}

fn raw_window_handle(
    hwnd: HWND,
    hinstance: HINSTANCE,
) -> Result<wgpu::rwh::RawWindowHandle, HostError> {
    let hwnd = NonZeroIsize::new(hwnd.0 as isize)
        .ok_or_else(|| HostError::Window(String::from("the window handle is null")))?;
    let mut handle = wgpu::rwh::Win32WindowHandle::new(hwnd);
    handle.hinstance = NonZeroIsize::new(hinstance.0 as isize);
    Ok(wgpu::rwh::RawWindowHandle::Win32(handle))
}

fn raw_display_handle() -> wgpu::rwh::RawDisplayHandle {
    wgpu::rwh::RawDisplayHandle::Windows(wgpu::rwh::WindowsDisplayHandle::new())
}

/// 先请求硬件适配器；适配器或设备创建失败时整体回退 WARP。
fn request_backend(
    instance: &wgpu::Instance,
    surface: &wgpu::Surface<'_>,
) -> Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue), HostError> {
    let mut options = wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: Some(surface),
        force_fallback_adapter: false,
        ..Default::default()
    };
    let mut device_error = None;
    for force_fallback_adapter in [false, true] {
        options.force_fallback_adapter = force_fallback_adapter;
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&options)) else {
            continue;
        };
        match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) {
            Ok((device, queue)) => return Ok((adapter, device, queue)),
            Err(error) => device_error = Some(error.to_string()),
        }
    }
    Err(match device_error {
        Some(message) => HostError::Device(message),
        None => HostError::NoAdapter,
    })
}

/// 命中元素可以没有对应 handler（例如静态按钮）、处于不可聚焦状态，
/// 或在派发过程中被 handler 重挂载而失效；这些拒绝是正常交互结果，
/// 不是宿主失败。
fn pointer_dispatch(result: Result<(), DispatchError>) -> Result<(), HostError> {
    match result {
        Ok(())
        | Err(
            DispatchError::WrongKind { .. }
            | DispatchError::Rejected { .. }
            | DispatchError::StaleNode(_),
        ) => Ok(()),
        Err(error) => Err(HostError::Input(error)),
    }
}

fn pointer_point(lparam: LPARAM, scale: f32) -> Point {
    let x = (lparam.0 & 0xffff) as u16 as i16 as f32;
    let y = ((lparam.0 >> 16) & 0xffff) as u16 as i16 as f32;
    Point::new(x / scale, y / scale)
}

fn client_size(hwnd: HWND) -> Result<(u32, u32), HostError> {
    let mut rect = RECT::default();
    // 安全：rect 是有效的输出缓冲。
    unsafe { GetClientRect(hwnd, &mut rect) }
        .map_err(|error| HostError::Window(error.to_string()))?;
    Ok((
        (rect.right - rect.left).max(0) as u32,
        (rect.bottom - rect.top).max(0) as u32,
    ))
}

fn new_renderer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> Result<GpuRenderer, HostError> {
    let target = target_config(format, width, height)?;
    Ok(GpuRenderer::from_parts(
        device,
        queue,
        target,
        RenderPalette::default(),
    ))
}

fn target_config(
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> Result<RenderTargetConfig, HostError> {
    let width = u16::try_from(width)
        .map_err(|_| HostError::Surface(String::from("the surface is wider than 65535 pixels")))?;
    let height = u16::try_from(height)
        .map_err(|_| HostError::Surface(String::from("the surface is taller than 65535 pixels")))?;
    RenderTargetConfig::new(format, width, height).map_err(HostError::Render)
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_CLOSE => {
            // 隐藏窗口并结束消息循环；销毁推迟到 Window::drop，使 HWND 在 surface
            // 存活期间保持有效。
            // 安全：hwnd 来自系统回调。
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // 安全：终止本线程的消息循环。
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let suggested = lparam.0 as *const RECT;
            if !suggested.is_null() {
                // 安全：WM_DPICHANGED 的 lParam 指向系统提供的 RECT。
                let suggested = unsafe { *suggested };
                // 安全：按系统建议调整窗口位置与大小。
                unsafe {
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        suggested.left,
                        suggested.top,
                        suggested.right - suggested.left,
                        suggested.bottom - suggested.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            }
            LRESULT(0)
        }
        _ => {
            // 安全：默认窗口过程。
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
    }
}

#[component]
fn TodoScreen() -> MountResult<MountPlan> {
    let write_tests = signal(true);
    let read_docs = signal(false);
    let filter = signal(Filter::All);
    let draft = signal(String::new());

    let write_tests_checked = memo(move || write_tests.get());
    let read_docs_checked = memo(move || read_docs.get());
    let remaining = memo(move || usize::from(!write_tests.get()) + usize::from(!read_docs.get()));
    let remaining_text = memo(move || format!("{} remaining", remaining.get()));
    let all = memo(move || filter.get() == Filter::All);
    let active = memo(move || filter.get() == Filter::Active);
    let done = memo(move || filter.get() == Filter::Completed);
    let show_write = memo(move || match filter.get() {
        Filter::All => true,
        Filter::Active => !write_tests.get(),
        Filter::Completed => write_tests.get(),
    });
    let show_read = memo(move || match filter.get() {
        Filter::All => true,
        Filter::Active => !read_docs.get(),
        Filter::Completed => read_docs.get(),
    });

    view! {
        <view role="main">
            <text>{remaining_text}</text>
            <form aria:label="Add task">
                <text-input
                    value=draft
                    aria:label="What needs to be done?"
                    on:input=move |value: String| draft.set(value)
                />
                <button on:click=move || draft.set(String::new())>"Add"</button>
            </form>
            <view role="group" aria:label="Filter">
                <button aria:pressed=all on:click=move || filter.set(Filter::All)>"All"</button>
                <button aria:pressed=active on:click=move || filter.set(Filter::Active)>"Active"</button>
                <button aria:pressed=done on:click=move || filter.set(Filter::Completed)>"Done"</button>
            </view>
            <Show when=show_write>
                <view role="listitem">
                    <checkbox
                        checked=write_tests_checked
                        aria:label="Complete Write tests"
                        on:change=move |done: bool| write_tests.set(done)
                    />
                    <text>"Write tests"</text>
                    <text-input value="Write tests" aria:label="Edit Write tests" />
                    <button aria:label="Delete Write tests">"Delete"</button>
                </view>
            </Show>
            <Show when=show_read>
                <view role="listitem">
                    <checkbox
                        checked=read_docs_checked
                        aria:label="Complete Read docs"
                        on:change=move |done: bool| read_docs.set(done)
                    />
                    <text>"Read docs"</text>
                    <text-input value="Read docs" aria:label="Edit Read docs" />
                    <button aria:label="Delete Read docs">"Delete"</button>
                </view>
            </Show>
        </view>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_points_scale_from_physical_to_logical() {
        let lparam = LPARAM((100isize << 16) | 200);
        assert_eq!(pointer_point(lparam, 2.0), Point::new(100.0, 50.0));
    }

    #[test]
    fn pointer_points_preserve_negative_coordinates() {
        let packed = (-5isize & 0xffff) | ((-9isize & 0xffff) << 16);
        assert_eq!(pointer_point(LPARAM(packed), 1.0), Point::new(-5.0, -9.0));
    }

    #[test]
    fn pointer_dispatch_treats_a_rejected_blur_as_a_no_op() {
        let mut app = Runtime::new();
        app.open_window("main", |_| {
            view! { <text-input value="x" aria:label="Input" /> }
        })
        .expect("open window");
        let id = app
            .presentation()
            .expect("snapshot")
            .find_label("Input")
            .expect("input")
            .id();
        assert!(
            app.blur_node(id).is_err(),
            "an unfocused input rejects blur"
        );
        assert!(
            pointer_dispatch(app.blur_node(id)).is_ok(),
            "the host ignores the rejection"
        );
    }
}
