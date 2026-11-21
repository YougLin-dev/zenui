use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::num::NonZeroIsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use windows::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE,
    WPARAM,
};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT, ValidateRect};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
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
    GetCapture, GetKeyState, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT,
    TrackMouseEvent, VIRTUAL_KEY, VK_A, VK_BACK, VK_CONTROL, VK_DELETE, VK_END, VK_ESCAPE, VK_HOME,
    VK_LEFT, VK_PROCESSKEY, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, GWLP_USERDATA, GetClientRect, GetForegroundWindow, GetMessageW,
    GetWindowLongPtrW, IDC_ARROW, IsIconic, IsZoomed, KillTimer, LoadCursorW, MSG, PM_REMOVE,
    PeekMessageW, PostMessageW, RegisterClassExW, SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOZORDER, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage,
    WINDOW_EX_STYLE, WM_APP, WM_CAPTURECHANGED, WM_CHAR, WM_CLOSE, WM_DESTROY, WM_DPICHANGED,
    WM_GETDPISCALEDSIZE, WM_IME_CHAR, WM_IME_COMPOSITION, WM_IME_ENDCOMPOSITION,
    WM_IME_STARTCOMPOSITION, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_PAINT, WM_SIZE, WM_TIMER, WNDCLASSEXW, WS_OVERLAPPEDWINDOW,
};

use windows::core::{HSTRING, w};
use zenui::prelude::*;
use zenui_render::{
    GpuRenderer, Point, RenderPalette, RenderTargetConfig, Scene, SceneElement, SceneEnv, Viewport,
    Visual, wgpu,
};

/// worker 唤醒只投递这一条消息；`WM_APP` 起是应用私有的消息区间。
const HOST_WAKE_MESSAGE: u32 = WM_APP + 1;

/// caret 闪烁定时器；OS 原生编辑控件的缺省周期约为 530ms。
const CARET_BLINK_TIMER: usize = 1;
const CARET_BLINK_MS: u32 = 530;

/// 打开宿主的进程级选项；每个窗口的标题与尺寸来自窗口目录。
#[derive(Clone, Copy, Debug)]
pub struct HostConfig {
    /// 是否在 `run` 时显示已打开的窗口；隐藏模式供 CI smoke 使用。
    pub visible: bool,
    /// 应用的初始外观。
    pub appearance: Appearance,
}

impl HostConfig {
    /// 显示所有已打开的窗口。
    pub const fn visible() -> Self {
        Self {
            visible: true,
            appearance: Appearance::Light,
        }
    }

    /// 创建窗口与 surface，但不调用 `ShowWindow`；供隐藏 smoke 使用。
    pub const fn hidden() -> Self {
        Self {
            visible: false,
            appearance: Appearance::Light,
        }
    }

    /// 设置初始应用外观。
    pub const fn with_appearance(mut self, appearance: Appearance) -> Self {
        self.appearance = appearance;
        self
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

/// worker 与 UI 线程共享的唤醒状态。
///
/// 至多一条消息在途：第一个唤醒登记 `pending` 并向当前 `target` 投递，其余
/// 唤醒合并；关闭后不再投递。runtime 只有一个唤醒回调，所以合并标志与投递
/// 目标属于宿主，而不是某个窗口；窗口增删时宿主更新 `target`。
struct WakeState {
    pending: AtomicBool,
    closed: AtomicBool,
    /// 当前接收 `WM_APP + 1` 的 HWND 值；零表示没有可投递的窗口。
    target: AtomicIsize,
}

impl WakeState {
    fn new() -> Self {
        Self {
            pending: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            target: AtomicIsize::new(0),
        }
    }

    /// 若没有在途唤醒则登记并返回 `true`；否则本次唤醒合并进已有消息。
    fn claim(&self) -> bool {
        !self.pending.swap(true, Ordering::SeqCst)
    }

    /// 宿主已处理或放弃这条唤醒，下一次唤醒需要重新投递。
    fn observed(&self) {
        self.pending.store(false, Ordering::SeqCst);
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    /// 更新唤醒目标；返回目标是否真的改变。
    fn set_target(&self, hwnd: Option<HWND>) -> bool {
        let previous = self
            .target
            .swap(hwnd.map_or(0, |hwnd| hwnd.0 as isize), Ordering::SeqCst);
        previous != self.target.load(Ordering::SeqCst)
    }

    fn is_pending(&self) -> bool {
        self.pending.load(Ordering::SeqCst)
    }

    fn target(&self) -> Option<HWND> {
        let value = self.target.load(Ordering::SeqCst);
        (value != 0).then_some(HWND(value as *mut core::ffi::c_void))
    }
}

/// 一个窗口的全部宿主状态；`Host` 为每个存活窗口持有一个条目。
struct WindowHost {
    // 字段顺序是安全约束：surface 必须先于 HWND 释放，因为它的安全契约
    // 要求创建 surface 的 HWND 在 surface 存活期间保持有效。
    surface: Option<wgpu::Surface<'static>>,
    surface_config: wgpu::SurfaceConfiguration,
    native: NativeWindow,
    ctx: WindowCtx,
    scene: Option<Scene>,
    pointer: Option<Point>,
    hovered: Option<NodeId>,
    pressed: Option<NodeId>,
    release_capture: bool,
    tracking_leave: bool,
    keyboard_modality: bool,
    pending_high_surrogate: Option<u16>,
    caret_visible: bool,
    blink_timer: Option<usize>,
    close_requested: bool,
    /// 指针、按压、模态或 caret 等瞬时状态变化需要重绘；它们不进入快照。
    paint_requested: bool,
    /// 最近一次构建的呈现快照；未变化的窗口跳过重绘。
    ///
    /// 场景构建发生在获取 surface 图像之前，所以 `Timeout` / `Occluded` 时
    /// 这里可能比屏幕上的帧更新。
    last_snapshot: Option<PresentationSnapshot>,
}

impl WindowHost {
    const fn hwnd(&self) -> HWND {
        self.native.hwnd
    }
}

/// Win32 宿主：一个 Runtime、一个消息循环，每个存活窗口一个 HWND/Surface/Scene。
pub struct Host {
    runtime: Runtime,
    _instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: GpuRenderer,
    renderer_target: RenderTargetConfig,
    format: wgpu::TextureFormat,
    hinstance: HINSTANCE,
    /// runtime 只安装一个唤醒回调；合并标志与当前投递目标都属于整条交付通道。
    wake: Arc<WakeState>,
    windows: BTreeMap<&'static str, WindowHost>,
    /// HWND 值到目录键的路由表；窗口销毁时同步移除。
    hwnds: BTreeMap<isize, &'static str>,
    /// 最近一次同步看到的窗口修订号。
    revision: u64,
    /// 隐藏模式只创建 native 窗口，不调用 `ShowWindow`。
    show_windows: bool,
    /// 仅在 `run`/`pump` 派发期间指向 self；窗口过程通过 GWLP_USERDATA 取回宿主。
    dispatching: Option<*mut Host>,
    /// 设备丢失后的一次恢复正在进行，避免递归重建。
    recovering: bool,
    /// 渲染时发现 surface 丢失；消息处理结束后统一重建所有窗口。
    device_lost: bool,
    /// 窗口过程不能返回错误；消息循环在派发后取走并返回。
    deferred_error: Option<HostError>,
}

impl Host {
    /// 从一个已声明窗口目录的 Runtime 创建宿主；primary 未打开时由宿主首次显示。
    ///
    /// 每个存活窗口创建一个 HWND 与 DX12 surface；device、queue 与 renderer
    /// 在窗口之间共享。优先硬件适配器，失败时回退 WARP。
    pub fn open(mut runtime: Runtime, config: HostConfig) -> Result<Self, HostError> {
        // 安全：进程级 DPI 设置；重复设置失败时保持已有策略。
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        let primary = runtime
            .primary_key()
            .ok_or_else(|| HostError::Window(String::from("the window catalog is not declared")))?;
        runtime
            .set_appearance(config.appearance)
            .map_err(HostError::Runtime)?;
        // primary 由宿主实现首次打开；已打开时只聚焦。
        runtime.show(primary).map_err(HostError::Open)?;
        let hinstance = module_handle()?;
        register_class(hinstance)?;
        let infos = runtime.windows();
        let primary_info = infos
            .iter()
            .find(|info| info.spec.key() == primary)
            .copied()
            .ok_or_else(|| {
                HostError::Window(String::from("the primary window is not live after show"))
            })?;

        let window = NativeWindow::new(primary_info.spec, hinstance)?;
        let hwnd = window.hwnd;
        // 安全：HWND 在窗口条目析构前有效；定时器随窗口销毁。
        let blink_timer = unsafe { SetTimer(Some(hwnd), CARET_BLINK_TIMER, CARET_BLINK_MS, None) };
        let blink_timer = (blink_timer != 0).then_some(blink_timer);

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12,
            // DirectComposition 视觉承载交换链：DWM 合成视觉内容而不是缩放
            // 上一帧，窗口缩放（尤其快速拖动）时文字不会先拉伸再回弹。
            // 代价是 RenderDoc 无法抓取这类交换链。
            backend_options: wgpu::BackendOptions {
                dx12: wgpu::Dx12BackendOptions {
                    presentation_system: wgpu::Dx12SwapchainKind::DxgiFromVisual,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = create_surface(&instance, hwnd, hinstance)?;
        let (adapter, device, queue) = request_backend(&instance, &surface)?;
        let surface_config = surface_configuration(&adapter, &surface, hwnd)?;
        surface.configure(&device, &surface_config);
        let format = surface_config.format;
        let target = target_config(format, surface_config.width, surface_config.height)?;
        let renderer = GpuRenderer::from_parts(&device, &queue, target, RenderPalette::default());

        // worker 只向当前存在的一个 HWND 投递一条合并后的消息；runtime 访问留在
        // UI 线程，关闭标志阻止宿主析构后的迟到投递。
        let wake = Arc::new(WakeState::new());
        // 宿主的第一个窗口已经创建；先给出投递目标，setup 期间完成的 worker
        // 也不会因为 target 为空而丢掉唯一一次唤醒。
        wake.set_target(Some(hwnd));
        {
            let wake = Arc::clone(&wake);
            runtime.set_wake_callback(move || {
                if wake.is_closed() || !wake.claim() {
                    return;
                }
                let Some(target) = wake.target() else {
                    wake.observed();
                    return;
                };
                // 安全：PostMessageW 是线程安全的；关闭标志尽量阻止窗口销毁后
                // 的新投递，竞态下已销毁窗口的句柄值只让调用返回错误，不会解引用。
                if unsafe { PostMessageW(Some(target), HOST_WAKE_MESSAGE, WPARAM(0), LPARAM(0)) }
                    .is_err()
                {
                    // 投递失败说明窗口已不能接收唤醒；释放合并标志，
                    // 避免之后的唤醒被永久吞掉。
                    wake.observed();
                }
            });
        }

        let mut host = Self {
            runtime,
            _instance: instance,
            adapter,
            device,
            queue,
            renderer,
            renderer_target: target,
            format,
            hinstance,
            wake,
            windows: BTreeMap::new(),
            hwnds: BTreeMap::new(),
            revision: 0,
            show_windows: config.visible,
            dispatching: None,
            recovering: false,
            device_lost: false,
            deferred_error: None,
        };
        host.insert_window(WindowHost::new(
            window,
            primary_info.window,
            surface,
            surface_config,
            blink_timer,
        ));
        for info in &infos {
            if info.spec.key() != primary {
                host.add_window(info)?;
            }
        }
        host.revision = host.runtime.window_revision();
        host.update_wake_target();
        Ok(host)
    }

    /// 指定窗口的 HWND；窗口当前没有原生实例时返回 `None`。
    pub fn hwnd(&self, key: &str) -> Option<HWND> {
        self.windows.get(key).map(WindowHost::hwnd)
    }

    /// 指定窗口最近一次呈现的场景；指针命中与几何查询使用。
    pub fn scene(&self, key: &str) -> Option<&Scene> {
        self.windows.get(key)?.scene.as_ref()
    }

    /// 显示目录窗口：未打开时挂载并创建 HWND；已打开时聚焦或从隐藏恢复。
    pub fn show(&mut self, key: &'static str) -> Result<WindowCtx, HostError> {
        let window = self.runtime.show(key).map_err(HostError::Open)?;
        self.sync_windows()?;
        self.repaint_changed_windows()?;
        Ok(window)
    }

    /// 隐藏窗口：保留 activation 与本地状态，只隐藏原生窗口并停止输入/呈现。
    pub fn hide(&mut self, key: &str) -> Result<(), HostError> {
        let ctx = self
            .windows
            .get(key)
            .map(|window| window.ctx)
            .ok_or_else(|| HostError::UnknownWindow(key.to_owned()))?;
        self.runtime.hide(ctx).map_err(HostError::Close)?;
        self.sync_windows()?;
        self.repaint_changed_windows()
    }

    /// 关闭窗口并释放它的 activation；其他窗口不受影响。
    pub fn close(&mut self, key: &str) -> Result<(), HostError> {
        let ctx = self
            .windows
            .get(key)
            .map(|window| window.ctx)
            .ok_or_else(|| HostError::UnknownWindow(key.to_owned()))?;
        self.runtime.close_window(ctx).map_err(HostError::Close)?;
        self.sync_windows()?;
        self.repaint_changed_windows()
    }

    /// 投递一条 `WM_CLOSE`；消息循环在派发后同步原生窗口。
    pub fn request_close(&self, key: &str) -> Result<(), HostError> {
        let hwnd = self
            .hwnd(key)
            .ok_or_else(|| HostError::UnknownWindow(key.to_owned()))?;
        // 安全：HWND 由窗口条目持有；消息只排队，不跨线程解引用。
        unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }
            .map_err(|error| HostError::Window(error.to_string()))
    }

    /// 显示所有打开的窗口（若配置为可见）并进入消息循环，直到最后一个窗口关闭。
    pub fn run(&mut self) -> Result<(), HostError> {
        self.sync_windows()?;
        if self.runtime.windows().is_empty() {
            return Ok(());
        }
        self.render()?;
        // 窗口过程要能在系统自己的拖动/缩放循环里重绘；指针只在循环期间安装。
        let host = self as *mut Self;
        let _pointer = HostPointer::install(host);
        // 安全：`host` 指向 `self`，地址在借用的 `run` 期间稳定；循环只通过
        // 裸指针访问宿主，派发消息时不持有引用。
        unsafe { run_loop(host) }
    }

    /// 处理所有已排队的窗口消息后立即返回；返回是否已没有存活窗口。
    ///
    /// 把宿主嵌进别的消息循环，或在测试里驱动一次输入而不阻塞时使用。
    pub fn pump(&mut self) -> Result<bool, HostError> {
        self.sync_windows()?;
        let host = self as *mut Self;
        let _pointer = HostPointer::install(host);
        // 安全：与 `run` 相同。
        unsafe { pump_loop(host) }
    }

    /// 处理一条已经派发到窗口过程的消息。
    ///
    /// 输入处理放在窗口过程而不是消息循环里，因为系统的模态拖动/缩放循环会
    /// 直接调用窗口过程，消息循环在此期间收不到任何消息。唤醒消息是宿主级
    /// 的：任一 HWND 收到它都交付全部 worker 结果，再逐窗口比较快照重绘。
    fn handle_message(&mut self, hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) {
        if message == HOST_WAKE_MESSAGE {
            if let Err(error) = self.deliver_tasks() {
                self.deferred_error.get_or_insert(error);
            }
            return;
        }
        let Some(key) = self.key_for(hwnd) else {
            return;
        };
        let result = self.with_window(key, |host, window| {
            host.handle_window_message(window, message, wparam, lparam)
        });
        if let Err(error) = result {
            self.deferred_error.get_or_insert(error);
        }
        if let Err(error) = self.repaint_changed_windows() {
            self.deferred_error.get_or_insert(error);
        }
    }

    /// 一条指向某个窗口的消息；调用方已按 HWND 路由。
    fn handle_window_message(
        &mut self,
        window: &mut WindowHost,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Result<(), HostError> {
        if window.close_requested || !self.runtime.window_visible(window.ctx) {
            // 关闭或隐藏的窗口不接受输入；迟到的唤醒由宿主级路径丢弃结果。
            return Ok(());
        }
        // 任何输入都让 caret 立即可见，之后由闪烁定时器继续。
        if matches!(
            message,
            WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MOUSEMOVE | WM_CHAR | WM_KEYDOWN
        ) && !window.caret_visible
        {
            window.caret_visible = true;
            window.paint_requested = true;
        }
        if matches!(message, WM_CHAR | WM_KEYDOWN) {
            if !window.keyboard_modality {
                window.keyboard_modality = true;
                window.paint_requested = true;
            }
        } else if message == WM_LBUTTONDOWN && window.keyboard_modality {
            window.keyboard_modality = false;
            window.paint_requested = true;
        }
        if message == WM_LBUTTONDOWN {
            self.pointer_pressed(window, lparam)
        } else if message == WM_LBUTTONUP {
            self.pointer_released(window, lparam)
        } else if message == WM_MOUSEMOVE {
            self.pointer_moved(window, lparam)
        } else if message == WM_MOUSELEAVE {
            self.pointer_left(window)
        } else if message == WM_CAPTURECHANGED {
            self.capture_lost(window)
        } else if message == WM_CHAR {
            self.char_typed(window, wparam)
        } else if message == WM_KEYDOWN {
            self.key_down(window, wparam)
        } else if message == WM_IME_STARTCOMPOSITION {
            self.ime_start(window)
        } else if message == WM_IME_COMPOSITION {
            self.ime_composition(window, lparam)
        } else if message == WM_IME_ENDCOMPOSITION {
            window.pressed = None;
            self.ime_cancel(window)
        } else if message == WM_KILLFOCUS {
            self.focus_lost(window)
        } else if message == WM_TIMER && Some(wparam.0) == window.blink_timer {
            self.blink_caret(window)
        } else {
            Ok(())
        }
    }

    /// `WM_PAINT` 的宿主处理；错误留给消息循环返回。
    fn paint(&mut self, hwnd: HWND) {
        let Some(key) = self.key_for(hwnd) else {
            return;
        };
        let result = self.with_window(key, |host, window| {
            if window.close_requested {
                return Ok(());
            }
            host.render_fresh(window)
        });
        if let Err(error) = result {
            self.deferred_error.get_or_insert(error);
        }
    }

    /// `WM_CLOSE` 的宿主处理：结束该窗口的 activation，消息循环随后销毁 HWND。
    fn close_from_window(&mut self, hwnd: HWND) {
        let Some(key) = self.key_for(hwnd) else {
            return;
        };
        let Some(ctx) = self.windows.get(key).map(|window| window.ctx) else {
            return;
        };
        match self.runtime.close_window(ctx) {
            Ok(()) => {
                if let Some(window) = self.windows.get_mut(key) {
                    window.close_requested = true;
                    window.pressed = None;
                    window.hovered = None;
                    window.pointer = None;
                }
            }
            Err(error) => {
                self.deferred_error.get_or_insert(HostError::Close(error));
            }
        }
    }

    /// 交付已排队的 worker 结果，并只重绘快照确实变化的窗口。
    ///
    /// 每条唤醒消息都会交付全部已排队结果，因此多次完成只产生一次重绘；
    /// 应用级结果在最后一个窗口关闭后仍可驱动共享模型，但不再有任何重绘。
    fn deliver_tasks(&mut self) -> Result<(), HostError> {
        self.wake.observed();
        let report = self.runtime.drive_tasks().map_err(HostError::Runtime)?;
        if report.delivered == 0 {
            return Ok(());
        }
        self.repaint_changed_windows()
    }

    /// 闪烁 caret；只有聚焦输入、非组合且窗口在前台时才翻转。
    fn blink_caret(&mut self, window: &mut WindowHost) -> Result<(), HostError> {
        let snapshot = self
            .runtime
            .presentation_of(window.ctx)
            .map_err(HostError::Snapshot)?;
        let focused = focused_input(&snapshot).is_some();
        let composing = focused_composition(&snapshot);
        // 安全：只读取前台窗口句柄用于比较。
        let foreground = unsafe { GetForegroundWindow() } == window.hwnd();
        let visible = blinked(window.caret_visible, focused, composing, foreground);
        if visible == window.caret_visible {
            return Ok(());
        }
        window.caret_visible = visible;
        window.paint_requested = true;
        Ok(())
    }

    /// 按逻辑尺寸调整指定窗口的客户区并立即重绘；供嵌入方与测试使用。
    ///
    /// # Errors
    ///
    /// 窗口尺寸调整失败时返回 [`HostError::Window`]；随后的重绘可能返回
    /// surface 或渲染错误。
    pub fn resize(
        &mut self,
        key: &str,
        logical_width: u32,
        logical_height: u32,
    ) -> Result<(), HostError> {
        let hwnd = self
            .hwnd(key)
            .ok_or_else(|| HostError::UnknownWindow(key.to_owned()))?;
        let key = self
            .key_for(hwnd)
            .expect("the HWND belongs to a live window");
        // 安全：HWND 在窗口条目析构前有效。
        let dpi = unsafe { GetDpiForWindow(hwnd) };
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
        // 只调整窗口尺寸，位置与 Z 序不变。WM_SIZE 会同步进入窗口过程；
        // 调整期间禁止路由，避免重入正在借出的宿主。
        self.without_routing(None, |_| unsafe {
            SetWindowPos(
                hwnd,
                None,
                0,
                0,
                physical_width,
                physical_height,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        })
        .map_err(|error| HostError::Window(error.to_string()))?;
        self.render_key(key)
    }

    /// 呈现所有可见窗口；每个窗口构建自己的场景，共享 device/queue/renderer。
    ///
    /// # Errors
    ///
    /// 任一窗口的 surface、场景或绘制失败时返回对应错误。
    pub fn render(&mut self) -> Result<(), HostError> {
        for key in self.windows.keys().copied().collect::<Vec<_>>() {
            self.render_key(key)?;
        }
        Ok(())
    }

    fn render_key(&mut self, key: &'static str) -> Result<(), HostError> {
        self.with_window(key, |host, window| {
            if window.close_requested {
                return Ok(());
            }
            host.render_fresh(window)
        })
    }

    /// 用该窗口最新的呈现快照重绘；隐藏或已关闭的窗口什么也不做。
    fn render_fresh(&mut self, window: &mut WindowHost) -> Result<(), HostError> {
        let snapshot = match self.runtime.presentation_of(window.ctx) {
            Ok(snapshot) => snapshot,
            Err(WindowError::Hidden) => return Ok(()),
            Err(error) => return Err(HostError::Snapshot(error)),
        };
        self.render_window(window, snapshot)
    }

    /// 把一个窗口的快照渲染到它自己的 surface；共享 renderer 按目标尺寸切换。
    fn render_window(
        &mut self,
        window: &mut WindowHost,
        snapshot: PresentationSnapshot,
    ) -> Result<(), HostError> {
        self.ensure_surface(window)?;
        let (width, height) = client_size(window.hwnd())?;
        if width == 0 || height == 0 {
            return Ok(());
        }
        let scene = self.build_scene(window, &snapshot, width, height)?;
        let target = target_config(self.format, width, height)?;
        if self.renderer_target != target {
            self.renderer.resize(target);
            self.renderer_target = target;
        }
        self.renderer
            .set_palette(palette_for(snapshot.appearance()));
        let mut reconfigured = false;
        loop {
            let status = self.current_surface(window)?.get_current_texture();
            let frame = match status {
                wgpu::CurrentSurfaceTexture::Success(frame)
                | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    window.paint_requested = false;
                    return Ok(());
                }
                wgpu::CurrentSurfaceTexture::Outdated => {
                    if reconfigured {
                        return Err(HostError::Surface(String::from(
                            "the surface stayed outdated after reconfiguration",
                        )));
                    }
                    self.current_surface(window)?
                        .configure(&self.device, &window.surface_config);
                    reconfigured = true;
                    continue;
                }
                wgpu::CurrentSurfaceTexture::Lost => return self.surface_lost(),
                wgpu::CurrentSurfaceTexture::Validation => {
                    return Err(HostError::Surface(String::from(
                        "the surface failed validation; rebuilding would mask an API misuse",
                    )));
                }
            };
            let view = frame.texture.create_view(&Default::default());
            let mut encoder = self.device.create_command_encoder(&Default::default());
            if let Err(error) = self.renderer.render(&scene, &view, &mut encoder) {
                if reconfigured {
                    return Err(HostError::Render(error));
                }
                return self.surface_lost();
            }
            self.queue.submit([encoder.finish()]);
            self.queue.present(frame);
            window.scene = Some(scene);
            window.paint_requested = false;
            self.position_ime(window, &snapshot);
            return Ok(());
        }
    }

    /// 标记设备丢失；消息处理结束后由 [`Host::recover_device`] 统一重建。
    fn surface_lost(&mut self) -> Result<(), HostError> {
        if self.recovering {
            return Err(HostError::Surface(String::from(
                "the surface stayed lost after rebuilding",
            )));
        }
        self.device_lost = true;
        Ok(())
    }

    /// 重新创建 device/queue/renderer 与每个窗口的 surface，然后重绘全部窗口。
    fn recover_device(&mut self) -> Result<(), HostError> {
        self.device_lost = false;
        self.recovering = true;
        let result = self.rebuild_presentations().and_then(|()| self.render());
        self.recovering = false;
        result
    }

    /// 一次左键按下：文本输入在按下时聚焦并放置 caret；按钮与复选框只记录
    /// 按住目标并捕获指针，激活发生在同一节点上松开时。
    ///
    /// `lparam` 是客户区物理像素坐标；场景几何是逻辑像素，按最近一次呈现的 scale 换算。
    fn pointer_pressed(
        &mut self,
        window: &mut WindowHost,
        lparam: LPARAM,
    ) -> Result<(), HostError> {
        // 指针操作会移动焦点或 caret；先结束任何组合，避免 preedit 跟随到别处。
        self.cancel_compositions(window)?;
        // 一次指针按下只属于一个窗口；其它窗口的按压瞬态立即结束。
        self.clear_pressed_except(window.ctx.key());
        let scale = window
            .scene
            .as_ref()
            .map_or(1.0, |scene| scene.viewport().scale());
        let point = pointer_point(lparam, scale);
        window.pointer = Some(point);
        window.paint_requested = true;

        let Some((node, visual, caret, disabled)) = window.scene.as_ref().and_then(|scene| {
            let node = scene.hit_test(point)?;
            let element = scene.node(node)?;
            Some((
                node,
                element.visual(),
                element.caret_index(point),
                element.disabled(),
            ))
        }) else {
            window.hovered = None;
            window.pressed = None;
            return self
                .runtime
                .blur_in(window.ctx)
                .map_err(HostError::WindowState);
        };

        // disabled 控件不聚焦、不激活，也不改变焦点。
        if disabled {
            window.hovered = None;
            window.pressed = None;
            return Ok(());
        }
        window.hovered = Some(node);

        match visual {
            Visual::TextInput => {
                pointer_dispatch(self.runtime.focus_node(node))?;
                if let Some(offset) = caret {
                    pointer_dispatch(self.runtime.set_caret_node(node, offset))?;
                }
            }
            Visual::Checkbox { .. } | Visual::Button { .. } => {
                window.pressed = Some(node);
                // 捕获转移会同步向旧捕获窗口送出 WM_CAPTURECHANGED；
                // 在禁止重入的窗口里调用 SetCapture。
                // 安全：捕获指针，拖动离开后的松开仍回到本窗口，但不再激活。
                self.without_routing(Some(window.hwnd()), |_| unsafe {
                    SetCapture(window.hwnd());
                });
            }
            Visual::Container | Visual::Text => {
                window.pressed = None;
                self.runtime
                    .blur_in(window.ctx)
                    .map_err(HostError::WindowState)?;
            }
        }
        Ok(())
    }

    /// 指针移动：更新悬停目标；按住时离开原目标会清除按住状态。
    fn pointer_moved(&mut self, window: &mut WindowHost, lparam: LPARAM) -> Result<(), HostError> {
        let scale = window
            .scene
            .as_ref()
            .map_or(1.0, |scene| scene.viewport().scale());
        let point = pointer_point(lparam, scale);
        window.pointer = Some(point);
        self.arm_leave_tracking(window);

        let hovered = window.scene.as_ref().and_then(|scene| {
            let node = scene.hit_test(point)?;
            (!scene.node(node)?.disabled()).then_some(node)
        });
        let mut changed = hovered != window.hovered;
        window.hovered = hovered;
        if window.pressed.is_some() && window.pressed != hovered {
            window.pressed = None;
            changed = true;
        }
        if changed {
            window.paint_requested = true;
        }
        Ok(())
    }

    /// 指针离开客户区：清除悬停；按住目标由松开或捕获丢失结束。
    fn pointer_left(&mut self, window: &mut WindowHost) -> Result<(), HostError> {
        window.tracking_leave = false;
        window.pointer = None;
        if window.hovered.take().is_some() {
            window.paint_requested = true;
        }
        Ok(())
    }

    /// 左键松开：只有仍命中同一个启用节点时才激活按钮或复选框。
    fn pointer_released(
        &mut self,
        window: &mut WindowHost,
        lparam: LPARAM,
    ) -> Result<(), HostError> {
        let scale = window
            .scene
            .as_ref()
            .map_or(1.0, |scene| scene.viewport().scale());
        let point = pointer_point(lparam, scale);
        window.pointer = Some(point);
        // 先取走目标：松开后的捕获释放会同步送出 WM_CAPTURECHANGED，
        // 那条消息不能把这次松开要激活的目标一起清掉。
        let target = window.pressed.take();
        // 捕获释放在窗口过程借出宿主之后进行：`ReleaseCapture` 会同步送达
        // `WM_CAPTURECHANGED`，嵌套的 `with_host` 不能与外层 `&mut Host` 重叠。
        window.release_capture = true;
        window.paint_requested = true;
        let activated = target.is_some_and(|node| {
            window.scene.as_ref().is_some_and(|scene| {
                scene.hit_test(point) == Some(node)
                    && scene.node(node).is_some_and(|element| !element.disabled())
            })
        });
        if activated && let Some(node) = target {
            let visual = window
                .scene
                .as_ref()
                .and_then(|scene| scene.node(node))
                .map(SceneElement::visual);
            match visual {
                Some(Visual::Button { .. }) => pointer_dispatch(self.runtime.click_node(node))?,
                Some(Visual::Checkbox { checked }) => {
                    pointer_dispatch(self.runtime.change_node(node, !checked))?;
                }
                _ => {}
            }
        }

        window.hovered = window.scene.as_ref().and_then(|scene| {
            let node = scene.hit_test(point)?;
            (!scene.node(node)?.disabled()).then_some(node)
        });
        Ok(())
    }

    /// 捕获丢失（例如系统抢走指针）：按住状态不能残留。
    fn capture_lost(&mut self, window: &mut WindowHost) -> Result<(), HostError> {
        if window.pressed.take().is_some() {
            window.paint_requested = true;
        }
        Ok(())
    }

    /// 焦点丢失：清除按住瞬态与组合，并让下一帧不再绘制按下状态。
    fn focus_lost(&mut self, window: &mut WindowHost) -> Result<(), HostError> {
        let was_pressed = window.pressed.take().is_some();
        if was_pressed {
            window.release_capture = true;
        }
        let cancelled = self.cancel_compositions(window)?;
        if cancelled || was_pressed {
            window.paint_requested = true;
        }
        Ok(())
    }

    /// 只在第一次进入客户区时请求一次 `WM_MOUSELEAVE`。
    fn arm_leave_tracking(&mut self, window: &mut WindowHost) {
        // 原生隐藏的窗口没有真实光标；注册跟踪会让系统立即投递 leave。
        if window.tracking_leave || !window.native.shown {
            return;
        }
        let mut event = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: window.hwnd(),
            dwHoverTime: 0,
        };
        // 安全：结构由本地值初始化；失败时下一次移动会重试。
        if unsafe { TrackMouseEvent(&mut event) }.is_ok() {
            window.tracking_leave = true;
        }
    }

    /// 用指定窗口自己的场景渲染到离屏纹理并读回 BGRA 像素；隐藏 CI smoke 使用。
    pub fn read_frame(&mut self, key: &str) -> Result<Frame, HostError> {
        let hwnd = self
            .hwnd(key)
            .ok_or_else(|| HostError::UnknownWindow(key.to_owned()))?;
        let key = self
            .key_for(hwnd)
            .expect("the HWND belongs to a live window");
        self.with_window(key, |host, window| host.read_frame_inner(window))
    }

    fn read_frame_inner(&mut self, window: &mut WindowHost) -> Result<Frame, HostError> {
        let snapshot = self
            .runtime
            .presentation_of(window.ctx)
            .map_err(HostError::Snapshot)?;
        self.ensure_surface(window)?;
        let (width, height) = client_size(window.hwnd())?;
        if width == 0 || height == 0 {
            return Err(HostError::Readback(String::from(
                "cannot read a frame from an empty client area",
            )));
        }
        let scene = self.build_scene(window, &snapshot, width, height)?;
        let target = target_config(self.format, width, height)?;
        if self.renderer_target != target {
            self.renderer.resize(target);
            self.renderer_target = target;
        }
        self.renderer
            .set_palette(palette_for(snapshot.appearance()));
        let format = self.format;
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
        window.scene = Some(scene);
        window.paint_requested = false;
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
    fn char_typed(&mut self, window: &mut WindowHost, wparam: WPARAM) -> Result<(), HostError> {
        let unit = wparam.0 as u16;
        let character = match unit {
            0xD800..=0xDBFF => {
                window.pending_high_surrogate = Some(unit);
                return Ok(());
            }
            0xDC00..=0xDFFF => {
                let Some(high) = window.pending_high_surrogate.take() else {
                    return Ok(());
                };
                char::decode_utf16([high, unit]).next().and_then(Result::ok)
            }
            _ => {
                window.pending_high_surrogate = None;
                char::from_u32(u32::from(unit))
            }
        };
        let Some(character) = character else {
            return Ok(());
        };
        if character.is_control() {
            return Ok(());
        }
        let snapshot = self
            .runtime
            .presentation_of(window.ctx)
            .map_err(HostError::Snapshot)?;
        let Some(focused) = focused_input(&snapshot) else {
            return Ok(());
        };
        let mut buffer = [0_u8; 4];
        let text = character.encode_utf8(&mut buffer);
        // 部分 IME 把候选确认作为 WM_CHAR 送达，而不是 GCS_RESULTSTR；
        // 组合中的字符必须走 commit，否则普通编辑会被 reducer 拒绝。
        if is_composing(&snapshot, focused) {
            pointer_dispatch(self.runtime.compose_commit_node(focused, text))?;
        } else {
            pointer_dispatch(self.runtime.insert_text_node(focused, text))?;
        }
        window.paint_requested = true;
        Ok(())
    }

    /// 一次 `WM_KEYDOWN`：从线程消息队列读取修饰键后进入映射本体。
    fn key_down(&mut self, window: &mut WindowHost, wparam: WPARAM) -> Result<(), HostError> {
        let key = VIRTUAL_KEY(wparam.0 as u16);
        if key == VK_PROCESSKEY {
            // IME 已消费该键。部分 IME 在 Escape 结束组合时不发送
            // WM_IME_ENDCOMPOSITION；本地仍在组合而 OS 已无组合串时补一次取消。
            if self.local_composition_active(window)?
                && !self.os_composition_active(window)
                && self.cancel_compositions(window)?
            {
                window.paint_requested = true;
            }
            return Ok(());
        }
        // 安全：GetKeyState 只查询本线程最近一条键盘消息的修饰键状态。
        let shift = unsafe { GetKeyState(i32::from(VK_SHIFT.0)) } < 0;
        let control = unsafe { GetKeyState(i32::from(VK_CONTROL.0)) } < 0;
        self.key_down_with(window, key, shift, control)
    }

    /// 键盘映射本体：编辑、caret/selection、焦点遍历与表单提交。
    ///
    /// Shift+方向键保留 anchor、移动焦点端；Ctrl+A 全选。
    fn key_down_with(
        &mut self,
        window: &mut WindowHost,
        key: VIRTUAL_KEY,
        shift: bool,
        control: bool,
    ) -> Result<(), HostError> {
        if key == VK_TAB {
            self.cancel_compositions(window)?;
            if shift {
                self.runtime.focus_previous_in(window.ctx)
            } else {
                self.runtime.focus_next_in(window.ctx)
            }
            .map_err(HostError::WindowState)?;
            window.paint_requested = true;
            return Ok(());
        }

        let snapshot = self
            .runtime
            .presentation_of(window.ctx)
            .map_err(HostError::Snapshot)?;
        let Some(focused) = focused_input(&snapshot) else {
            return Ok(());
        };
        if key == VK_ESCAPE {
            // 运行时在组合中优先取消并恢复文本；否则派发给用户 keydown。
            let composing = is_composing(&snapshot, focused);
            pointer_dispatch(self.runtime.keydown_node(focused, Key::Escape))?;
            if composing {
                self.notify_ime_cancel(window);
            }
            window.paint_requested = true;
            return Ok(());
        }
        if control && key == VK_A {
            pointer_dispatch(self.runtime.select_all_node(focused))?;
            window.paint_requested = true;
            return Ok(());
        }
        if key == VK_RETURN {
            if !is_composing(&snapshot, focused) {
                pointer_dispatch(self.runtime.keydown_node(focused, Key::Enter))?;
                if let Some(form) = ancestor_form(&snapshot, focused) {
                    pointer_dispatch(self.runtime.submit_node(form))?;
                }
            }
            window.paint_requested = true;
            return Ok(());
        }
        if key == VK_BACK {
            pointer_dispatch(self.runtime.backspace_node(focused))?;
            window.paint_requested = true;
            return Ok(());
        }
        if key == VK_DELETE {
            pointer_dispatch(self.runtime.delete_forward_node(focused))?;
            window.paint_requested = true;
            return Ok(());
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
        window.paint_requested = true;
        Ok(())
    }

    /// `WM_IME_STARTCOMPOSITION`：在当前聚焦输入上开始组合。
    fn ime_start(&mut self, window: &mut WindowHost) -> Result<(), HostError> {
        let snapshot = self
            .runtime
            .presentation_of(window.ctx)
            .map_err(HostError::Snapshot)?;
        let Some(node) = focused_input(&snapshot) else {
            return Ok(());
        };
        if !is_composing(&snapshot, node) {
            pointer_dispatch(self.runtime.compose_start_node(node))?;
        }
        window.paint_requested = true;
        Ok(())
    }

    /// `WM_IME_COMPOSITION`：结果串提交，组合串更新 preedit。
    fn ime_composition(&mut self, window: &mut WindowHost, flags: LPARAM) -> Result<(), HostError> {
        let request = ime_request(flags.0 as u32);
        let snapshot = self
            .runtime
            .presentation_of(window.ctx)
            .map_err(HostError::Snapshot)?;
        let Some(node) = focused_input(&snapshot) else {
            return Ok(());
        };
        let commit = if request.commit {
            self.composition_string(window, GCS_RESULTSTR)?
        } else {
            None
        };
        let update = if request.update {
            self.composition_string(window, GCS_COMPSTR)?
        } else {
            None
        };
        self.apply_ime_composition(window, node, request, commit, update)?;
        window.paint_requested = true;
        Ok(())
    }

    /// 把已取出的 IME 字符串应用到运行时；与 Win32 消息解耦以便测试。
    fn apply_ime_composition(
        &mut self,
        window: &mut WindowHost,
        node: NodeId,
        request: ImeRequest,
        commit: Option<String>,
        update: Option<String>,
    ) -> Result<(), HostError> {
        let mut composing = self
            .runtime
            .presentation_of(window.ctx)
            .map_err(HostError::Snapshot)?
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
    fn ime_cancel(&mut self, window: &mut WindowHost) -> Result<(), HostError> {
        if self.cancel_compositions(window)? {
            window.paint_requested = true;
        }
        Ok(())
    }

    /// 取消所有仍在组合的输入并通知 OS IME；返回是否取消了组合。
    ///
    /// 扫描该窗口全部输入而不是只看焦点，焦点已在宿主内移走或输入被禁用时也能清理。
    fn cancel_compositions(&mut self, window: &mut WindowHost) -> Result<bool, HostError> {
        let snapshot = self
            .runtime
            .presentation_of(window.ctx)
            .map_err(HostError::Snapshot)?;
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
            self.notify_ime_cancel(window);
        }
        Ok(cancelled)
    }

    fn notify_ime_cancel(&self, window: &WindowHost) {
        // 安全：HIMC 由 ImmGetContext 获取并与 HWND 配对释放。
        unsafe {
            let himc = ImmGetContext(window.hwnd());
            if !himc.0.is_null() {
                let _ = ImmNotifyIME(himc, NI_COMPOSITIONSTR, CPS_CANCEL, 0);
                let _ = ImmReleaseContext(window.hwnd(), himc);
            }
        }
    }

    /// 运行时里是否有聚焦输入仍在组合。
    fn local_composition_active(&self, window: &WindowHost) -> Result<bool, HostError> {
        let snapshot = self
            .runtime
            .presentation_of(window.ctx)
            .map_err(HostError::Snapshot)?;
        Ok(focused_composition(&snapshot))
    }

    /// OS IME 当前是否仍有组合串；Escape 取消后部分 IME 直接清空它。
    fn os_composition_active(&self, window: &WindowHost) -> bool {
        // 安全：himc 与 HWND 配对；只查询组合串长度，不读取缓冲。
        let himc = unsafe { ImmGetContext(window.hwnd()) };
        if himc.0.is_null() {
            return false;
        }
        let bytes = unsafe { ImmGetCompositionStringW(himc, GCS_COMPSTR, None, 0) };
        unsafe {
            let _ = ImmReleaseContext(window.hwnd(), himc);
        }
        bytes > 0
    }

    fn composition_string(
        &self,
        window: &WindowHost,
        kind: IME_COMPOSITION_STRING,
    ) -> Result<Option<String>, HostError> {
        // 安全：himc 由 ImmGetContext 返回，读取的缓冲在调用期间有效。
        let himc = unsafe { ImmGetContext(window.hwnd()) };
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
            let _ = ImmReleaseContext(window.hwnd(), himc);
        }
        result
    }

    /// 把组合位置告诉 IME，并用 preedit 起点定位候选窗口。
    ///
    /// 系统组合窗口本身被 `WM_IME_STARTCOMPOSITION` 的窗口过程分支抑制；这里的
    /// `COMPOSITIONFORM` 仍参与候选窗定位。两个结构的坐标相对窗口左上角，不是屏幕坐标。
    fn position_ime(&self, window: &WindowHost, snapshot: &PresentationSnapshot) {
        let Some(node) = focused_input(snapshot) else {
            return;
        };
        if !is_composing(snapshot, node) {
            return;
        }
        let Some(scene) = &window.scene else {
            return;
        };
        // 候选窗锚在 preedit 起点，而不是 caret（preedit 末尾）。
        let composition_start = snapshot
            .node(node)
            .and_then(SnapshotElement::edit)
            .and_then(|edit| edit.composition())
            .map(|(start, _)| start);
        let caret = composition_start
            .and_then(|start| {
                scene
                    .node(node)
                    .and_then(|element| element.caret_at_value(start))
            })
            .or_else(|| scene.caret(node));
        let Some(caret) = caret else {
            return;
        };
        let scale = scene.viewport().scale();
        let point = POINT {
            x: (caret.x * scale).round() as i32,
            y: (caret.y * scale).round() as i32,
        };
        // 安全：HIMC 由 ImmGetContext 获取，两个结构都是本地值。
        unsafe {
            let himc = ImmGetContext(window.hwnd());
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
            let _ = ImmReleaseContext(window.hwnd(), himc);
        }
    }

    fn current_surface<'a>(
        &self,
        window: &'a WindowHost,
    ) -> Result<&'a wgpu::Surface<'static>, HostError> {
        window
            .surface
            .as_ref()
            .ok_or_else(|| HostError::Surface(String::from("the surface is not available")))
    }

    /// 设备丢失后重建 device/queue/renderer 与每个窗口的 surface。
    ///
    /// 先释放所有旧交换链；用第一个窗口的 surface 请求新 device，再让其余
    /// 窗口共享同一 adapter/device/format。失败时保持 surface 为空，下一次
    /// 消息会再次尝试重建。
    fn rebuild_presentations(&mut self) -> Result<(), HostError> {
        let Some(first_key) = self.windows.keys().next().copied() else {
            return Ok(());
        };
        for window in self.windows.values_mut() {
            window.surface = None;
        }
        let first_hwnd = self.windows[&first_key].hwnd();
        let first_surface = create_surface(&self._instance, first_hwnd, self.hinstance)?;
        let (adapter, device, queue) = request_backend(&self._instance, &first_surface)?;
        let first_config = surface_configuration(&adapter, &first_surface, first_hwnd)?;
        first_surface.configure(&device, &first_config);
        let format = first_config.format;
        let target = target_config(format, first_config.width, first_config.height)?;
        let renderer = GpuRenderer::from_parts(&device, &queue, target, RenderPalette::default());
        self.adapter = adapter;
        self.device = device;
        self.queue = queue;
        self.format = format;
        self.renderer = renderer;
        self.renderer_target = target;
        {
            let window = self
                .windows
                .get_mut(first_key)
                .expect("the first window stays live");
            window.surface = Some(first_surface);
            window.surface_config = first_config;
        }
        for key in self.windows.keys().copied().collect::<Vec<_>>() {
            if key == first_key {
                continue;
            }
            let hwnd = self.windows[&key].hwnd();
            let surface = create_surface(&self._instance, hwnd, self.hinstance)?;
            let config = surface_configuration(&self.adapter, &surface, hwnd)?;
            surface.configure(&self.device, &config);
            let window = self.windows.get_mut(key).expect("the window stays live");
            window.surface = Some(surface);
            window.surface_config = config;
        }
        Ok(())
    }

    fn ensure_surface(&mut self, window: &mut WindowHost) -> Result<(), HostError> {
        let (width, height) = client_size(window.hwnd())?;
        if width == 0 || height == 0 {
            return Ok(());
        }
        if window.surface.is_none() {
            let surface = create_surface(&self._instance, window.hwnd(), self.hinstance)?;
            let config = surface_configuration(&self.adapter, &surface, window.hwnd())?;
            surface.configure(&self.device, &config);
            window.surface = Some(surface);
            window.surface_config = config;
            return Ok(());
        }
        if window.surface_config.width == width && window.surface_config.height == height {
            return Ok(());
        }
        window.surface_config.width = width;
        window.surface_config.height = height;
        self.current_surface(window)?
            .configure(&self.device, &window.surface_config);
        Ok(())
    }

    fn build_scene(
        &mut self,
        window: &mut WindowHost,
        snapshot: &PresentationSnapshot,
        width: u32,
        height: u32,
    ) -> Result<Scene, HostError> {
        // 安全：HWND 在窗口条目析构前有效。
        let dpi = unsafe { GetDpiForWindow(window.hwnd()) };
        let scale = if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 };
        let viewport = Viewport::new(width as f32 / scale, height as f32 / scale, scale)
            .map_err(HostError::Scene)?;
        let mut scene =
            Scene::build_with(snapshot, viewport, &scene_env(window)).map_err(HostError::Scene)?;
        // 新场景可能已经移走或卸载了指针下的节点；失效目标不能继续高亮。
        if self.revalidate_pointer(window, &scene) {
            scene = Scene::build_with(snapshot, viewport, &scene_env(window))
                .map_err(HostError::Scene)?;
        }
        // 组合期间不绘制 caret，草稿由 preedit 下划线标识；闪烁只在非组合状态生效。
        let composing = focused_composition(snapshot);
        scene.set_caret_visible(window.caret_visible && !composing);
        window.last_snapshot = Some(snapshot.clone());
        Ok(scene)
    }

    /// 用新场景重新命中最后的指针位置；返回是否需要按新瞬态重建。
    fn revalidate_pointer(&mut self, window: &mut WindowHost, scene: &Scene) -> bool {
        let Some(point) = window.pointer else {
            return false;
        };
        let hit = scene.hit_test(point);
        let enabled = hit.filter(|node| scene.node(*node).is_some_and(|e| !e.disabled()));
        let hover_changed = enabled != window.hovered;
        let press_cleared = window.pressed.is_some_and(|pressed| {
            hit != Some(pressed) || scene.node(pressed).is_none_or(|element| element.disabled())
        });
        if !hover_changed && !press_cleared {
            return false;
        }
        window.hovered = enabled;
        if press_cleared {
            window.pressed = None;
        }
        true
    }

    /// 把一个窗口从映射中取出执行动作，再放回；设备丢失在放回后统一恢复。
    ///
    /// 动作同时需要该窗口与宿主的共享资源（runtime、device、renderer）。取出
    /// 条目让借用检查器看到两者互不重叠；动作运行在当前消息处理内，不会重入。
    fn with_window<R>(
        &mut self,
        key: &'static str,
        action: impl FnOnce(&mut Host, &mut WindowHost) -> Result<R, HostError>,
    ) -> Result<R, HostError> {
        let Some(mut window) = self.windows.remove(key) else {
            return Err(HostError::UnknownWindow(key.to_owned()));
        };
        let result = action(self, &mut window);
        self.windows.insert(key, window);
        if result.is_ok() && self.device_lost && !self.recovering {
            self.recover_device()?;
        }
        result
    }

    /// 在会同步派发窗口消息的 Win32 调用期间清除所有窗口过程的宿主地址。
    ///
    /// `extra` 用于 `with_window` 已从映射取出的当前窗口。`remove_window`
    /// 销毁窗口时使用自己的清除/恢复路径，不经过这里。
    fn without_routing<R>(
        &mut self,
        extra: Option<HWND>,
        action: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let pointer = self.dispatching;
        if pointer.is_some() {
            for hwnd in self.routing_hwnds(extra) {
                // 安全：只清除窗口过程的宿主地址槽。
                unsafe {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
            }
        }
        let result = action(self);
        if let Some(pointer) = pointer {
            // `extra` 是已从映射取出的当前窗口；调用方保证它在动作后仍存活
            // （销毁窗口走 `remove_window` 的专用路径）。
            for hwnd in self.routing_hwnds(extra) {
                // 安全：恢复派发期间的宿主地址。
                unsafe {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, pointer as isize);
                }
            }
        }
        result
    }

    fn routing_hwnds(&self, extra: Option<HWND>) -> Vec<HWND> {
        let mut hwnds: Vec<HWND> = self.windows.values().map(WindowHost::hwnd).collect();
        if let Some(extra) = extra
            && !hwnds.contains(&extra)
        {
            hwnds.push(extra);
        }
        hwnds
    }

    fn clear_pressed_except(&mut self, key: &'static str) {
        for (other_key, other) in &mut self.windows {
            if *other_key != key && other.pressed.take().is_some() {
                other.paint_requested = true;
            }
        }
    }

    fn key_for(&self, hwnd: HWND) -> Option<&'static str> {
        self.hwnds.get(&(hwnd.0 as isize)).copied()
    }

    fn take_release_capture(&mut self, hwnd: HWND) -> bool {
        let Some(key) = self.key_for(hwnd) else {
            return false;
        };
        self.windows
            .get_mut(key)
            .is_some_and(|window| std::mem::take(&mut window.release_capture))
    }

    fn insert_window(&mut self, window: WindowHost) {
        let key = window.ctx.key();
        let hwnd = window.hwnd();
        if let Some(pointer) = self.dispatching {
            // 安全：窗口过程只在派发期间读取这个槽。
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, pointer as isize);
            }
        }
        self.hwnds.insert(hwnd.0 as isize, key);
        self.windows.insert(key, window);
    }

    fn add_window(&mut self, info: &WindowInfo) -> Result<(), HostError> {
        let window = NativeWindow::new(info.spec, self.hinstance)?;
        let hwnd = window.hwnd;
        // 安全：HWND 在窗口条目析构前有效；定时器随窗口销毁。
        let blink_timer = unsafe { SetTimer(Some(hwnd), CARET_BLINK_TIMER, CARET_BLINK_MS, None) };
        let blink_timer = (blink_timer != 0).then_some(blink_timer);
        let surface = create_surface(&self._instance, hwnd, self.hinstance)?;
        let config = surface_configuration(&self.adapter, &surface, hwnd)?;
        surface.configure(&self.device, &config);
        self.insert_window(WindowHost::new(
            window,
            info.window,
            surface,
            config,
            blink_timer,
        ));
        Ok(())
    }

    fn remove_window(&mut self, key: &'static str) {
        let Some(window) = self.windows.remove(key) else {
            return;
        };
        self.hwnds.remove(&(window.hwnd().0 as isize));
        // DestroyWindow 会同步派发 WM_DESTROY 并可能激活其它窗口；整个销毁
        // 期间清除所有窗口过程的宿主地址。
        let pointer = self.dispatching;
        if pointer.is_some() {
            for hwnd in self.routing_hwnds(Some(window.hwnd())) {
                unsafe {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
            }
        }
        window.kill_timer();
        self.update_wake_target();
        drop(window);
        if let Some(pointer) = pointer {
            for hwnd in self.routing_hwnds(None) {
                unsafe {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, pointer as isize);
                }
            }
        }
        // 销毁期间的激活/重绘消息被跳过；让存活窗口在下一帧重绘。
        for window in self.windows.values_mut() {
            window.paint_requested = true;
        }
    }

    /// 按 runtime 的窗口修订号增删窗口条目，并同步原生可见性。
    fn sync_windows(&mut self) -> Result<(), HostError> {
        let revision = self.runtime.window_revision();
        if revision != self.revision {
            let infos = self.runtime.windows();
            let live: BTreeMap<&'static str, WindowInfo> =
                infos.iter().map(|info| (info.spec.key(), *info)).collect();
            for key in self.windows.keys().copied().collect::<Vec<_>>() {
                let current = live
                    .get(key)
                    .is_some_and(|info| self.windows[key].ctx == info.window);
                if !current {
                    self.remove_window(key);
                }
            }
            for info in &infos {
                if !self.windows.contains_key(info.spec.key()) {
                    self.add_window(info)?;
                }
            }
            // 只有全部窗口都就绪才提交修订号；失败时下一次同步会重试缺的窗口。
            self.revision = revision;
        }
        self.sync_visibility();
        self.update_wake_target();
        Ok(())
    }

    /// 运行时可见性与 native `ShowWindow` 状态对齐；隐藏模式从不显示窗口。
    fn sync_visibility(&mut self) {
        for (key, show) in self.desired_visibility() {
            self.set_native_visible(key, show);
        }
    }

    /// 每个原生窗口应有的可见性；隐藏的运行时窗口与 CI 隐藏模式都是 false。
    fn desired_visibility(&self) -> Vec<(&'static str, bool)> {
        let visible: BTreeMap<&'static str, bool> = self
            .runtime
            .windows()
            .into_iter()
            .map(|info| (info.spec.key(), info.visible))
            .collect();
        self.windows
            .keys()
            .map(|key| {
                (
                    *key,
                    self.show_windows && visible.get(key).copied().unwrap_or(false),
                )
            })
            .collect()
    }

    /// 切换一个窗口的原生可见性；刷新期间清空 USERDATA，避免 ShowWindow
    /// 同步派发的焦点/尺寸消息重入正被借出的宿主。
    fn set_native_visible(&mut self, key: &'static str, show: bool) {
        let Some(window) = self.windows.get(key) else {
            return;
        };
        if window.native.shown == show {
            return;
        }
        let hwnd = window.hwnd();
        // ShowWindow 会同步派发激活/焦点消息给同进程的其它窗口；整个调用期间
        // 清除所有窗口过程的宿主地址，避免重入正在借出的 Host。
        self.without_routing(None, |_| {
            // 安全：HWND 由窗口条目持有。
            unsafe {
                let _ = ShowWindow(hwnd, if show { SW_SHOW } else { SW_HIDE });
                if !show && GetCapture() == hwnd {
                    let _ = ReleaseCapture();
                }
            }
        });
        let Some(window) = self.windows.get_mut(key) else {
            return;
        };
        window.native.shown = show;
        window.paint_requested = true;
        if !show {
            window.pointer = None;
            window.hovered = None;
            window.pressed = None;
            window.release_capture = false;
            window.tracking_leave = false;
        }
        // 路由关闭期间其它窗口可能收到但跳过了重绘消息；统一标记后重绘。
        for window in self.windows.values_mut() {
            window.paint_requested = true;
        }
    }

    fn update_wake_target(&self) {
        let target = self.windows.values().next().map(WindowHost::hwnd);
        let changed = self.wake.set_target(target);
        // 有唤醒在途而目标被销毁时，把那条唤醒转投到仍存活的窗口；
        // 否则 pending 会永远为 true，后续完成不再投递。
        if changed
            && self.wake.is_pending()
            && let Some(target) = target
            && unsafe { PostMessageW(Some(target), HOST_WAKE_MESSAGE, WPARAM(0), LPARAM(0)) }
                .is_err()
        {
            self.wake.observed();
        }
    }

    /// 逐窗口比较最新快照与瞬时重绘请求；只重绘确有变化的窗口。
    fn repaint_changed_windows(&mut self) -> Result<(), HostError> {
        for key in self.windows.keys().copied().collect::<Vec<_>>() {
            let Some(window) = self.windows.get(&key) else {
                continue;
            };
            let ctx = window.ctx;
            let requested = window.paint_requested;
            let snapshot = match self.runtime.presentation_of(ctx) {
                Ok(snapshot) => snapshot,
                // 隐藏窗口不呈现；保留 paint_requested，显示后再绘制。
                Err(_) => continue,
            };
            let unchanged = !requested
                && self
                    .windows
                    .get(&key)
                    .and_then(|window| window.last_snapshot.as_ref())
                    == Some(&snapshot);
            if unchanged {
                continue;
            }
            self.with_window(key, |host, window| host.render_window(window, snapshot))?;
        }
        Ok(())
    }
}

impl WindowHost {
    fn new(
        native: NativeWindow,
        ctx: WindowCtx,
        surface: wgpu::Surface<'static>,
        surface_config: wgpu::SurfaceConfiguration,
        blink_timer: Option<usize>,
    ) -> Self {
        Self {
            surface: Some(surface),
            surface_config,
            native,
            ctx,
            scene: None,
            pointer: None,
            hovered: None,
            pressed: None,
            release_capture: false,
            tracking_leave: false,
            keyboard_modality: false,
            pending_high_surrogate: None,
            caret_visible: true,
            blink_timer,
            close_requested: false,
            paint_requested: true,
            last_snapshot: None,
        }
    }

    fn kill_timer(&self) {
        if let Some(timer) = self.blink_timer {
            // 安全：定时器属于本窗口；重复销毁安全。
            unsafe {
                let _ = KillTimer(Some(self.hwnd()), timer);
            }
        }
    }
}

fn scene_env(window: &WindowHost) -> SceneEnv {
    SceneEnv::default()
        .with_hovered(window.hovered)
        .with_pressed(window.pressed)
        .with_focus_visible(window.keyboard_modality)
}

impl Drop for Host {
    fn drop(&mut self) {
        // Drop 不能返回错误；先阻断 worker 唤醒，再尽力释放每个窗口的 activation。
        self.wake.close();
        for window in self.windows.values() {
            window.kill_timer();
        }
        let contexts: Vec<WindowCtx> = self.windows.values().map(|window| window.ctx).collect();
        for ctx in contexts {
            let _ = self.runtime.close_window(ctx);
        }
    }
}

/// 宿主的一帧失败；没有 panics。
#[derive(Debug)]
pub enum HostError {
    /// 运行时 setup、挂载或快照失败。
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
    /// 宿主没有这个目录键的原生窗口。
    UnknownWindow(String),
    /// 打开或显示窗口失败。
    Open(WindowError),
    /// 关闭窗口并释放 activation 失败。
    Close(WindowError),
    /// 读取窗口的呈现快照失败（隐藏或 stale）。
    Snapshot(WindowError),
    /// 窗口状态操作（焦点遍历、清除焦点）失败。
    WindowState(WindowError),
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
            Self::UnknownWindow(key) => write!(f, "window key {key} has no native window"),
            Self::Open(error) => write!(f, "opening the window failed: {error}"),
            Self::Close(error) => write!(f, "closing the window failed: {error}"),
            Self::Snapshot(error) => write!(f, "reading the window snapshot failed: {error}"),
            Self::WindowState(error) => write!(f, "window state change failed: {error}"),
            Self::Scene(error) => write!(f, "scene failed: {error}"),
            Self::Render(error) => write!(f, "render failed: {error}"),
            Self::Input(error) => write!(f, "pointer dispatch failed: {error}"),
            Self::MessageLoop => f.write_str("the Win32 message loop failed"),
        }
    }
}

impl Error for HostError {}

struct NativeWindow {
    hwnd: HWND,
    /// 原生窗口当前是否由 `ShowWindow(SW_SHOW)` 显示；隐藏模式始终为 false。
    shown: bool,
}

impl NativeWindow {
    fn new(spec: WindowSpec, hinstance: HINSTANCE) -> Result<Self, HostError> {
        let title: HSTRING = spec.title().into();
        let style = WS_OVERLAPPEDWINDOW;
        let (width, height) = spec.size();
        // 目录尺寸是逻辑像素；按系统 DPI 换算成初始物理客户区。
        // 安全：进程 DPI awareness 已在 Host::open 设置。
        let dpi = unsafe { GetDpiForSystem() };
        let scale = if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 };
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: (width as f32 * scale) as i32,
            bottom: (height as f32 * scale) as i32,
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
        Ok(Self { hwnd, shown: false })
    }
}

impl Drop for NativeWindow {
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

/// 聚焦输入是否仍在组合；没有聚焦输入时视为未组合。
fn focused_composition(snapshot: &PresentationSnapshot) -> bool {
    focused_input(snapshot).is_some_and(|node| is_composing(snapshot, node))
}

/// 一次闪烁后的 caret 可见性：只有聚焦输入、非组合且窗口在前台时才翻转；
/// 其他情况保持可见，避免 caret 停在该隐藏的状态。
const fn blinked(current: bool, focused: bool, composing: bool, foreground: bool) -> bool {
    if focused && !composing && foreground {
        !current
    } else {
        true
    }
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

/// 按新旧 DPI 比例换算物理客户区，使 DPI 变化后的逻辑客户区保持不变。
fn scaled_client_size(width: i32, height: i32, current_dpi: u32, new_dpi: u32) -> (i32, i32) {
    let scale = new_dpi as f32 / current_dpi as f32;
    (
        (width as f32 * scale).round() as i32,
        (height as f32 * scale).round() as i32,
    )
}

/// 从 HWND 创建 DX12 surface；重建时先换掉旧 surface 再配置。
fn create_surface(
    instance: &wgpu::Instance,
    hwnd: HWND,
    hinstance: HINSTANCE,
) -> Result<wgpu::Surface<'static>, HostError> {
    let raw_window = raw_window_handle(hwnd, hinstance)?;
    // 安全：HWND 由 WindowHost 的 NativeWindow 持有，销毁推迟到
    // NativeWindow::drop；WindowHost 的 surface 字段先于 native 声明，
    // 因此 surface 先于 HWND 释放。
    unsafe {
        instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(raw_display_handle()),
            raw_window_handle: raw_window,
        })
    }
    .map_err(|error| HostError::Surface(error.to_string()))
}

/// 一次 surface 配置产生的 GPU 资源；首次打开与丢失恢复共用。
/// 为一个已存在的 surface 选择非 sRGB BGRA 交换链配置。
///
/// RenderPalette 的字节是 sRGB 编码值；只有非 sRGB 视图能原样保存它们。
/// sRGB 视图会再做一次编码，因此这里明确拒绝而不是静默产出错误颜色。
fn surface_configuration(
    adapter: &wgpu::Adapter,
    surface: &wgpu::Surface<'_>,
    hwnd: HWND,
) -> Result<wgpu::SurfaceConfiguration, HostError> {
    let (width, height) = client_size(hwnd)?;
    let capabilities = surface.get_capabilities(adapter);
    if !capabilities
        .formats
        .contains(&wgpu::TextureFormat::Bgra8Unorm)
    {
        return Err(HostError::Surface(String::from(
            "the DX12 surface offers no non-sRGB BGRA format; an sRGB-only surface needs an explicit conversion",
        )));
    }
    let mut config = surface
        .get_default_config(adapter, width.max(1), height.max(1))
        .ok_or_else(|| {
            HostError::Surface(String::from("the surface has no default configuration"))
        })?;
    config.format = wgpu::TextureFormat::Bgra8Unorm;
    config.present_mode = wgpu::PresentMode::Fifo;
    Ok(config)
}

const fn palette_for(appearance: Appearance) -> RenderPalette {
    match appearance {
        Appearance::Light => RenderPalette::light(),
        Appearance::Dark => RenderPalette::dark(),
    }
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

/// 窗口过程可见的宿主指针；析构时清除所有窗口的槽位。
///
/// 只在 `run` 与 `pump` 期间安装：两者的循环在派发消息前不持有宿主引用，
/// 窗口过程因此可以在系统模态循环里安全地重入宿主。循环中新开的窗口由
/// [`Host::insert_window`] 补装；析构清除也覆盖 panic 展开与提前返回。
struct HostPointer {
    host: *mut Host,
}

impl HostPointer {
    fn install(host: *mut Host) -> Self {
        // 安全：槽位只保存地址，不拥有对象；调用方保证 host 在 guard 生命周期
        // 内有效，且此时没有其它引用正在访问它。
        unsafe {
            (*host).dispatching = Some(host);
            for window in (*host).windows.values() {
                SetWindowLongPtrW(window.hwnd(), GWLP_USERDATA, host as isize);
            }
        }
        Self { host }
    }
}

impl Drop for HostPointer {
    fn drop(&mut self) {
        // 安全：循环已返回，重新通过裸指针访问宿主。
        unsafe {
            for window in (*self.host).windows.values() {
                SetWindowLongPtrW(window.hwnd(), GWLP_USERDATA, 0);
            }
            (*self.host).dispatching = None;
        }
    }
}

/// 在窗口过程里取回宿主并执行一次动作；未安装指针时什么也不做。
///
/// # Safety
///
/// 动作执行期间不得通过其他引用访问宿主。`run`/`pump` 满足这一点：它们在
/// 派发消息时只持有裸指针，派发返回后才重新借出宿主。
unsafe fn with_host(hwnd: HWND, action: impl FnOnce(&mut Host)) {
    // 安全：读取窗口的 user data 槽；未安装时为零。
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Host;
    if pointer.is_null() {
        return;
    }
    // 安全：指针由 HostPointer 安装，指向在 `run`/`pump` 期间不会移动的 Host；
    // 调用方不在动作执行期间访问宿主。
    let host = unsafe { &mut *pointer };
    action(host);
}

/// 取走窗口过程的延迟捕获释放请求；调用时不得持有宿主引用。
///
/// # Safety
///
/// 调用方必须在窗口过程借出宿主结束后调用，期间不得有其它引用指向宿主。
unsafe fn take_capture_release(hwnd: HWND) -> bool {
    // 安全：读取窗口的 user data 槽；未安装时为零。
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Host;
    if pointer.is_null() {
        return false;
    }
    // 安全：指针由 HostPointer 安装；调用方不持有宿主引用。
    let host = unsafe { &mut *pointer };
    host.take_release_capture(hwnd)
}

/// 翻译并派发一条消息；窗口过程负责处理。
fn dispatch_message(message: &MSG) {
    // 安全：消息来自本线程队列，窗口由 Host 持有。
    unsafe {
        let _ = TranslateMessage(message);
        DispatchMessageW(message);
    }
}

/// `run` 的消息循环；返回关闭或处理错误。
///
/// # Safety
///
/// `host` 指向在循环期间不会移动的 `Host`；循环在派发消息时不持有它的引用。
unsafe fn run_loop(host: *mut Host) -> Result<(), HostError> {
    loop {
        let mut message = MSG::default();
        // 安全：标准 Win32 消息循环；窗口由 Host 持有。
        let result = unsafe { GetMessageW(&mut message, None, 0, 0).0 };
        if result == -1 {
            return Err(HostError::MessageLoop);
        }
        if result == 0 {
            return Ok(());
        }
        dispatch_message(&message);
        // 安全：派发已返回，重新借出宿主同步窗口并读取处理结果。
        let host = unsafe { &mut *host };
        host.sync_windows()?;
        host.repaint_changed_windows()?;
        if let Some(error) = host.deferred_error.take() {
            return Err(error);
        }
        if host.runtime.windows().is_empty() {
            return Ok(());
        }
    }
}

/// `pump` 的消息循环；返回是否已没有存活窗口。
///
/// # Safety
///
/// 与 [`run_loop`] 相同。
unsafe fn pump_loop(host: *mut Host) -> Result<bool, HostError> {
    loop {
        // 安全：每次迭代重新借出宿主；派发期间不持有引用。
        if unsafe { &*host }.runtime.windows().is_empty() {
            return Ok(true);
        }
        let mut message = MSG::default();
        // 安全：只取本线程队列里的消息；窗口由 Host 持有。
        let result = unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) };
        if result.0 == 0 {
            return Ok(false);
        }
        dispatch_message(&message);
        // 安全：派发已返回，重新借出宿主同步窗口并读取处理结果。
        let host = unsafe { &mut *host };
        host.sync_windows()?;
        host.repaint_changed_windows()?;
        if let Some(error) = host.deferred_error.take() {
            return Err(error);
        }
        if host.runtime.windows().is_empty() {
            return Ok(true);
        }
    }
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        // 提交已经由 WM_IME_COMPOSITION 的 GCS_RESULTSTR 或 WM_CHAR 处理；
        // WM_IME_CHAR 若再交给默认窗口过程会产生重复字符。
        WM_IME_CHAR => LRESULT(0),
        WM_IME_STARTCOMPOSITION => {
            // 应用自己在输入框里绘制 preedit；把开始消息交给默认过程会让微软
            // 拼音再画一份组合窗口，与 preedit 叠成两层。这里只通知运行时。
            // 安全：派发期间不持有宿主引用，窗口过程借出一次。
            unsafe {
                with_host(hwnd, |host| {
                    host.handle_message(hwnd, message, wparam, lparam)
                })
            };
            LRESULT(0)
        }
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            // 安全：hwnd 来自系统回调；paint 是有效的输出缓冲。
            unsafe {
                let _ = BeginPaint(hwnd, &mut paint);
            }
            // 尺寸变化已同步呈现并清除区域；空的更新区域不再重复绘制。
            let region = paint.rcPaint;
            if region.right > region.left && region.bottom > region.top {
                // 安全：派发期间不持有宿主引用，窗口过程可以借出一次。
                unsafe { with_host(hwnd, |host| host.paint(hwnd)) };
            }
            // 安全：与 BeginPaint 配对。
            unsafe {
                let _ = EndPaint(hwnd, &paint);
            }
            LRESULT(0)
        }
        WM_SIZE => {
            // 拖动边框时系统进入自己的模态循环，只派发消息不返回消息循环。
            // 先让默认过程完成尺寸变化与类样式失效，再同步按新客户区重排并呈现；
            // 这样 DWM 在窗口缩放后拿到的是新帧，而不是被拉伸的上一帧。
            let result = unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
            // 安全：派发期间不持有宿主引用，窗口过程可以借出一次。
            unsafe { with_host(hwnd, |host| host.paint(hwnd)) };
            // 帧已按当前尺寸呈现；清除同一尺寸的重复重绘区域。
            // 安全：只清除本窗口的更新区域。
            unsafe {
                let _ = ValidateRect(Some(hwnd), None);
            }
            result
        }
        WM_CLOSE => {
            // 安全：派发期间不持有宿主引用；结束该窗口的 activation。
            // HWND 由消息循环在派发返回后同步销毁，最后一个窗口关闭时循环退出。
            unsafe { with_host(hwnd, |host| host.close_from_window(hwnd)) };
            LRESULT(0)
        }
        WM_DESTROY => {
            // 销毁由宿主在同步阶段发起；循环以“没有存活窗口”判断退出，
            // 这里不投递 WM_QUIT，避免关闭一个附属窗口时结束其他窗口的消息循环。
            LRESULT(0)
        }
        WM_GETDPISCALEDSIZE => {
            // 系统默认按 DPI 比例线性缩放外框，非客户区取整会让客户区的逻辑
            // 尺寸漂移一两像素。这里按当前物理客户区的比例算出目标客户区，
            // 再按新 DPI 加上非客户区，把外框作为候选尺寸交给 WM_DPICHANGED。
            let size = lparam.0 as *mut SIZE;
            let new_dpi = (wparam.0 & 0xFFFF) as u32;
            // 最大化/最小化时由系统按工作区调整；无效参数同样交回默认过程。
            // 安全：hwnd 来自系统回调。
            let special = unsafe { IsIconic(hwnd).as_bool() || IsZoomed(hwnd).as_bool() };
            if size.is_null() || new_dpi == 0 || special {
                return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
            }
            let mut client = RECT::default();
            // 安全：client 是有效的输出缓冲；hwnd 来自系统回调。
            let current_dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
            // 安全：client 是有效的输出缓冲。
            if unsafe { GetClientRect(hwnd, &mut client) }.is_err() {
                return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
            }
            let (width, height) = scaled_client_size(
                client.right - client.left,
                client.bottom - client.top,
                current_dpi,
                new_dpi,
            );
            let mut outer = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };
            // 安全：outer 是本地值；样式与窗口创建时一致。
            if unsafe {
                AdjustWindowRectExForDpi(
                    &mut outer,
                    WS_OVERLAPPEDWINDOW,
                    false,
                    WINDOW_EX_STYLE::default(),
                    new_dpi,
                )
            }
            .is_err()
            {
                return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
            }
            // 安全：lParam 是系统的 in/out SIZE；上面已检查非空。
            unsafe {
                (*size).cx = outer.right - outer.left;
                (*size).cy = outer.bottom - outer.top;
            }
            LRESULT(1)
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
            // WM_GETDPISCALEDSIZE 不保证送达（常见于交互拖动），所以这里仍以
            // 系统建议矩形为准。DPI 改变可能不改变窗口尺寸，因而没有 WM_SIZE；
            // 这里同步按新 scale 重绘。
            // 安全：派发期间不持有宿主引用，窗口过程可以借出一次。
            unsafe { with_host(hwnd, |host| host.paint(hwnd)) };
            // 安全：只清除本窗口的更新区域。
            unsafe {
                let _ = ValidateRect(Some(hwnd), None);
            }
            LRESULT(0)
        }
        _ => {
            // 安全：默认窗口过程。
            let result = unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
            // 安全：默认过程已返回；窗口过程借出宿主处理这条消息。
            unsafe {
                with_host(hwnd, |host| {
                    host.handle_message(hwnd, message, wparam, lparam)
                })
            };
            // 捕获释放在借出结束后进行；`ReleaseCapture` 同步送出
            // `WM_CAPTURECHANGED`，嵌套的 `with_host` 不能与外层引用重叠。
            if unsafe { take_capture_release(hwnd) } {
                // 安全：释放本窗口的指针捕获；没有捕获时同样安全。
                unsafe {
                    let _ = ReleaseCapture();
                }
            }
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::SendMessageW;

    const MAIN: &str = "main";

    #[test]
    fn resizing_relayouts_the_scene_and_surface() {
        let mut host = open_host();
        host.render().expect("render");
        let before = host.scene(MAIN).expect("scene").viewport().width();

        host.resize(MAIN, 640, 480).expect("resize");
        let scene = host.scene(MAIN).expect("scene");
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

        let (width, height) = client_size(host.windows[MAIN].hwnd()).expect("client size");
        let frame = host.read_frame(MAIN).expect("frame after resize");
        assert_eq!((frame.width(), frame.height()), (width, height));
    }

    #[test]
    fn get_dpi_scaled_size_preserves_the_logical_client_size() {
        let mut host = open_host();
        host.resize(MAIN, 640, 480).expect("resize");
        let hwnd = host.hwnd(MAIN).expect("a live window");
        let new_dpi = 192;
        let mut size = SIZE::default();
        // 安全：hwnd 属于本线程；WM_GETDPISCALEDSIZE 的 lParam 是 SIZE 的 in/out 指针。
        let result = unsafe {
            SendMessageW(
                hwnd,
                WM_GETDPISCALEDSIZE,
                Some(WPARAM(new_dpi as usize)),
                Some(LPARAM(&mut size as *mut SIZE as isize)),
            )
        };
        assert_eq!(result.0, 1, "the host computes the DPI-scaled size");

        // 640×480 逻辑客户区在 192 DPI 下是 1280×960 物理像素；
        // 目标外框由系统按新 DPI 的窗口样式加出。
        let mut outer = RECT {
            left: 0,
            top: 0,
            right: 1280,
            bottom: 960,
        };
        // 安全：outer 是本地值；样式与窗口创建时一致。
        unsafe {
            AdjustWindowRectExForDpi(
                &mut outer,
                WS_OVERLAPPEDWINDOW,
                false,
                WINDOW_EX_STYLE::default(),
                new_dpi,
            )
        }
        .expect("frame metrics");
        assert_eq!(
            (size.cx, size.cy),
            (outer.right - outer.left, outer.bottom - outer.top),
            "the candidate outer size keeps the logical client size"
        );
    }

    #[test]
    fn resizing_to_zero_renders_nothing_and_refuses_readback() {
        let mut host = open_host();
        host.resize(MAIN, 0, 0).expect("zero resize");
        assert!(
            matches!(host.read_frame(MAIN), Err(HostError::Readback(_))),
            "a zero client area cannot be read back"
        );
        host.resize(MAIN, 320, 240).expect("restore size");
        let frame = host.read_frame(MAIN).expect("frame after restore");
        assert!(frame.width() > 0 && frame.height() > 0);
    }

    #[test]
    fn a_paint_message_repaints_after_an_external_resize() {
        let mut host = open_host();
        host.render().expect("initial render");
        let before = host.scene(MAIN).expect("scene").viewport().width();

        // 系统的拖动/缩放循环会直接调用窗口过程；这里用 SetWindowPos + WM_PAINT
        // 复现同一路径，确认重排发生在窗口过程里而不是消息循环里。
        let hwnd = host.hwnd(MAIN).expect("a live window");
        {
            let _pointer = HostPointer::install(&mut host as *mut Host);
            // 安全：hwnd 有效；只改大小，不移动也不改变 Z 序。
            unsafe {
                SetWindowPos(
                    hwnd,
                    None,
                    0,
                    0,
                    640,
                    480,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                )
                .expect("resize the window");
                SendMessageW(hwnd, WM_PAINT, Some(WPARAM(0)), Some(LPARAM(0)));
            }
        }

        // 窗口尺寸不是客户区尺寸；重排的目标是新的客户区。
        let (width, _) = client_size(hwnd).expect("client size after resize");
        let scene = host.scene(MAIN).expect("scene after paint");
        let scale = scene.viewport().scale();
        assert!(
            (scene.viewport().width() - width as f32 / scale).abs() < 2.0,
            "WM_PAINT relayouts at the new physical client size"
        );
        assert_ne!(scene.viewport().width(), before);
    }

    #[test]
    fn caret_blinking_requires_focus_composition_and_foreground() {
        assert!(!blinked(true, true, false, true), "a blink hides the caret");
        assert!(blinked(false, true, false, true), "the next tick shows it");
        assert!(
            blinked(false, false, false, true),
            "no focused input keeps the caret visible"
        );
        assert!(
            blinked(false, true, true, true),
            "composition keeps the caret visible"
        );
        assert!(
            blinked(false, true, false, false),
            "a background window keeps the caret visible"
        );
    }

    #[test]
    fn a_closed_host_ignores_later_paint_and_input() {
        let mut host = open_host();
        host.render().expect("initial render");
        let hwnd = host.hwnd(MAIN).expect("a live window");
        host.close_from_window(hwnd);
        assert!(host.windows[MAIN].close_requested);

        // 关闭后 runtime 已释放 activation；再绘制或输入若没被跳过会记录错误。
        host.handle_message(hwnd, WM_CHAR, WPARAM('X' as usize), LPARAM(0));
        host.paint(hwnd);
        assert!(
            host.deferred_error.is_none(),
            "a closed host does not touch the runtime"
        );
    }

    fn wake_state() -> WakeState {
        WakeState {
            pending: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            target: AtomicIsize::new(0),
        }
    }

    #[test]
    fn wake_notifications_coalesce_until_the_host_observes_them() {
        let wake = wake_state();
        assert!(wake.claim(), "the first wake posts one message");
        assert!(
            !wake.claim(),
            "further wakes coalesce into the queued message"
        );
        wake.observed();
        assert!(wake.claim(), "a wake after handling posts again");
    }

    #[test]
    fn concurrent_wakes_claim_exactly_one_message() {
        let wake = Arc::new(wake_state());
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let wake = Arc::clone(&wake);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    wake.claim()
                })
            })
            .collect();
        barrier.wait();
        let claimed = workers
            .into_iter()
            .map(|worker| worker.join().expect("worker"))
            .filter(|claimed| *claimed)
            .count();
        assert_eq!(claimed, 1, "exactly one concurrent wake posts a message");
    }

    #[test]
    fn closing_the_window_cancels_window_owned_tasks() {
        let mut host = open_host();
        // 事件 handler 在已安装的运行时上下文里启动窗口所有权任务；
        // 不 pump 唤醒消息，确保关闭发生在交付之前。
        host.runtime
            .click_text("Preview import")
            .expect("spawn a window-owned task");

        host.close(MAIN).expect("close the window");
        host.runtime.wait_for_tasks();

        let report = host.runtime.drive_tasks().expect("drive after close");
        assert_eq!(report.delivered, 0, "a closed window receives no delivery");
        assert_eq!(report.discarded, 1, "the finished result is discarded");
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
            .scene(MAIN)
            .expect("scene")
            .elements()
            .iter()
            .map(|element| element.id())
            .collect();
        let before = host.read_frame(MAIN).expect("frame");
        let caret_before = host
            .scene(MAIN)
            .expect("scene")
            .caret(node)
            .expect("caret")
            .x;

        host.rebuild_presentations().expect("rebuild");
        host.render().expect("render after rebuild");

        let scene = host.scene(MAIN).expect("scene after rebuild");
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
        let after = host.read_frame(MAIN).expect("frame after rebuild");
        assert_eq!(before.pixels(), after.pixels());
    }

    fn open_two_window_host() -> Host {
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(|catalog| {
                catalog.window(
                    WindowSpec::new("main")
                        .primary()
                        .titled("Main two")
                        .sized(320, 240),
                    |_| view! { <text>"one"</text> },
                )?;
                catalog.window(
                    WindowSpec::new("second")
                        .titled("Second two")
                        .sized(200, 120),
                    |_| view! { <text>"two"</text> },
                )?;
                Ok(())
            })
            .expect("declare two windows");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open host");
        host.show("second").expect("show second");
        host.render().expect("render both");
        host
    }

    #[test]
    fn native_visibility_follows_runtime_state_and_hidden_mode() {
        let mut host = open_two_window_host();
        assert!(
            host.desired_visibility().iter().all(|(_, show)| !*show),
            "hidden mode never shows a native window"
        );

        host.show_windows = true;
        assert!(
            host.desired_visibility().iter().all(|(_, show)| *show),
            "visible mode follows the runtime visibility"
        );

        host.hide("second").expect("hide second");
        let targets = host.desired_visibility();
        assert_eq!(
            targets
                .iter()
                .find(|(key, _)| *key == "second")
                .map(|(_, show)| *show),
            Some(false),
            "a hidden runtime window is hidden natively"
        );
        assert_eq!(
            targets
                .iter()
                .find(|(key, _)| *key == "main")
                .map(|(_, show)| *show),
            Some(true),
            "the other window keeps its native visibility"
        );
        host.show_windows = false;
    }

    fn open_two_input_host() -> Host {
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(|catalog| {
                catalog.window(WindowSpec::new("main").primary(), |_| {
                    view! { <text-input aria:label="main-input" /> }
                })?;
                catalog.window(WindowSpec::new("second"), |_| {
                    view! { <text-input aria:label="second-input" /> }
                })?;
                Ok(())
            })
            .expect("declare two input windows");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open host");
        host.show("second").expect("show second");
        host.render().expect("render both");
        host
    }

    #[test]
    fn ime_composition_is_scoped_to_the_receiving_window() {
        let mut host = open_two_input_host();
        let main_ctx = host.windows["main"].ctx;
        let second_ctx = host.windows["second"].ctx;
        let main_id = host
            .runtime
            .presentation_of(main_ctx)
            .expect("main snapshot")
            .find_label("main-input")
            .expect("main input")
            .id();
        let second_id = host
            .runtime
            .presentation_of(second_ctx)
            .expect("second snapshot")
            .find_label("second-input")
            .expect("second input")
            .id();
        host.runtime.focus_node(main_id).expect("focus main");
        host.runtime.focus_node(second_id).expect("focus second");

        send_msg_in(
            &mut host,
            "second",
            MSG {
                message: WM_IME_STARTCOMPOSITION,
                ..MSG::default()
            },
        );

        let main = host
            .runtime
            .presentation_of(main_ctx)
            .expect("main snapshot after");
        let second = host
            .runtime
            .presentation_of(second_ctx)
            .expect("second snapshot after");
        assert!(
            main.node(main_id)
                .expect("main input")
                .edit()
                .expect("edit")
                .composition()
                .is_none(),
            "IME messages do not start a composition in another window"
        );
        assert!(
            second
                .node(second_id)
                .expect("second input")
                .edit()
                .expect("edit")
                .composition()
                .is_some(),
            "the receiving HWND starts its own composition"
        );
    }

    #[test]
    fn run_returns_when_no_window_is_live() {
        let mut host = open_host();
        host.close(MAIN).expect("close the only window");
        host.run().expect("run does not block without windows");
    }

    #[test]
    fn a_pending_wake_is_forwarded_when_its_target_is_removed() {
        let mut host = open_two_window_host();
        host.wake.pending.store(true, Ordering::SeqCst);
        host.remove_window("main");
        host.pump().expect("deliver the forwarded wake");
        assert!(
            !host.wake.is_pending(),
            "the forwarded wake is observed by the surviving window"
        );
    }

    #[test]
    fn wake_target_changes_are_reported_once() {
        let host = open_two_window_host();
        let first = host.hwnd("main").expect("the primary HWND");
        let second = host.hwnd("second").expect("the secondary HWND");
        let wake = wake_state();
        assert!(wake.set_target(Some(first)));
        assert!(!wake.set_target(Some(first)));
        assert!(wake.set_target(Some(second)));
        assert!(wake.set_target(None));
        assert!(!wake.is_pending());
    }

    #[test]
    fn opening_a_host_requires_a_declared_catalog() {
        let error = match Host::open(Runtime::new(), HostConfig::hidden()) {
            Ok(_) => panic!("a host needs a declared catalog"),
            Err(error) => error,
        };
        assert!(
            matches!(&error, HostError::Window(message) if message.contains("catalog")),
            "the host reports the missing catalog: {error}"
        );
    }

    #[test]
    fn device_loss_rebuilds_every_window_surface() {
        let mut host = open_two_window_host();
        let before_main = host.read_frame("main").expect("main frame");
        let before_second = host.read_frame("second").expect("second frame");

        host.device_lost = true;
        host.recover_device().expect("rebuild every surface");

        let main = host.read_frame("main").expect("main frame after");
        let second = host.read_frame("second").expect("second frame after");
        assert_eq!(main.width(), before_main.width());
        assert_eq!(second.width(), before_second.width());
        assert_eq!(main.pixels(), before_main.pixels());
        assert_eq!(second.pixels(), before_second.pixels());
        assert!(host.scene("main").is_some());
        assert!(host.scene("second").is_some());

        host.request_close("second").expect("close second");
        host.request_close("main").expect("close main");
        host.run().expect("the loop exits after the last window");
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
        let mut runtime = Runtime::new();
        zenui_demo::open_todo_window(&mut runtime).expect("open the todo window");
        zenui_demo::seed(&mut runtime).expect("seed the todo rows");
        Host::open(runtime, HostConfig::hidden()).expect("open host")
    }

    /// 在 primary 窗口上执行需要窗口与宿主共享资源的动作。
    fn with_main<R>(
        host: &mut Host,
        action: impl FnOnce(&mut Host, &mut WindowHost) -> Result<R, HostError>,
    ) -> R {
        host.with_window(MAIN, action).expect("window action")
    }

    /// 通过真实窗口过程同步派发一条消息，验证路由而不是直接调用处理函数。
    fn send_msg(host: &mut Host, message: MSG) {
        send_msg_in(host, MAIN, message);
    }

    fn send_msg_in(host: &mut Host, key: &'static str, message: MSG) {
        let hwnd = host.hwnd(key).expect("a live window");
        {
            let _pointer = HostPointer::install(host as *mut Host);
            // 安全：SendMessageW 在调用线程同步调用窗口过程；调用期间不使用 host。
            unsafe {
                SendMessageW(
                    hwnd,
                    message.message,
                    Some(message.wParam),
                    Some(message.lParam),
                );
            }
        }
        host.sync_windows().expect("sync windows");
        host.repaint_changed_windows().expect("repaint");
        if let Some(error) = host.deferred_error.take() {
            panic!("the window procedure failed: {error}");
        }
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

        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_A, false, true)
        });
        let snapshot = host.runtime.presentation().expect("snapshot");
        let edit = snapshot.node(node).expect("editor").edit().expect("edit");
        assert_eq!(edit.selection(), Some((0, 11)));
        assert_eq!(edit.anchor(), 0);
        assert_eq!(edit.caret(), 11);

        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_LEFT, true, false)
        });
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

        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_A, false, true)
        });
        with_main(&mut host, |host, window| {
            host.char_typed(window, WPARAM('X' as usize))
        });
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
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_TAB, true, false)
        });
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

        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_RETURN, false, false)
        });
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

        with_main(&mut host, |host, window| {
            host.char_typed(window, WPARAM(0x0009))
        });
        with_main(&mut host, |host, window| {
            host.char_typed(window, WPARAM(0xDC00))
        });
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

        with_main(&mut host, |host, window| {
            host.char_typed(window, WPARAM(0xD83D))
        });
        with_main(&mut host, |host, window| {
            host.char_typed(window, WPARAM(0xDE42))
        });
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
        with_main(&mut host, |host, window| host.ime_start(window));
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
        with_main(&mut host, |host, window| {
            host.apply_ime_composition(window, node, request(false, true), None, Some("ni".into()))
        });
        let snapshot = host.runtime.presentation().expect("snapshot");
        let element = snapshot.node(node).expect("editor");
        assert_eq!(element.text_value(), Some("Write tests"));
        let edit = element.edit().expect("edit");
        assert_eq!(edit.preedit(), Some("ni"));
        assert!(edit.composition().is_some());
    }

    #[test]
    fn a_wm_char_commits_the_active_composition() {
        let (mut host, node) = composing_host();
        with_main(&mut host, |host, window| {
            host.apply_ime_composition(
                window,
                node,
                request(false, true),
                None,
                Some("zhong".into()),
            )
        });
        with_main(&mut host, |host, window| {
            host.char_typed(window, WPARAM(u32::from('中') as usize))
        });

        let snapshot = host.runtime.presentation().expect("snapshot");
        let element = snapshot.node(node).expect("editor");
        assert_eq!(element.text_value(), Some("Write tests中"));
        let edit = element.edit().expect("edit");
        assert_eq!(edit.composition(), None);
        assert_eq!(edit.preedit(), None);
    }

    #[test]
    fn ime_commit_inserts_the_result() {
        let (mut host, node) = composing_host();
        with_main(&mut host, |host, window| {
            host.apply_ime_composition(window, node, request(true, false), Some("你".into()), None)
        });
        let snapshot = host.runtime.presentation().expect("snapshot");
        let element = snapshot.node(node).expect("editor");
        assert_eq!(element.text_value(), Some("Write tests你"));
        assert!(element.edit().expect("edit").composition().is_none());
    }

    #[test]
    fn ime_commit_then_update_starts_the_next_composition() {
        let (mut host, node) = composing_host();
        with_main(&mut host, |host, window| {
            host.apply_ime_composition(
                window,
                node,
                request(true, true),
                Some("你".into()),
                Some("ni".into()),
            )
        });
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
        with_main(&mut host, |host, window| {
            host.apply_ime_composition(window, node, request(true, false), None, None)
        });
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

        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_ESCAPE, false, false)
        });
        let snapshot = host.runtime.presentation().expect("snapshot");
        let element = snapshot.node(node).expect("editor");
        assert_eq!(element.text_value(), Some("Write tests"));
        assert!(
            element.edit().expect("edit").composition().is_none(),
            "Escape ends the composition"
        );
    }

    #[test]
    fn an_ime_consumed_key_cancels_a_local_composition() {
        let (mut host, node) = composing_host();
        host.runtime
            .compose_update_node(node, "ni")
            .expect("preedit");

        // 没有真实 IME 上下文时 OS 组合串为空；本地仍在组合，按 Escape 的
        // 静默结束路径补一次取消。
        with_main(&mut host, |host, window| {
            host.key_down(window, WPARAM(VK_PROCESSKEY.0 as usize))
        });
        let snapshot = host.runtime.presentation().expect("snapshot");
        let element = snapshot.node(node).expect("editor");
        assert_eq!(element.text_value(), Some("Write tests"));
        assert!(
            element.edit().expect("edit").composition().is_none(),
            "a local composition without an OS string is cancelled"
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
        send_msg(&mut host, message);
        assert!(
            !host.windows[MAIN].close_requested,
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
        with_main(&mut host, |host, window| host.ime_start(window));
        with_main(&mut host, |host, window| {
            host.ime_composition(window, LPARAM(GCS_COMPSTR.0 as isize))
        });
        with_main(&mut host, |host, window| host.ime_cancel(window));
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
        send_msg(&mut host, start);
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
        send_msg(&mut host, end);
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
        send_msg(&mut host, message);
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
        send_msg(&mut host, message);
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
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_TAB, false, false)
        });
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
        with_main(&mut host, |host, window| {
            host.pointer_pressed(window, LPARAM(0))
        });
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
