use std::error::Error;
use std::fmt;
use std::num::NonZeroIsize;

use windows::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem,
    GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::Ime::{
    CANDIDATEFORM, CFS_CANDIDATEPOS, CFS_POINT, COMPOSITIONFORM, CPS_CANCEL, GCS_COMPSTR,
    GCS_RESULTSTR, IME_COMPOSITION_STRING, ImmGetCompositionStringW, ImmGetContext, ImmNotifyIME,
    ImmReleaseContext, ImmSetCandidateWindow, ImmSetCompositionWindow, NI_COMPOSITIONSTR,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VIRTUAL_KEY, VK_A, VK_BACK, VK_CONTROL, VK_DELETE, VK_END, VK_ESCAPE, VK_HOME,
    VK_LEFT, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, GetClientRect, GetMessageW, IDC_ARROW, LoadCursorW, MSG, PM_REMOVE,
    PeekMessageW, PostMessageW, PostQuitMessage, RegisterClassExW, SW_HIDE, SW_SHOW,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SetWindowPos, ShowWindow, TranslateMessage,
    WINDOW_EX_STYLE, WM_CHAR, WM_CLOSE, WM_DESTROY, WM_DPICHANGED, WM_IME_CHAR, WM_IME_COMPOSITION,
    WM_IME_ENDCOMPOSITION, WM_IME_STARTCOMPOSITION, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDOWN,
    WM_PAINT, WM_SIZE, WNDCLASSEXW, WS_OVERLAPPEDWINDOW,
};
use windows::core::{HSTRING, w};
use zenui::prelude::*;
use zenui_render::{
    GpuRenderer, Point, RenderPalette, RenderTargetConfig, Scene, Viewport, Visual, wgpu,
};

use zenui_demo::TodoApp;

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
    surface: Option<wgpu::Surface<'static>>,
    surface_config: wgpu::SurfaceConfiguration,
    renderer: GpuRenderer,
    window: Window,
    scene: Option<Scene>,
    pending_high_surrogate: Option<u16>,
}

impl Host {
    /// 创建窗口、挂载共享 Todo demo，并配置 DX12 surface；优先硬件适配器，失败时回退 WARP。
    pub fn open(config: HostConfig) -> Result<Self, HostError> {
        // 安全：进程级 DPI 设置；重复设置失败时保持已有策略。
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        let mut runtime = Runtime::new();
        let window_ctx = runtime
            .open_window("main", |window| view! { <TodoApp window=window /> })
            .map_err(|error| HostError::Window(error.to_string()))?;
        zenui_demo::seed(&mut runtime).map_err(HostError::Runtime)?;

        let hinstance = module_handle()?;
        register_class(hinstance)?;
        let window = Window::new(config, hinstance)?;
        let hwnd = window.hwnd;

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = create_surface(&instance, hwnd, hinstance)?;
        let presentation = configure_presentation(&instance, &surface, hwnd)?;

        Ok(Self {
            runtime,
            window_ctx,
            _instance: instance,
            device: presentation.device,
            queue: presentation.queue,
            surface: Some(surface),
            surface_config: presentation.config,
            renderer: presentation.renderer,
            window,
            scene: None,
            pending_high_surrogate: None,
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
        // 提交已经由 WM_IME_COMPOSITION 的 GCS_RESULTSTR 处理；WM_IME_CHAR
        // 若再交给默认窗口过程会产生重复字符。
        if message.message == WM_IME_CHAR {
            return Ok(false);
        }
        // 安全：消息来自本线程消息队列，目标窗口仍有效。
        unsafe {
            let _ = TranslateMessage(message);
            DispatchMessageW(message);
        }
        if message.message == WM_LBUTTONDOWN {
            self.pointer_pressed(message.lParam)?;
            self.render()?;
        } else if message.message == WM_CHAR {
            self.char_typed(message.wParam)?;
        } else if message.message == WM_KEYDOWN {
            self.key_down(message.wParam)?;
        } else if message.message == WM_IME_STARTCOMPOSITION {
            self.ime_start()?;
        } else if message.message == WM_IME_COMPOSITION {
            self.ime_composition(message.lParam)?;
        } else if matches!(message.message, WM_IME_ENDCOMPOSITION | WM_KILLFOCUS) {
            self.ime_cancel()?;
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

    /// 按逻辑尺寸调整客户区并立即重绘；供嵌入方与测试使用。
    ///
    /// # Errors
    ///
    /// 窗口尺寸调整失败时返回 [`HostError::Window`]；随后的重绘可能返回
    /// surface 或渲染错误。
    pub fn resize(&mut self, logical_width: u32, logical_height: u32) -> Result<(), HostError> {
        // 安全：HWND 在 Host 生命周期内有效。
        let dpi = unsafe { GetDpiForWindow(self.window.hwnd) };
        let dpi = dpi.max(96);
        let scale = dpi as f32 / 96.0;
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: (logical_width as f32 * scale) as i32,
            bottom: (logical_height as f32 * scale) as i32,
        };
        // 安全：rect 由本地值初始化；按窗口 DPI 计算非客户区。
        unsafe {
            AdjustWindowRectExForDpi(
                &mut rect,
                WS_OVERLAPPEDWINDOW,
                false,
                WINDOW_EX_STYLE::default(),
                dpi,
            )
        }
        .map_err(|error| HostError::Window(error.to_string()))?;
        let physical_width = rect.right - rect.left;
        let physical_height = rect.bottom - rect.top;
        // 只调整窗口尺寸，位置与 Z 序不变。
        unsafe {
            SetWindowPos(
                self.window.hwnd,
                None,
                0,
                0,
                physical_width,
                physical_height,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        }
        .map_err(|error| HostError::Window(error.to_string()))?;
        self.render()
    }

    /// 向 surface 渲染并呈现一帧；最小化或零尺寸时什么也不做。
    ///
    /// # Errors
    ///
    /// surface 或 device 无法获取、重配置或重建时返回 [`HostError::Surface`]；
    /// 场景构建与 GPU 绘制失败分别返回对应的错误。
    pub fn render(&mut self) -> Result<(), HostError> {
        let mut recovered = false;
        loop {
            self.ensure_surface()?;
            let (width, height) = client_size(self.window.hwnd)?;
            if width == 0 || height == 0 {
                return Ok(());
            }
            let (scene, snapshot) = self.current_scene(width, height)?;
            let status = self.current_surface()?.get_current_texture();
            let frame = match status {
                wgpu::CurrentSurfaceTexture::Success(frame)
                | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    return Ok(());
                }
                wgpu::CurrentSurfaceTexture::Outdated => {
                    if recovered {
                        return Err(HostError::Surface(String::from(
                            "the surface stayed outdated after reconfiguration",
                        )));
                    }
                    self.current_surface()?
                        .configure(&self.device, &self.surface_config);
                    recovered = true;
                    continue;
                }
                wgpu::CurrentSurfaceTexture::Lost => {
                    if recovered {
                        return Err(HostError::Surface(String::from(
                            "the surface stayed lost after rebuilding",
                        )));
                    }
                    self.rebuild_presentation()?;
                    recovered = true;
                    continue;
                }
                wgpu::CurrentSurfaceTexture::Validation => {
                    return Err(HostError::Surface(String::from(
                        "the surface failed validation; rebuilding would mask an API misuse",
                    )));
                }
            };
            let view = frame.texture.create_view(&Default::default());
            let mut encoder = self.device.create_command_encoder(&Default::default());
            if let Err(error) = self.renderer.render(&scene, &view, &mut encoder) {
                if recovered {
                    return Err(HostError::Render(error));
                }
                self.rebuild_presentation()?;
                recovered = true;
                continue;
            }
            self.queue.submit([encoder.finish()]);
            self.queue.present(frame);
            self.scene = Some(scene);
            self.position_ime(&snapshot);
            return Ok(());
        }
    }

    /// 一次左键按下：命中测试后按元素类型聚焦、放置 caret 或派发控件事件。
    ///
    /// `lparam` 是客户区物理像素坐标；场景几何是逻辑像素，按最近一次呈现的 scale 换算。
    fn pointer_pressed(&mut self, lparam: LPARAM) -> Result<(), HostError> {
        // 指针操作会移动焦点或 caret；先结束任何组合，避免 preedit 跟随到别处。
        self.cancel_compositions()?;
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
        let (scene, _) = self.current_scene(width, height)?;
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

    /// 一次 `WM_CHAR`：把可打印字符插入当前聚焦的文本输入；控制字符忽略。
    ///
    /// Windows 用两条 UTF-16 代理项消息发送非 BMP 字符；宿主缓存高代理项并在
    /// 低代理项到达时组合。
    fn char_typed(&mut self, wparam: WPARAM) -> Result<(), HostError> {
        let unit = wparam.0 as u16;
        let character = match unit {
            0xD800..=0xDBFF => {
                self.pending_high_surrogate = Some(unit);
                return Ok(());
            }
            0xDC00..=0xDFFF => {
                let Some(high) = self.pending_high_surrogate.take() else {
                    return Ok(());
                };
                char::decode_utf16([high, unit]).next().and_then(Result::ok)
            }
            _ => {
                self.pending_high_surrogate = None;
                char::from_u32(u32::from(unit))
            }
        };
        let Some(character) = character else {
            return Ok(());
        };
        if character.is_control() {
            return Ok(());
        }
        let snapshot = self.runtime.presentation().map_err(HostError::Runtime)?;
        let Some(focused) = focused_input(&snapshot) else {
            return Ok(());
        };
        let mut buffer = [0_u8; 4];
        pointer_dispatch(
            self.runtime
                .insert_text_node(focused, character.encode_utf8(&mut buffer)),
        )?;
        self.render()
    }

    /// 一次 `WM_KEYDOWN`：从线程消息队列读取修饰键后进入映射本体。
    fn key_down(&mut self, wparam: WPARAM) -> Result<(), HostError> {
        // 安全：GetKeyState 只查询本线程最近一条键盘消息的修饰键状态。
        let shift = unsafe { GetKeyState(i32::from(VK_SHIFT.0)) } < 0;
        let control = unsafe { GetKeyState(i32::from(VK_CONTROL.0)) } < 0;
        self.key_down_with(VIRTUAL_KEY(wparam.0 as u16), shift, control)
    }

    /// 键盘映射本体：编辑、caret/selection、焦点遍历与表单提交。
    ///
    /// Shift+方向键保留 anchor、移动焦点端；Ctrl+A 全选。
    fn key_down_with(
        &mut self,
        key: VIRTUAL_KEY,
        shift: bool,
        control: bool,
    ) -> Result<(), HostError> {
        if key == VK_TAB {
            self.cancel_compositions()?;
            if shift {
                self.runtime.focus_previous()
            } else {
                self.runtime.focus_next()
            }
            .map_err(HostError::Runtime)?;
            return self.render();
        }

        let snapshot = self.runtime.presentation().map_err(HostError::Runtime)?;
        let Some(focused) = focused_input(&snapshot) else {
            return Ok(());
        };
        if key == VK_ESCAPE {
            // 运行时在组合中优先取消并恢复文本；否则派发给用户 keydown。
            let composing = is_composing(&snapshot, focused);
            pointer_dispatch(self.runtime.keydown_node(focused, Key::Escape))?;
            if composing {
                self.notify_ime_cancel();
            }
            return self.render();
        }
        if control && key == VK_A {
            pointer_dispatch(self.runtime.select_all_node(focused))?;
            return self.render();
        }
        if key == VK_RETURN {
            if !is_composing(&snapshot, focused) {
                pointer_dispatch(self.runtime.keydown_node(focused, Key::Enter))?;
                if let Some(form) = ancestor_form(&snapshot, focused) {
                    pointer_dispatch(self.runtime.submit_node(form))?;
                }
            }
            return self.render();
        }
        if key == VK_BACK {
            pointer_dispatch(self.runtime.backspace_node(focused))?;
            return self.render();
        }
        if key == VK_DELETE {
            pointer_dispatch(self.runtime.delete_forward_node(focused))?;
            return self.render();
        }

        let Some(command) = caret_command_for(&snapshot, focused, key, shift) else {
            return Ok(());
        };
        match command {
            CaretCommand::Caret(offset) => {
                pointer_dispatch(self.runtime.set_caret_node(focused, offset))?;
            }
            CaretCommand::Selection(anchor, focus) => {
                pointer_dispatch(self.runtime.set_selection_node(focused, anchor, focus))?;
            }
        }
        self.render()
    }

    /// `WM_IME_STARTCOMPOSITION`：在当前聚焦输入上开始组合。
    fn ime_start(&mut self) -> Result<(), HostError> {
        let snapshot = self.runtime.presentation().map_err(HostError::Runtime)?;
        let Some(node) = focused_input(&snapshot) else {
            return Ok(());
        };
        if !is_composing(&snapshot, node) {
            pointer_dispatch(self.runtime.compose_start_node(node))?;
        }
        self.render()
    }

    /// `WM_IME_COMPOSITION`：结果串提交，组合串更新 preedit。
    fn ime_composition(&mut self, flags: LPARAM) -> Result<(), HostError> {
        let request = ime_request(flags.0 as u32);
        let snapshot = self.runtime.presentation().map_err(HostError::Runtime)?;
        let Some(node) = focused_input(&snapshot) else {
            return Ok(());
        };
        let commit = if request.commit {
            self.composition_string(GCS_RESULTSTR)?
        } else {
            None
        };
        let update = if request.update {
            self.composition_string(GCS_COMPSTR)?
        } else {
            None
        };
        self.apply_ime_composition(node, request, commit, update)?;
        self.render()
    }

    /// 把已取出的 IME 字符串应用到运行时；与 Win32 消息解耦以便测试。
    fn apply_ime_composition(
        &mut self,
        node: NodeId,
        request: ImeRequest,
        commit: Option<String>,
        update: Option<String>,
    ) -> Result<(), HostError> {
        let mut composing = self
            .runtime
            .presentation()
            .map_err(HostError::Runtime)?
            .node(node)
            .and_then(SnapshotElement::edit)
            .is_some_and(|edit| edit.composition().is_some());

        if request.commit {
            match commit {
                Some(text) => {
                    if !composing {
                        pointer_dispatch(self.runtime.compose_start_node(node))?;
                    }
                    pointer_dispatch(self.runtime.compose_commit_node(node, &text))?;
                }
                None if composing => {
                    pointer_dispatch(self.runtime.compose_cancel_node(node))?;
                }
                None => {}
            }
            composing = false;
        }
        if request.update {
            let text = update.unwrap_or_default();
            if !composing {
                pointer_dispatch(self.runtime.compose_start_node(node))?;
            }
            pointer_dispatch(self.runtime.compose_update_node(node, &text))?;
        }
        Ok(())
    }

    /// 取消未提交的组合；焦点丢失与 `WM_IME_ENDCOMPOSITION` 共用。
    fn ime_cancel(&mut self) -> Result<(), HostError> {
        if self.cancel_compositions()? {
            self.render()?;
        }
        Ok(())
    }

    /// 取消所有仍在组合的输入并通知 OS IME；返回是否取消了组合。
    ///
    /// 扫描全部输入而不是只看焦点，焦点已在宿主内移走或输入被禁用时也能清理。
    fn cancel_compositions(&mut self) -> Result<bool, HostError> {
        let snapshot = self.runtime.presentation().map_err(HostError::Runtime)?;
        let composing: Vec<NodeId> = snapshot
            .elements()
            .filter(|element| element.tag() == "text-input")
            .filter(|element| {
                element
                    .edit()
                    .is_some_and(|edit| edit.composition().is_some())
            })
            .map(SnapshotElement::id)
            .collect();
        let mut cancelled = false;
        for node in composing {
            match self.runtime.compose_cancel_node(node) {
                Ok(()) => cancelled = true,
                Err(error) => pointer_dispatch(Err(error))?,
            }
        }
        if cancelled {
            self.notify_ime_cancel();
        }
        Ok(cancelled)
    }

    fn notify_ime_cancel(&self) {
        // 安全：HIMC 由 ImmGetContext 获取并与 HWND 配对释放。
        unsafe {
            let himc = ImmGetContext(self.window.hwnd);
            if !himc.0.is_null() {
                let _ = ImmNotifyIME(himc, NI_COMPOSITIONSTR, CPS_CANCEL, 0);
                let _ = ImmReleaseContext(self.window.hwnd, himc);
            }
        }
    }

    fn composition_string(
        &self,
        kind: IME_COMPOSITION_STRING,
    ) -> Result<Option<String>, HostError> {
        // 安全：himc 由 ImmGetContext 返回，读取的缓冲在调用期间有效。
        let himc = unsafe { ImmGetContext(self.window.hwnd) };
        if himc.0.is_null() {
            return Ok(None);
        }
        let result: Result<Option<String>, HostError> = (|| {
            let bytes = unsafe { ImmGetCompositionStringW(himc, kind, None, 0) };
            if bytes <= 0 {
                return Ok(None);
            }
            let mut buffer = vec![0_u16; bytes as usize / 2];
            let written = unsafe {
                ImmGetCompositionStringW(himc, kind, Some(buffer.as_mut_ptr().cast()), bytes as u32)
            };
            if written <= 0 {
                return Ok(None);
            }
            buffer.truncate(written as usize / 2);
            String::from_utf16(&buffer).map(Some).map_err(|_| {
                HostError::Window(String::from("IME composition was not valid UTF-16"))
            })
        })();
        // 安全：himc 与获取它的 HWND 配对释放。
        unsafe {
            let _ = ImmReleaseContext(self.window.hwnd, himc);
        }
        result
    }

    /// 用 caret 的物理客户区坐标定位 OS 组合与候选窗口。
    ///
    /// `COMPOSITIONFORM` / `CANDIDATEFORM` 的坐标相对窗口左上角，不是屏幕坐标。
    fn position_ime(&self, snapshot: &PresentationSnapshot) {
        let Some(node) = focused_input(snapshot) else {
            return;
        };
        if !is_composing(snapshot, node) {
            return;
        }
        let Some(scene) = &self.scene else {
            return;
        };
        let Some(caret) = scene.caret(node) else {
            return;
        };
        let scale = scene.viewport().scale();
        let point = POINT {
            x: (caret.x * scale).round() as i32,
            y: (caret.y * scale).round() as i32,
        };
        // 安全：HIMC 由 ImmGetContext 获取，两个结构都是本地值。
        unsafe {
            let himc = ImmGetContext(self.window.hwnd);
            if himc.0.is_null() {
                return;
            }
            let composition = COMPOSITIONFORM {
                dwStyle: CFS_POINT,
                ptCurrentPos: point,
                rcArea: RECT::default(),
            };
            let candidate = CANDIDATEFORM {
                dwIndex: 0,
                dwStyle: CFS_CANDIDATEPOS,
                ptCurrentPos: point,
                rcArea: RECT::default(),
            };
            let _ = ImmSetCompositionWindow(himc, &composition);
            let _ = ImmSetCandidateWindow(himc, &candidate);
            let _ = ImmReleaseContext(self.window.hwnd, himc);
        }
    }

    /// 重建 device、surface 与 renderer；运行时状态（Node ID、焦点、caret）
    /// 留在 Runtime 中，因此恢复后保持不变。
    fn current_surface(&self) -> Result<&wgpu::Surface<'static>, HostError> {
        self.surface
            .as_ref()
            .ok_or_else(|| HostError::Surface(String::from("the surface is not available")))
    }

    fn rebuild_presentation(&mut self) -> Result<(), HostError> {
        // 释放旧 surface 的交换链后再配置新 surface。失败时保持 None，
        // 下一次重绘会再次尝试重建，而不是留下未配置的 surface。
        self.surface = None;
        let surface = create_surface(&self._instance, self.window.hwnd, module_handle()?)?;
        let presentation = configure_presentation(&self._instance, &surface, self.window.hwnd)?;
        self.surface = Some(surface);
        self.device = presentation.device;
        self.queue = presentation.queue;
        self.surface_config = presentation.config;
        self.renderer = presentation.renderer;
        Ok(())
    }

    fn ensure_surface(&mut self) -> Result<(), HostError> {
        let (width, height) = client_size(self.window.hwnd)?;
        if width == 0 || height == 0 {
            return Ok(());
        }
        if self.surface.is_none() {
            self.rebuild_presentation()?;
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
        self.current_surface()?
            .configure(&self.device, &self.surface_config);
        Ok(())
    }

    fn current_scene(
        &self,
        width: u32,
        height: u32,
    ) -> Result<(Scene, PresentationSnapshot), HostError> {
        // 安全：HWND 在 Host 生命周期内有效。
        let dpi = unsafe { GetDpiForWindow(self.window.hwnd) };
        let scale = if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 };
        let viewport = Viewport::new(width as f32 / scale, height as f32 / scale, scale)
            .map_err(HostError::Scene)?;
        let snapshot = self.runtime.presentation().map_err(HostError::Runtime)?;
        let scene = Scene::build(&snapshot, viewport).map_err(HostError::Scene)?;
        Ok((scene, snapshot))
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
        unsafe {
            AdjustWindowRectExForDpi(
                &mut rect,
                style,
                false,
                WINDOW_EX_STYLE::default(),
                dpi.max(96),
            )
        }
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

/// 一次导航键对 caret/selection 的目标；运行时的 selection 是归一化区间，
/// 因此 Shift+左键扩展左端、Shift+右键扩展右端。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaretCommand {
    Caret(usize),
    Selection(usize, usize),
}

/// 一条 `WM_IME_COMPOSITION` 同时可能带结果串与新组合串。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ImeRequest {
    commit: bool,
    update: bool,
}

fn ime_request(flags: u32) -> ImeRequest {
    ImeRequest {
        commit: flags & GCS_RESULTSTR.0 != 0,
        update: flags & GCS_COMPSTR.0 != 0,
    }
}

fn focused_input(snapshot: &PresentationSnapshot) -> Option<NodeId> {
    snapshot
        .focus()
        .and_then(|node| snapshot.node(node))
        .filter(|element| element.tag() == "text-input" && !element.disabled())
        .map(SnapshotElement::id)
}

fn is_composing(snapshot: &PresentationSnapshot, node: NodeId) -> bool {
    snapshot
        .node(node)
        .and_then(SnapshotElement::edit)
        .is_some_and(|edit| edit.composition().is_some())
}

fn caret_command_for(
    snapshot: &PresentationSnapshot,
    node: NodeId,
    key: VIRTUAL_KEY,
    shift: bool,
) -> Option<CaretCommand> {
    let element = snapshot.node(node)?;
    let edit = element.edit()?;
    let length = element.text_value().unwrap_or_default().chars().count();
    caret_command(edit, length, key, shift)
}

fn caret_command(
    edit: &TextEditState,
    length: usize,
    key: VIRTUAL_KEY,
    shift: bool,
) -> Option<CaretCommand> {
    let caret = edit.caret().min(length);
    let anchor = edit.anchor().min(length);
    // set_caret 会留下折叠的 (offset, offset)；只有非折叠区间才算选择。
    let selection = edit
        .selection()
        .filter(|(start, end)| start != end)
        .map(|(start, end)| (start.min(length), end.min(length)));
    if shift {
        return match key {
            k if k == VK_LEFT => Some(CaretCommand::Selection(anchor, caret.saturating_sub(1))),
            k if k == VK_RIGHT => Some(CaretCommand::Selection(anchor, (caret + 1).min(length))),
            k if k == VK_HOME => Some(CaretCommand::Selection(anchor, 0)),
            k if k == VK_END => Some(CaretCommand::Selection(anchor, length)),
            _ => None,
        };
    }
    let offset = match key {
        k if k == VK_LEFT => selection.map_or_else(|| caret.saturating_sub(1), |(start, _)| start),
        k if k == VK_RIGHT => selection.map_or_else(|| (caret + 1).min(length), |(_, end)| end),
        k if k == VK_HOME => 0,
        k if k == VK_END => length,
        _ => return None,
    };
    Some(CaretCommand::Caret(offset))
}

fn ancestor_form(snapshot: &PresentationSnapshot, node: NodeId) -> Option<NodeId> {
    fn walk(nodes: &[SnapshotNode], target: NodeId, form: Option<NodeId>) -> Option<NodeId> {
        for candidate in nodes {
            let SnapshotNode::Element(element) = candidate else {
                continue;
            };
            let form = if element.tag() == "form" {
                Some(element.id())
            } else {
                form
            };
            if element.id() == target {
                return form;
            }
            if let Some(found) = walk(element.children(), target, form) {
                return Some(found);
            }
        }
        None
    }
    walk(snapshot.roots(), node, None)
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

/// 从 HWND 创建 DX12 surface；重建时先换掉旧 surface 再配置。
fn create_surface(
    instance: &wgpu::Instance,
    hwnd: HWND,
    hinstance: HINSTANCE,
) -> Result<wgpu::Surface<'static>, HostError> {
    let raw_window = raw_window_handle(hwnd, hinstance)?;
    // 安全：HWND 由 Host 的 Window 持有，销毁推迟到 Window::drop；Host 的
    // surface 字段在 window 之前声明，因此 surface 先于 HWND 释放。
    unsafe {
        instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(raw_display_handle()),
            raw_window_handle: raw_window,
        })
    }
    .map_err(|error| HostError::Surface(error.to_string()))
}

/// 一次 surface 配置产生的 GPU 资源；首次打开与丢失恢复共用。
struct Presentation {
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: GpuRenderer,
}

/// 为已存在的 surface 请求适配器/设备并配置交换链。
fn configure_presentation(
    instance: &wgpu::Instance,
    surface: &wgpu::Surface<'_>,
    hwnd: HWND,
) -> Result<Presentation, HostError> {
    let (width, height) = client_size(hwnd)?;
    let (adapter, device, queue) = request_backend(instance, surface)?;
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
    let mut config = surface
        .get_default_config(&adapter, width.max(1), height.max(1))
        .ok_or_else(|| {
            HostError::Surface(String::from("the surface has no default configuration"))
        })?;
    config.format = format;
    config.present_mode = wgpu::PresentMode::Fifo;
    surface.configure(&device, &config);
    let renderer = new_renderer(&device, &queue, format, config.width, config.height)?;
    Ok(Presentation {
        device,
        queue,
        config,
        renderer,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resizing_relayouts_the_scene_and_surface() {
        let mut host = open_host();
        host.render().expect("render");
        let before = host.scene().expect("scene").viewport().width();

        host.resize(640, 480).expect("resize");
        let scene = host.scene().expect("scene");
        let scale = scene.viewport().scale();
        assert!(
            (scene.viewport().width() - 640.0).abs() < 2.0,
            "the viewport follows the new logical width"
        );
        assert!(
            (scene.viewport().width() * scale - 640.0 * scale).abs() < 2.0,
            "logical width times scale stays the physical width"
        );
        assert_eq!(
            scene.elements()[0].rect().width,
            scene.viewport().width(),
            "the root fills the resized viewport"
        );
        assert!(before > 0.0);

        let (width, height) = client_size(host.window.hwnd).expect("client size");
        let frame = host.read_frame().expect("frame after resize");
        assert_eq!((frame.width(), frame.height()), (width, height));
    }

    #[test]
    fn resizing_to_zero_renders_nothing_and_refuses_readback() {
        let mut host = open_host();
        host.resize(0, 0).expect("zero resize");
        assert!(
            matches!(host.read_frame(), Err(HostError::Readback(_))),
            "a zero client area cannot be read back"
        );
        host.resize(320, 240).expect("restore size");
        let frame = host.read_frame().expect("frame after restore");
        assert!(frame.width() > 0 && frame.height() > 0);
    }

    #[test]
    fn a_surface_rebuild_preserves_focus_and_caret() {
        let mut host = open_host();
        host.runtime.focus("Edit Write tests").expect("focus");
        let node = editor_node(&host);
        host.runtime
            .set_selection_node(node, 1, 3)
            .expect("selection");
        host.render().expect("render");
        let ids: Vec<_> = host
            .scene()
            .expect("scene")
            .elements()
            .iter()
            .map(|element| element.id())
            .collect();
        let before = host.read_frame().expect("frame");
        let caret_before = host.scene().expect("scene").caret(node).expect("caret").x;

        host.rebuild_presentation().expect("rebuild");
        host.render().expect("render after rebuild");

        let scene = host.scene().expect("scene after rebuild");
        assert_eq!(
            scene
                .elements()
                .iter()
                .map(|element| element.id())
                .collect::<Vec<_>>(),
            ids,
            "Node IDs survive the rebuild"
        );
        let snapshot = host.runtime.presentation().expect("snapshot");
        assert_eq!(snapshot.focus(), Some(node));
        let edit = snapshot.node(node).expect("editor").edit().expect("edit");
        assert_eq!(edit.selection(), Some((1, 3)));
        assert_eq!(edit.anchor(), 1);
        assert_eq!(edit.caret(), 3);
        let caret_after = scene.caret(node).expect("caret").x;
        assert_eq!(caret_before, caret_after);
        let after = host.read_frame().expect("frame after rebuild");
        assert_eq!(before.pixels(), after.pixels());
    }

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

    fn edit_state(caret: usize, selection: Option<(usize, usize)>) -> (Runtime, NodeId) {
        let mut app = Runtime::new();
        app.open_window("main", |_| {
            view! { <text-input value="Alpha" aria:label="Input" /> }
        })
        .expect("open window");
        let id = app
            .presentation()
            .expect("snapshot")
            .find_label("Input")
            .expect("input")
            .id();
        app.focus_node(id).expect("focus");
        match selection {
            Some((start, end)) => app.set_selection_node(id, start, end).expect("selection"),
            None => app.set_caret_node(id, caret).expect("caret"),
        }
        (app, id)
    }

    fn command(
        caret: usize,
        selection: Option<(usize, usize)>,
        key: VIRTUAL_KEY,
        shift: bool,
    ) -> Option<CaretCommand> {
        let (app, id) = edit_state(caret, selection);
        let snapshot = app.presentation().expect("snapshot");
        let element = snapshot.node(id).expect("input");
        let edit = element.edit().expect("edit");
        let length = element.text_value().unwrap_or_default().chars().count();
        caret_command(edit, length, key, shift)
    }

    #[test]
    fn caret_commands_step_collapse_and_clamp() {
        assert_eq!(
            command(3, None, VK_LEFT, false),
            Some(CaretCommand::Caret(2))
        );
        assert_eq!(
            command(3, None, VK_RIGHT, false),
            Some(CaretCommand::Caret(4))
        );
        assert_eq!(
            command(0, None, VK_LEFT, false),
            Some(CaretCommand::Caret(0)),
            "left clamps at the start"
        );
        assert_eq!(
            command(5, None, VK_RIGHT, false),
            Some(CaretCommand::Caret(5)),
            "right clamps at the end"
        );
        assert_eq!(
            command(3, Some((1, 3)), VK_LEFT, false),
            Some(CaretCommand::Caret(1)),
            "left collapses the selection to its left edge"
        );
        assert_eq!(
            command(3, Some((1, 3)), VK_RIGHT, false),
            Some(CaretCommand::Caret(3)),
            "right collapses the selection to its right edge"
        );
        assert_eq!(
            command(3, None, VK_HOME, false),
            Some(CaretCommand::Caret(0))
        );
        assert_eq!(
            command(3, None, VK_END, false),
            Some(CaretCommand::Caret(5))
        );
        assert_eq!(command(3, None, VK_RETURN, false), None);
    }

    #[test]
    fn shift_caret_commands_keep_the_anchor_and_move_the_focus() {
        assert_eq!(
            command(3, None, VK_LEFT, true),
            Some(CaretCommand::Selection(3, 2))
        );
        assert_eq!(
            command(3, None, VK_RIGHT, true),
            Some(CaretCommand::Selection(3, 4))
        );
        assert_eq!(
            command(3, Some((1, 3)), VK_LEFT, true),
            Some(CaretCommand::Selection(1, 2)),
            "shift-left shrinks toward the anchor"
        );
        assert_eq!(
            command(3, Some((1, 3)), VK_RIGHT, true),
            Some(CaretCommand::Selection(1, 4)),
            "shift-right extends away from the anchor"
        );
        assert_eq!(
            command(3, Some((1, 3)), VK_HOME, true),
            Some(CaretCommand::Selection(1, 0))
        );
        assert_eq!(
            command(3, Some((1, 3)), VK_END, true),
            Some(CaretCommand::Selection(1, 5))
        );
        assert_eq!(
            command(0, None, VK_LEFT, true),
            Some(CaretCommand::Selection(0, 0)),
            "shift-left at the start stays collapsed"
        );
    }

    #[test]
    fn a_backward_selection_keeps_its_direction() {
        assert_eq!(
            command(1, Some((3, 1)), VK_LEFT, true),
            Some(CaretCommand::Selection(3, 0)),
            "the anchor stays at 3 and the focus moves left"
        );
        assert_eq!(
            command(1, Some((3, 1)), VK_RIGHT, true),
            Some(CaretCommand::Selection(3, 2))
        );
        assert_eq!(
            command(1, Some((3, 1)), VK_LEFT, false),
            Some(CaretCommand::Caret(1)),
            "plain left collapses to the normalized left edge"
        );
    }

    fn open_host() -> Host {
        Host::open(HostConfig::hidden("ZenUI host keyboard")).expect("open host")
    }

    fn editor_node(host: &Host) -> NodeId {
        host.runtime
            .presentation()
            .expect("snapshot")
            .find_label("Edit Write tests")
            .expect("editor")
            .id()
    }

    #[test]
    fn ctrl_a_selects_all_and_shift_left_shrinks_from_the_right() {
        let mut host = open_host();
        host.runtime.focus("Edit Write tests").expect("focus");
        let node = editor_node(&host);

        host.key_down_with(VK_A, false, true).expect("select all");
        let snapshot = host.runtime.presentation().expect("snapshot");
        let edit = snapshot.node(node).expect("editor").edit().expect("edit");
        assert_eq!(edit.selection(), Some((0, 11)));
        assert_eq!(edit.anchor(), 0);
        assert_eq!(edit.caret(), 11);

        host.key_down_with(VK_LEFT, true, false).expect("shrink");
        let snapshot = host.runtime.presentation().expect("snapshot");
        let edit = snapshot.node(node).expect("editor").edit().expect("edit");
        assert_eq!(edit.selection(), Some((0, 10)));
        assert_eq!(edit.anchor(), 0);
        assert_eq!(edit.caret(), 10);
    }

    #[test]
    fn typing_replaces_the_selection() {
        let mut host = open_host();
        host.runtime.focus("Edit Write tests").expect("focus");
        let node = editor_node(&host);

        host.key_down_with(VK_A, false, true).expect("select all");
        host.char_typed(WPARAM('X' as usize)).expect("type");
        let snapshot = host.runtime.presentation().expect("snapshot");
        let element = snapshot.node(node).expect("editor");
        assert_eq!(element.text_value(), Some("X"));
        let edit = element.edit().expect("edit");
        assert_eq!(edit.caret(), 1);
        assert_eq!(edit.selection(), None, "typing collapses the selection");
    }

    #[test]
    fn shift_tab_moves_focus_backward() {
        let mut host = open_host();
        host.runtime.focus("Edit Write tests").expect("focus");
        let before = host.runtime.presentation().expect("snapshot").focus();
        host.key_down_with(VK_TAB, true, false).expect("shift tab");
        let after = host.runtime.presentation().expect("snapshot").focus();
        assert!(after.is_some());
        assert_ne!(after, before, "Shift+Tab walks the focus order backwards");
    }

    #[test]
    fn enter_is_ignored_while_composing() {
        let mut host = open_host();
        host.runtime.focus("Edit Write tests").expect("focus");
        let node = editor_node(&host);
        host.runtime.compose_start_node(node).expect("start");
        host.runtime
            .compose_update_node(node, "ni")
            .expect("update");

        host.key_down_with(VK_RETURN, false, false)
            .expect("press enter");
        let snapshot = host.runtime.presentation().expect("snapshot");
        assert_eq!(
            snapshot
                .elements()
                .filter(|element| element.tag() == "checkbox")
                .count(),
            2,
            "Enter during composition does not submit"
        );
        assert!(
            snapshot
                .node(node)
                .expect("editor")
                .edit()
                .expect("edit")
                .composition()
                .is_some(),
            "the composition survives"
        );
    }

    #[test]
    fn control_characters_and_stray_surrogates_do_not_insert() {
        let mut host = open_host();
        host.runtime.focus("Edit Write tests").expect("focus");
        let node = editor_node(&host);

        host.char_typed(WPARAM(0x0009)).expect("tab character");
        host.char_typed(WPARAM(0xDC00))
            .expect("stray low surrogate");
        let snapshot = host.runtime.presentation().expect("snapshot");
        assert_eq!(
            snapshot.node(node).expect("editor").text_value(),
            Some("Write tests"),
            "control characters and stray surrogates are ignored"
        );
    }

    #[test]
    fn surrogate_pairs_insert_one_non_bmp_character() {
        let mut host = open_host();
        host.runtime.focus("Edit Write tests").expect("focus");
        let node = editor_node(&host);

        host.char_typed(WPARAM(0xD83D)).expect("high surrogate");
        host.char_typed(WPARAM(0xDE42)).expect("low surrogate");
        let snapshot = host.runtime.presentation().expect("snapshot");
        assert!(
            snapshot
                .node(node)
                .expect("editor")
                .text_value()
                .expect("value")
                .ends_with('🙂'),
            "the surrogate pair becomes one character"
        );
    }

    fn composing_host() -> (Host, NodeId) {
        let mut host = open_host();
        host.runtime.focus("Edit Write tests").expect("focus");
        let node = editor_node(&host);
        host.ime_start().expect("start composition");
        (host, node)
    }

    fn request(commit: bool, update: bool) -> ImeRequest {
        ImeRequest { commit, update }
    }

    #[test]
    fn ime_flags_recognize_the_composition_string() {
        assert_eq!(ime_request(0), request(false, false));
        assert_eq!(ime_request(GCS_COMPSTR.0), request(false, true));
    }

    #[test]
    fn ime_flags_recognize_the_result_string() {
        assert_eq!(ime_request(GCS_RESULTSTR.0), request(true, false));
        assert_eq!(
            ime_request(GCS_COMPSTR.0 | GCS_RESULTSTR.0),
            request(true, true)
        );
    }

    #[test]
    fn ime_update_shows_preedit_without_writing_the_model() {
        let (mut host, node) = composing_host();
        host.apply_ime_composition(node, request(false, true), None, Some("ni".into()))
            .expect("apply update");
        let snapshot = host.runtime.presentation().expect("snapshot");
        let element = snapshot.node(node).expect("editor");
        assert_eq!(element.text_value(), Some("Write tests"));
        let edit = element.edit().expect("edit");
        assert_eq!(edit.preedit(), Some("ni"));
        assert!(edit.composition().is_some());
    }

    #[test]
    fn ime_commit_inserts_the_result() {
        let (mut host, node) = composing_host();
        host.apply_ime_composition(node, request(true, false), Some("你".into()), None)
            .expect("apply commit");
        let snapshot = host.runtime.presentation().expect("snapshot");
        let element = snapshot.node(node).expect("editor");
        assert_eq!(element.text_value(), Some("Write tests你"));
        assert!(element.edit().expect("edit").composition().is_none());
    }

    #[test]
    fn ime_commit_then_update_starts_the_next_composition() {
        let (mut host, node) = composing_host();
        host.apply_ime_composition(
            node,
            request(true, true),
            Some("你".into()),
            Some("ni".into()),
        )
        .expect("apply commit and update");
        let snapshot = host.runtime.presentation().expect("snapshot");
        let element = snapshot.node(node).expect("editor");
        assert!(element.text_value().expect("value").ends_with('你'));
        let edit = element.edit().expect("edit");
        assert_eq!(edit.preedit(), Some("ni"));
        assert!(edit.composition().is_some());
    }

    #[test]
    fn an_empty_result_cancels_the_composition() {
        let (mut host, node) = composing_host();
        host.apply_ime_composition(node, request(true, false), None, None)
            .expect("empty result");
        let snapshot = host.runtime.presentation().expect("snapshot");
        let element = snapshot.node(node).expect("editor");
        assert_eq!(element.text_value(), Some("Write tests"));
        assert!(element.edit().expect("edit").composition().is_none());
    }

    #[test]
    fn escape_cancels_composition_without_writing_the_model() {
        let (mut host, node) = composing_host();
        host.runtime
            .compose_update_node(node, "ni")
            .expect("preedit");

        host.key_down_with(VK_ESCAPE, false, false).expect("escape");
        let snapshot = host.runtime.presentation().expect("snapshot");
        let element = snapshot.node(node).expect("editor");
        assert_eq!(element.text_value(), Some("Write tests"));
        assert!(
            element.edit().expect("edit").composition().is_none(),
            "Escape ends the composition"
        );
    }

    #[test]
    fn wm_killfocus_cancels_composition() {
        let (mut host, node) = composing_host();
        host.runtime
            .compose_update_node(node, "ni")
            .expect("preedit");
        let message = MSG {
            message: WM_KILLFOCUS,
            ..MSG::default()
        };
        assert!(
            !host.process_message(&message).expect("handle message"),
            "focus loss does not close the window"
        );
        let snapshot = host.runtime.presentation().expect("snapshot");
        let element = snapshot.node(node).expect("editor");
        assert_eq!(element.text_value(), Some("Write tests"));
        assert!(element.edit().expect("edit").composition().is_none());
    }

    #[test]
    fn ime_without_focus_is_a_no_op() {
        let mut host = open_host();
        host.ime_start().expect("no focus");
        host.ime_composition(LPARAM(GCS_COMPSTR.0 as isize))
            .expect("no focus");
        host.ime_cancel().expect("no focus");
    }

    #[test]
    fn ime_message_routing_starts_and_ends_composition() {
        let mut host = open_host();
        host.runtime.focus("Edit Write tests").expect("focus");
        let node = editor_node(&host);

        let start = MSG {
            message: WM_IME_STARTCOMPOSITION,
            ..MSG::default()
        };
        host.process_message(&start).expect("start message");
        let snapshot = host.runtime.presentation().expect("snapshot");
        assert!(
            snapshot
                .node(node)
                .expect("editor")
                .edit()
                .expect("edit")
                .composition()
                .is_some(),
            "WM_IME_STARTCOMPOSITION starts the composition"
        );

        let end = MSG {
            message: WM_IME_ENDCOMPOSITION,
            ..MSG::default()
        };
        host.process_message(&end).expect("end message");
        let snapshot = host.runtime.presentation().expect("snapshot");
        assert!(
            snapshot
                .node(node)
                .expect("editor")
                .edit()
                .expect("edit")
                .composition()
                .is_none(),
            "WM_IME_ENDCOMPOSITION cancels the composition"
        );
    }

    #[test]
    fn ime_composition_without_an_os_string_keeps_an_empty_composition() {
        let mut host = open_host();
        host.runtime.focus("Edit Write tests").expect("focus");
        let node = editor_node(&host);
        let message = MSG {
            message: WM_IME_COMPOSITION,
            lParam: LPARAM(GCS_COMPSTR.0 as isize),
            ..MSG::default()
        };
        host.process_message(&message).expect("composition message");
        let snapshot = host.runtime.presentation().expect("snapshot");
        let element = snapshot.node(node).expect("editor");
        assert_eq!(element.text_value(), Some("Write tests"));
        assert!(
            element.edit().expect("edit").composition().is_some(),
            "the update path starts a composition even with an empty string"
        );
    }

    #[test]
    fn ime_char_is_ignored_to_avoid_double_commits() {
        let mut host = open_host();
        host.runtime.focus("Edit Write tests").expect("focus");
        let node = editor_node(&host);
        let message = MSG {
            message: WM_IME_CHAR,
            wParam: WPARAM('X' as usize),
            ..MSG::default()
        };
        host.process_message(&message).expect("IME char");
        let snapshot = host.runtime.presentation().expect("snapshot");
        assert_eq!(
            snapshot.node(node).expect("editor").text_value(),
            Some("Write tests"),
            "WM_IME_CHAR does not insert a second copy"
        );
    }

    #[test]
    fn tab_cancels_the_composition_before_moving_focus() {
        let (mut host, node) = composing_host();
        host.runtime
            .compose_update_node(node, "ni")
            .expect("preedit");
        host.key_down_with(VK_TAB, false, false).expect("tab");
        let snapshot = host.runtime.presentation().expect("snapshot");
        let element = snapshot.node(node).expect("editor");
        assert_eq!(element.text_value(), Some("Write tests"));
        assert!(element.edit().expect("edit").composition().is_none());
    }

    #[test]
    fn pointer_input_cancels_the_composition() {
        let (mut host, node) = composing_host();
        host.runtime
            .compose_update_node(node, "ni")
            .expect("preedit");
        host.pointer_pressed(LPARAM(0)).expect("click blank space");
        let snapshot = host.runtime.presentation().expect("snapshot");
        assert!(
            snapshot
                .node(node)
                .expect("editor")
                .edit()
                .expect("edit")
                .composition()
                .is_none()
        );
    }

    #[test]
    fn ancestor_form_finds_the_nearest_enclosing_form() {
        let mut app = Runtime::new();
        app.open_window("main", |_| {
            view! {
                <view>
                    <form aria:label="Form">
                        <text-input value="a" aria:label="Inside" />
                    </form>
                    <text-input value="b" aria:label="Outside" />
                </view>
            }
        })
        .expect("open window");
        let snapshot = app.presentation().expect("snapshot");
        let inside = snapshot.find_label("Inside").expect("inside").id();
        let outside = snapshot.find_label("Outside").expect("outside").id();
        let form = snapshot
            .elements()
            .find(|element| element.tag() == "form")
            .expect("form")
            .id();
        assert_eq!(ancestor_form(&snapshot, inside), Some(form));
        assert_eq!(ancestor_form(&snapshot, outside), None);
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
