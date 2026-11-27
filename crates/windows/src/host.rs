use std::collections::BTreeMap;
use std::error::Error;
use std::ffi::c_void;
use std::fmt;
use std::mem::size_of;
use std::num::NonZeroIsize;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, ERROR_SUCCESS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT,
    POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, COLOR_BTNFACE, COLOR_BTNHIGHLIGHT, COLOR_BTNSHADOW, COLOR_GRAYTEXT,
    COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW, COLOR_WINDOWFRAME, COLOR_WINDOWTEXT,
    ClientToScreen, EndPaint, GetSysColor, PAINTSTRUCT, SYS_COLOR_INDEX, ValidateRect,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Accessibility::{
    HCF_HIGHCONTRASTON, HIGHCONTRASTW, IRawElementProviderSimple, UiaReturnRawElementProvider,
    UiaRootObjectId,
};
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
    EnableWindow, GetCapture, GetKeyState, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT,
    TrackMouseEvent, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE,
    VK_HOME, VK_LEFT, VK_MENU, VK_NEXT, VK_PRIOR, VK_PROCESSKEY, VK_RETURN, VK_RIGHT, VK_SHIFT,
    VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::Shell::NIN_SELECT;
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CheckMenuItem, CreateWindowExW, DefWindowProcW,
    DestroyWindow, DispatchMessageW, EnableMenuItem, GWLP_USERDATA, GetClientRect, GetCursorPos,
    GetForegroundWindow, GetMenu, GetMenuItemCount, GetMenuItemID, GetMessageW, GetWindowLongPtrW,
    HACCEL, HICON, HMENU, IDC_ARROW, IDI_APPLICATION, IsIconic, IsZoomed, KillTimer, LoadCursorW,
    LoadIconW, MF_BYCOMMAND, MF_CHECKED, MF_ENABLED, MF_GRAYED, MF_UNCHECKED, MSG, PM_REMOVE,
    PeekMessageW, PostMessageW, RegisterClassExW, RegisterWindowMessageW, SC_CLOSE,
    SPI_GETHIGHCONTRAST, SPI_GETWHEELSCROLLLINES, SW_HIDE, SW_RESTORE, SW_SHOW, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOZORDER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetForegroundWindow, SetTimer,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, SystemParametersInfoW, TranslateAcceleratorW,
    TranslateMessage, WINDOW_EX_STYLE, WM_CAPTURECHANGED, WM_CHAR, WM_CLOSE, WM_COMMAND,
    WM_CONTEXTMENU, WM_DESTROY, WM_DPICHANGED, WM_ENTERMENULOOP, WM_GETDPISCALEDSIZE, WM_GETOBJECT,
    WM_IME_CHAR, WM_IME_COMPOSITION, WM_IME_ENDCOMPOSITION, WM_IME_STARTCOMPOSITION, WM_INITMENU,
    WM_INITMENUPOPUP, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEHWHEEL,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NULL, WM_PAINT, WM_SETTINGCHANGE, WM_SIZE, WM_SYSCOMMAND,
    WM_SYSKEYDOWN, WM_TIMER, WNDCLASSEXW, WS_OVERLAPPEDWINDOW, WS_POPUP,
};

use windows::core::{HSTRING, w};
use zenui::prelude::*;

use crate::accessibility::{
    self, AccessibilityAction, AccessibilityIds, AccessibilityTree, ScrollCommand,
    WindowAccessibility,
};
use crate::clipboard::{ClipboardProvider, NativeClipboard, single_line_text};
use crate::context_menu::{ContextMenuPresenter, ContextMenuRequest, NativeContextMenuPresenter};
use crate::drag_drop::{DragDropError, DropRegistration, should_register};
use crate::file_dialog::{FileDialogPresenter, NativeFileDialog};
use crate::menus::{MenuError, NativeAccelerator, NativeMenu};
use crate::ole::{OleApartment, OleError};
use crate::shortcuts::{EditorAction, accelerator_is_shadowed, editor_shortcut};
use crate::tray::{
    NativeTrayPresenter, TrayIconRequest, TrayMenuRequest, TrayPresenter, TrayTooltipRequest,
};
use zenui_render::{
    GpuRenderer, HighContrastColors, Point, Rect, RenderPalette, RenderTargetConfig, Scene,
    SceneCache, SceneElement, SceneEnv, SceneRowBand, Scrollbar, TextAssets, Viewport, Visual,
    wgpu,
};

/// 宿主私有的窗口消息注册表；其他模块不得自行定义 `WM_APP + N`。
mod host_message {
    use windows::Win32::UI::WindowsAndMessaging::WM_APP;

    /// worker 唤醒；任一存活窗口收到后交付全部 worker 结果。
    pub(super) const WAKE: u32 = WM_APP + 1;
    /// 设备丢失恢复请求；回调与失败尝试的重试都投递它。
    pub(super) const DEVICE_LOSS: u32 = WM_APP + 2;
    /// 辅助技术动作请求，保留给 UIA 工单。
    pub(super) const UIA_ACTION: u32 = WM_APP + 3;
    /// 托盘回调；托盘 HWND 在窗口过程里使用自己的路由分支。
    pub(super) const TRAY_CALLBACK: u32 = WM_APP + 4;
    /// OLE drop target 已把文件路径排入窗口队列。
    pub(super) const DROP: u32 = WM_APP + 5;

    /// 注册表全集；窗口过程与 `handle_message` 显式认领它们，保留值不会落入
    /// 窗口输入路由。
    pub(super) const ALL: [u32; 5] = [WAKE, DEVICE_LOSS, UIA_ACTION, TRAY_CALLBACK, DROP];
}

/// `NIN_KEYSELECT`；windows crate 只生成了相邻的 `NIN_SELECT`。
const NIN_KEYSELECT: u32 = NIN_SELECT + 1;

/// 唤醒窗口的 UI 线程以执行排队的辅助技术动作。
///
/// provider 线程只入队；投递失败时入队方撤回动作并向 UIA 报告不可用。
pub(crate) fn post_uia_action(hwnd: HWND) -> bool {
    // 安全：HWND 由窗口条目持有；消息只排队，不跨线程解引用。
    unsafe { PostMessageW(Some(hwnd), host_message::UIA_ACTION, WPARAM(0), LPARAM(0)).is_ok() }
}

/// 唤醒窗口的 UI 线程，drain OLE drop target 排队的路径。
pub(crate) fn post_drop_drain(hwnd: HWND) -> bool {
    // 安全：HWND 由窗口条目持有；消息只排队，不跨线程解引用。
    unsafe { PostMessageW(Some(hwnd), host_message::DROP, WPARAM(0), LPARAM(0)).is_ok() }
}

/// `SPI_GETWHEELSCROLLLINES` 的哨兵值：一个 notch 滚动一页。
const WHEEL_PAGESCROLL: u32 = u32::MAX;
/// 查询失败时的缺省行数；与 Windows 的常用缺省一致。
const DEFAULT_WHEEL_LINES: u32 = 3;
/// 一个 notch 的滚轮单位。
const WHEEL_NOTCH_UNITS: i64 = 120;
/// ADR-0010 的一行逻辑像素高度。
const WHEEL_LINE_HEIGHT: i64 = 24;

/// 当前系统设置的滚轮行数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WheelLines {
    /// 系统设置关闭了滚轮滚动。
    Disabled,
    /// 一个 notch 滚动固定行数。
    Lines(u32),
    /// 一个 notch 滚动一页；页高在应用偏移时传入。
    Page,
}

impl WheelLines {
    /// 一个 notch 的逻辑像素；`page` 只由 `Page` 使用。
    fn notch_pixels(self, page: i32) -> i64 {
        match self {
            Self::Disabled | Self::Lines(0) => 0,
            Self::Lines(lines) => i64::from(lines) * WHEEL_LINE_HEIGHT,
            Self::Page => i64::from(page.max(1)),
        }
    }
}

/// 一个 activation 的滚轮累积器与锚点。
///
/// 单位是 1/120 notch 的逻辑像素；正值表示向前滚动（内容向上）。
#[derive(Clone, Copy, Debug)]
struct WheelScroll {
    lines: WheelLines,
    accumulator: i64,
    anchor: i64,
}

impl WheelScroll {
    fn from_system() -> Self {
        Self {
            lines: system_wheel_lines(),
            accumulator: 0,
            anchor: 0,
        }
    }

    fn reset(&mut self) {
        self.accumulator = 0;
        self.anchor = 0;
    }

    /// 行数设置变化时丢弃未应用的 residue。
    fn set_lines(&mut self, lines: WheelLines) {
        if lines != self.lines {
            self.lines = lines;
            self.reset();
        }
    }

    /// 返回本事件应移动的逻辑像素；正向 notch 使 offset 减小。
    ///
    /// `page` 是 `WheelLines::Page` 使用的逻辑页高。
    fn step(&mut self, delta: i32, page: i32) -> i32 {
        if matches!(self.lines, WheelLines::Disabled | WheelLines::Lines(0)) {
            self.reset();
            return 0;
        }
        let notch = self.lines.notch_pixels(page);
        self.accumulator = self
            .accumulator
            .saturating_add(i64::from(delta).saturating_mul(notch));
        let pixels = self.accumulator / WHEEL_NOTCH_UNITS;
        let step = pixels.saturating_sub(self.anchor);
        self.anchor = pixels;
        step.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    }

    /// UIA 小步长的行像素；与一个滚轮 notch 使用同一系统设置。
    fn line_step(&self, page: i32) -> i32 {
        i32::try_from(self.lines.notch_pixels(page)).unwrap_or(i32::MAX)
    }
}

fn system_wheel_lines() -> WheelLines {
    let mut lines = DEFAULT_WHEEL_LINES;
    // 安全：pvParam 指向本地 u32；SPI_GETWHEELSCROLLLINES 只写入该值。
    let result = unsafe {
        SystemParametersInfoW(
            SPI_GETWHEELSCROLLLINES,
            0,
            Some((&raw mut lines).cast::<c_void>()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if result.is_err() {
        return WheelLines::Lines(DEFAULT_WHEEL_LINES);
    }
    wheel_lines_from(lines)
}

/// 把 `SPI_GETWHEELSCROLLLINES` 的原始值映射为滚动单位。
fn wheel_lines_from(lines: u32) -> WheelLines {
    match lines {
        0 => WheelLines::Disabled,
        WHEEL_PAGESCROLL => WheelLines::Page,
        lines => WheelLines::Lines(lines),
    }
}

/// caret 闪烁定时器；OS 原生编辑控件的缺省周期约为 530ms。
const CARET_BLINK_TIMER: usize = 1;
const CARET_BLINK_MS: u32 = 530;

/// Windows 系统外观偏好：高对比度与辅助功能文本缩放。
///
/// 这是宿主输入，不是应用状态，也不进入呈现快照或应用事务。文本缩放的有效
/// 范围是 100–225；越界值在构造时回退到 100。文本缩放不是显示 DPI：显示器
/// 缩放仍由窗口 viewport 的 scale 承担。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemPreferences {
    high_contrast: bool,
    text_scale_percent: u16,
}

impl SystemPreferences {
    /// 构造偏好；`text_scale_percent` 不在 100–225 时使用 100。
    pub const fn new(high_contrast: bool, text_scale_percent: u16) -> Self {
        Self {
            high_contrast,
            text_scale_percent: match text_scale_percent {
                100..=225 => text_scale_percent,
                _ => 100,
            },
        }
    }

    /// 系统是否启用高对比度。
    pub const fn high_contrast(self) -> bool {
        self.high_contrast
    }

    /// 文本缩放百分比，始终在 100–225 之间。
    pub const fn text_scale_percent(self) -> u16 {
        self.text_scale_percent
    }
}

impl Default for SystemPreferences {
    /// 高对比度关闭、文本缩放 100%；隐藏宿主与缺失系统值时使用。
    fn default() -> Self {
        Self::new(false, 100)
    }
}

/// 采样当前 Windows 偏好；读取失败或值缺失时使用默认值。
fn sample_system_preferences() -> SystemPreferences {
    SystemPreferences::new(high_contrast_enabled(), text_scale_percent())
}

fn high_contrast_enabled() -> bool {
    let mut high_contrast = HIGHCONTRASTW {
        cbSize: size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    // 安全：cbSize 与缓冲区一致，API 只写入这个结构。
    let result = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            high_contrast.cbSize,
            Some((&mut high_contrast as *mut HIGHCONTRASTW).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS::default(),
        )
    };
    result.is_ok() && high_contrast.dwFlags.contains(HCF_HIGHCONTRASTON)
}

fn text_scale_percent() -> u16 {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    // 安全：键名与值名是静态字符串，输出缓冲区与长度匹配。
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Accessibility"),
            w!("TextScaleFactor"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    };
    if status == ERROR_SUCCESS && size == size_of::<u32>() as u32 {
        u16::try_from(value).unwrap_or(100)
    } else {
        100
    }
}

fn high_contrast_colors() -> HighContrastColors {
    HighContrastColors {
        window: system_color(COLOR_WINDOW),
        window_text: system_color(COLOR_WINDOWTEXT),
        button_face: system_color(COLOR_BTNFACE),
        button_highlight: system_color(COLOR_BTNHIGHLIGHT),
        button_shadow: system_color(COLOR_BTNSHADOW),
        gray_text: system_color(COLOR_GRAYTEXT),
        highlight: system_color(COLOR_HIGHLIGHT),
        highlight_text: system_color(COLOR_HIGHLIGHTTEXT),
        window_frame: system_color(COLOR_WINDOWFRAME),
    }
}

fn system_color(index: SYS_COLOR_INDEX) -> [u8; 4] {
    // 安全：GetSysColor 只读取当前主题的缓存颜色，没有指针参数。
    let value = unsafe { GetSysColor(index) };
    [
        (value & 0xFF) as u8,
        ((value >> 8) & 0xFF) as u8,
        ((value >> 16) & 0xFF) as u8,
        255,
    ]
}

/// 打开宿主的进程级选项；每个窗口的标题与尺寸来自窗口目录。
#[derive(Clone, Debug)]
pub struct HostConfig {
    /// 是否在 `run` 时显示已打开的窗口；隐藏模式供 CI smoke 使用。
    pub visible: bool,
    /// 应用的初始外观。
    pub appearance: Appearance,
    /// 固定字体与分词语料；`None` 沿用系统字体集合。
    pub text_assets: Option<TextAssets>,
    /// 初始系统偏好；`None` 时可见宿主采样当前系统，隐藏宿主使用默认值。
    pub system_preferences: Option<SystemPreferences>,
}

impl HostConfig {
    /// 显示所有已打开的窗口。
    pub const fn visible() -> Self {
        Self {
            visible: true,
            appearance: Appearance::Light,
            text_assets: None,
            system_preferences: None,
        }
    }

    /// 创建窗口与 surface，但不调用 `ShowWindow`；供隐藏 smoke 使用。
    pub const fn hidden() -> Self {
        Self {
            visible: false,
            appearance: Appearance::Light,
            text_assets: None,
            system_preferences: None,
        }
    }

    /// 设置初始应用外观。
    pub const fn with_appearance(mut self, appearance: Appearance) -> Self {
        self.appearance = appearance;
        self
    }

    /// 使用应用提供的字体与分词语料；无效资产让 [`Host::open`] 在创建窗口前失败。
    #[must_use]
    pub fn with_text_assets(mut self, assets: TextAssets) -> Self {
        self.text_assets = Some(assets);
        self
    }

    /// 使用指定的系统偏好，而不是在可见宿主启动时采样。
    ///
    /// 隐藏宿主默认使用 [`SystemPreferences::default`]，不读取真实系统设置。
    pub const fn with_system_preferences(mut self, preferences: SystemPreferences) -> Self {
        self.system_preferences = Some(preferences);
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

/// 当前设备代的丢失状态；代际、丢失位与丢失序号必须一起读写。
struct DeviceLossState {
    generation: u64,
    lost: bool,
    /// 每次新的丢失递增；恢复只在序号未变时清除信号。
    epoch: u64,
}

/// 当前设备代的丢失信号。
///
/// wgpu 的设备丢失回调在任意线程运行，只允许置位并投递一条合并消息；
/// 回调不接触 Host、Runtime、surface 或 scene。代际号让旧设备的迟到回调
/// 无法触发新设备的恢复。
struct DeviceLossSignal {
    state: Mutex<DeviceLossState>,
    pending: AtomicBool,
    /// 当前接收 `DEVICE_LOSS` 的 HWND 值；零表示没有可投递的窗口。
    target: AtomicIsize,
}

impl DeviceLossSignal {
    fn new() -> Self {
        Self {
            state: Mutex::new(DeviceLossState {
                generation: 0,
                lost: false,
                epoch: 0,
            }),
            pending: AtomicBool::new(false),
            target: AtomicIsize::new(0),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, DeviceLossState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn set_generation(&self, generation: u64) {
        self.lock().generation = generation;
        // 上一代的在途消息不再代表新设备；新回调会重新投递。
        self.pending.store(false, Ordering::SeqCst);
    }

    fn raise(&self, generation: u64) {
        let accepted = {
            let mut state = self.lock();
            if state.generation != generation {
                false
            } else {
                state.lost = true;
                state.epoch = state.epoch.wrapping_add(1);
                true
            }
        };
        if accepted {
            self.post(generation);
        }
    }

    fn mark_lost(&self) {
        let mut state = self.lock();
        state.lost = true;
        state.epoch = state.epoch.wrapping_add(1);
    }

    fn epoch(&self) -> u64 {
        self.lock().epoch
    }

    /// 回调驱动的投递；旧代设备的迟到回调被信号代际淘汰。
    fn post(&self, generation: u64) {
        if self.lock().generation != generation {
            return;
        }
        self.post_from_host(generation);
    }

    /// 宿主驱动的投递；没有存活目标时保留信号，等目标出现。
    ///
    /// 失败的 staging 会先把信号代际推进到未发布的设备，因此这里不按信号
    /// 代际过滤；`generation` 是宿主已发布的代际，派发端用它淘汰重建完成后
    /// 的迟到消息。
    fn post_from_host(&self, generation: u64) {
        if self.pending.swap(true, Ordering::SeqCst) {
            return;
        }
        let Some(target) = self.target() else {
            self.pending.store(false, Ordering::SeqCst);
            return;
        };
        // 安全：PostMessageW 是线程安全的；失败只释放合并标志，信号仍在。
        if unsafe {
            PostMessageW(
                Some(target),
                host_message::DEVICE_LOSS,
                WPARAM(generation as usize),
                LPARAM(0),
            )
        }
        .is_err()
        {
            self.pending.store(false, Ordering::SeqCst);
        }
    }

    fn observed(&self) {
        self.pending.store(false, Ordering::SeqCst);
    }

    /// 只有丢失序号仍等于 `epoch` 时清除；恢复期间的新丢失不会被吞掉。
    fn clear_if(&self, epoch: u64) -> bool {
        let mut state = self.lock();
        if state.epoch != epoch {
            return false;
        }
        state.lost = false;
        self.observed();
        true
    }

    fn is_lost(&self) -> bool {
        self.lock().lost
    }

    fn set_target(&self, hwnd: Option<HWND>) -> bool {
        let previous = self
            .target
            .swap(hwnd.map_or(0, |hwnd| hwnd.0 as isize), Ordering::SeqCst);
        previous != self.target.load(Ordering::SeqCst)
    }

    fn target(&self) -> Option<HWND> {
        let value = self.target.load(Ordering::SeqCst);
        (value != 0).then_some(HWND(value as *mut core::ffi::c_void))
    }
}

/// 为新 device 安装丢失回调，并让信号接受这一代。
///
/// 调用发生在 surface 配置与渲染器创建之前：staging 期间的丢失也必须进入
/// 信号。每次尝试安装自己的代际；staging 失败时丢弃设备的迟到回调不会与
/// 之后发布的设备代际重合。
fn install_loss_callback(device: &wgpu::Device, loss: &Arc<DeviceLossSignal>, generation: u64) {
    loss.set_generation(generation);
    let loss = Arc::clone(loss);
    device.set_device_lost_callback(move |_reason, _message| loss.raise(generation));
}

/// 设备丢失后的宿主阶段；丢失信号跨重建保留，阶段决定是否还欠一次呈现。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LossPhase {
    /// 没有未处理的丢失。
    Clear,
    /// 信号已置位，等待下一次重建。
    Pending,
    /// 已重建但还没有成功呈现；此时不重复创建设备。
    AwaitingPresent,
}

/// `add_window` 的结果；设备在重建期间又丢失时把请求留到下一次同步。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowAdd {
    Added,
    Deferred,
}

/// 一个窗口的原生菜单栏与同一代快捷键表。
///
/// 两者共享命令 id 映射与 activation 生命周期：窗口关闭时一起释放，重新打开
/// 必须从新的目录声明重建。
struct WindowMenu {
    menu: NativeMenu,
    accelerator: Option<NativeAccelerator>,
}

impl WindowMenu {
    fn build(menu: NativeMenu) -> Result<Self, MenuError> {
        let accelerator = NativeAccelerator::build(&menu)?;
        Ok(Self { menu, accelerator })
    }

    fn command(&self, id: u16) -> Option<&MenuCommand> {
        self.menu.command(id)
    }

    fn accelerator(&self) -> Option<&NativeAccelerator> {
        self.accelerator.as_ref()
    }
}

/// 一个窗口的全部宿主状态；`Host` 为每个存活窗口持有一个条目。
struct WindowHost {
    // 字段顺序是安全约束：surface 必须先于 HWND 释放，因为它的安全契约
    // 要求创建 surface 的 HWND 在 surface 存活期间保持有效。
    surface: Option<wgpu::Surface<'static>>,
    surface_config: wgpu::SurfaceConfiguration,
    /// 可见且有 `on_drop` 的窗口的 OLE drop target；在 HWND 销毁前撤销。
    drop_target: Option<DropRegistration>,
    native: NativeWindow,
    /// 命令 id 映射、菜单句柄与同一代快捷键表；随着窗口条目一起释放。
    menu: Option<WindowMenu>,
    ctx: WindowCtx,
    scene: Option<Scene>,
    pointer: Option<Point>,
    hovered: Option<NodeId>,
    pressed: Option<NodeId>,
    release_capture: bool,
    tracking_leave: bool,
    keyboard_modality: bool,
    pending_high_surrogate: Option<u16>,
    /// 被用户 keydown 消费/跳过默认后，等待丢弃的同一次按键的 WM_CHAR。
    char_suppression: Option<CharSuppression>,
    caret_visible: bool,
    blink_timer: Option<usize>,
    close_requested: bool,
    /// 原生 HWND 是否启用；待决关闭的父窗口保持禁用，直到 Deny。
    enabled: bool,
    /// 当前 activation 采用的逻辑像素滚动偏移；关闭后随 activation 释放。
    scroll_offset: i32,
    /// 滚轮累积器与锚点；行数设置在 activation 内缓存。
    wheel: WheelScroll,
    /// 最近一次构建的逻辑 viewport；resize/DPI 变化时丢弃滚轮 residue。
    last_viewport: Option<Viewport>,
    /// 滚动触发的重建只更新 hover，不清除 capture-based 的按压目标。
    preserve_press_on_next_rebuild: bool,
    /// 滚动条 thumb 拖动中指针相对 thumb 顶部的逻辑偏移；捕获与它同生共死。
    scrollbar_drag: Option<f32>,
    /// 下一次构建前把聚焦元素或 caret 显露到视口内；显式滚动会清除它。
    reveal_requested: bool,
    /// 指针、按压、模态或 caret 等瞬时状态变化需要重绘；它们不进入快照。
    ///
    /// 只有成功呈现才清除；`Timeout` / `Occluded` / `Lost` 与零尺寸保留它，
    /// 下一次绘制重试同一份 damage。
    paint_requested: bool,
    /// 最近一次构建的呈现快照；未变化的窗口跳过重绘。
    ///
    /// 场景构建发生在获取 surface 图像之前，所以跳过的帧可能让这里比屏幕上
    /// 的帧更新；只有成功呈现才提交 `scene`。
    last_snapshot: Option<PresentationSnapshot>,
    /// `last_snapshot` 对应的运行时呈现版本；只在同步快照缓存时前进。
    snapshot_revision: u64,
    /// 目录标题；UIA 根节点的名称。
    title: String,
    /// 当前发布的可访问性树；provider 通过弱引用解析，随窗口条目一起释放。
    accessibility: Arc<WindowAccessibility>,
    /// UIA 运行时 ID 分配器；与发布树一起更新。
    accessibility_ids: AccessibilityIds,
    /// 测试注入的 OS 组合状态；真实宿主读取 HIMC 的组合串长度。
    #[cfg(test)]
    os_composition: Option<bool>,
    /// 测试期计入的 `ImmNotifyIME` 取消次数；真实宿主只调用系统 API。
    #[cfg(test)]
    ime_cancel_notifications: usize,
    /// 测试期代替 `GetForegroundWindow`；隐藏窗口无法成为前台窗口。
    #[cfg(test)]
    foreground: Option<bool>,
}

impl WindowHost {
    const fn hwnd(&self) -> HWND {
        self.native.hwnd
    }
}

/// 提交场景并整体替换窗口的可访问性树。
///
/// 树与场景来自同一份快照，因此名称、状态和边界始终对应屏幕上这一帧。
/// 所有呈现路径都经这里提交，避免新增路径只更新其中一个发布物。
fn commit_scene(window: &mut WindowHost, snapshot: &PresentationSnapshot, scene: Scene) {
    let tree = accessibility::build_tree(
        snapshot,
        &scene,
        &window.title,
        window.enabled,
        &mut window.accessibility_ids,
    );
    window.accessibility.publish(tree);
    window.scene = Some(scene);
}

/// 从 Host 取出的一次弹窗工作；presenter 在调用期间不持有 Host 引用。
struct FileDialogWork {
    presenter: Box<dyn FileDialogPresenter>,
    /// presenter 来自 Host 的注入槽，调用后需要放回。
    restore: bool,
    hwnd: HWND,
    options: FileDialogOptions,
}

/// 测试期渲染计数；空闲窗口不应重建场景或呈现（caret 闪烁只呈现缓存场景）。
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RenderCounters {
    scene_builds: usize,
    presents: usize,
}

/// 可恢复诊断的保留上限；最旧的先丢弃。
const DIAGNOSTIC_LIMIT: usize = 32;

/// Win32 宿主：一个 Runtime、一个消息循环，每个存活窗口一个 HWND/Surface/Scene。
pub struct Host {
    runtime: Runtime,
    _instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: GpuRenderer,
    renderer_target: RenderTargetConfig,
    /// 消息循环重复构建场景时复用未变化的文本 layout。
    scene_cache: SceneCache,
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
    /// 宿主当前采用的 Windows 系统偏好；可见宿主在启动与设置变化时采样。
    system_preferences: SystemPreferences,
    /// 测试注入的设置变化采样；真实宿主直接读取系统。
    #[cfg(test)]
    settings_sample: Option<SystemPreferences>,
    /// 仅在 `run`/`pump` 派发期间指向 self；窗口过程通过 GWLP_USERDATA 取回宿主。
    dispatching: Option<*mut Host>,
    /// 宿主正在执行借出动作；此时的同步窗口消息不得再次进入窗口过程。
    routing_suppressed: bool,
    /// 设备丢失后的一次恢复正在进行，避免递归重建。
    recovering: bool,
    /// 当前设备代的丢失信号；wgpu 回调与宿主之间只通过它通信。
    loss: Arc<DeviceLossSignal>,
    /// 丢失信号的处理阶段；重建后仍需第一次成功呈现。
    loss_phase: LossPhase,
    /// 设备代际；每次新 device 递增，旧回调按它淘汰。
    device_generation: u64,
    /// 下一个待安装的设备代际；每次安装回调时前进，失败的 staging 也消耗一个，
    /// 避免被丢弃设备的迟到回调命中后来发布的设备。
    next_device_generation: u64,
    /// `AwaitingPresent` 对应的丢失序号；呈现只清除同一序号的信号。
    loss_epoch: u64,
    /// 本次恢复开始后是否已经成功呈现过一帧。
    presented_since_rebuild: bool,
    /// 本轮丢失事件已经消耗的恢复尝试次数；完整证明性呈现后归零。
    recovery_attempts: u32,
    /// 最近一次重建尝试拿到的适配器；失败时用于终止错误。
    attempt_adapter: Option<GpuAdapterInfo>,
    /// 达到尝试上限后的终止错误；存在时不再创建设备。
    latched_loss: Option<DeviceLostError>,
    /// 测试注入的 surface 状态队列；只存在于测试构建。
    #[cfg(test)]
    forced_surface_statuses: std::collections::VecDeque<wgpu::CurrentSurfaceTexture>,
    /// 测试期场景构建与呈现次数；真实宿主不需要这组计数。
    #[cfg(test)]
    counters: RenderCounters,
    /// 测试注入的重建失败次数；只存在于测试构建。
    #[cfg(test)]
    injected_rebuild_failures: usize,
    /// 窗口过程不能返回错误；消息循环在派发后取走并返回。
    deferred_error: Option<HostError>,
    /// 可恢复的宿主诊断（行工厂拒绝新窗口等）；致命错误仍走 `deferred_error`。
    diagnostics: Vec<String>,
    /// 关闭协议改变了焦点/启用状态，消息循环在下次同步后重定向前台。
    focus_after_sync: bool,
    /// 测试或嵌入方注入的文件选择器；`None` 时使用原生对话框或 Unsupported。
    file_dialog_presenter: Option<Box<dyn FileDialogPresenter>>,
    /// 原生弹窗期间拒绝替换 presenter；只在 UI 线程读写。
    file_dialog_active: bool,
    /// 窗口级上下文菜单 presenter；隐藏模式默认没有，避免 CI 打开原生弹窗。
    context_menu_presenter: Option<Box<dyn ContextMenuPresenter>>,
    /// 上下文菜单弹窗正在展示；期间的菜单初始化不属于窗口菜单栏。
    context_menu_active: bool,
    /// 应用托盘；隐藏模式也保留自己的顶层 HWND 以接住回调广播。
    tray: Option<TrayHost>,
    /// 可见宿主默认安装原生 presenter；隐藏模式由测试注入。
    tray_presenter: Option<Box<dyn TrayPresenter>>,
    /// Explorer 重建通知区域时广播的注册消息；0 表示没有声明托盘。
    taskbar_created: u32,
    /// 宿主剪贴板 seam；可见宿主安装原生 provider，隐藏宿主默认没有。
    clipboard: Option<Box<dyn ClipboardProvider>>,
    /// OLE 初始化失败的稳定原因；避免每次请求重试并保留显式错误。
    ole_error: Option<OleError>,
    /// 必须最后声明：最后一个字段最后释放，OLE 生命周期覆盖 presenter。
    ole: Option<OleApartment>,
}

impl Host {
    /// 从一个已声明窗口目录的 Runtime 创建宿主；primary 未打开时由宿主首次显示。
    ///
    /// 每个存活窗口创建一个 HWND 与 DX12 surface；device、queue 与 renderer
    /// 在窗口之间共享。优先硬件适配器，失败时回退 WARP。
    ///
    /// `StayResident` 会话必须同时声明托盘，否则返回
    /// [`HostError::Session`] 的 [`SessionError::TrayRequired`]；托盘命令缺少
    /// 应用级处理器时返回 [`HostError::Tray`]。
    pub fn open(mut runtime: Runtime, config: HostConfig) -> Result<Self, HostError> {
        // 安全：进程级 DPI 设置；重复设置失败时保持已有策略。
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        let primary = runtime
            .primary_key()
            .ok_or_else(|| HostError::Window(String::from("the window catalog is not declared")))?;
        if runtime.session_policy() == SessionPolicy::StayResident && runtime.tray().is_none() {
            return Err(HostError::Session(SessionError::TrayRequired));
        }
        if let Some(id) = runtime.missing_tray_handler() {
            return Err(HostError::Tray(format!(
                "tray command {id} has no application-level handler"
            )));
        }
        runtime
            .set_appearance(config.appearance)
            .map_err(HostError::Runtime)?;
        // 可见宿主在启动时采样真实偏好；隐藏宿主只接受注入值，避免测试
        // 依赖或改动运行机器的辅助功能设置。
        let system_preferences = config.system_preferences.unwrap_or_else(|| {
            if config.visible {
                sample_system_preferences()
            } else {
                SystemPreferences::default()
            }
        });
        // 资产在创建任何原生窗口之前注册并校验；坏资产是启动错误，不是绘制错误。
        let scene_cache = match config.text_assets {
            Some(assets) => SceneCache::with_text_assets(assets).map_err(HostError::TextAssets)?,
            None => SceneCache::new(),
        };
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

        let menu = build_window_menu(&runtime, primary_info.window)?;
        let (window, menu) = NativeWindow::new(primary_info.spec, hinstance, menu)?;
        let menu = complete_window_menu(menu)?;
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
        // 回调在配置与渲染器创建之前安装：staging 期间的丢失也必须进入信号，
        // 而不是在第一次取纹理时被当成 API misuse。
        let loss = Arc::new(DeviceLossSignal::new());
        install_loss_callback(&device, &loss, 1);
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
                if unsafe { PostMessageW(Some(target), host_message::WAKE, WPARAM(0), LPARAM(0)) }
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
            scene_cache,
            format,
            hinstance,
            wake,
            windows: BTreeMap::new(),
            hwnds: BTreeMap::new(),
            revision: 0,
            show_windows: config.visible,
            system_preferences,
            #[cfg(test)]
            settings_sample: None,
            dispatching: None,
            routing_suppressed: false,
            recovering: false,
            loss,
            loss_phase: LossPhase::Clear,
            device_generation: 1,
            next_device_generation: 1,
            loss_epoch: 0,
            presented_since_rebuild: false,
            recovery_attempts: 0,
            attempt_adapter: None,
            latched_loss: None,
            #[cfg(test)]
            forced_surface_statuses: std::collections::VecDeque::new(),
            #[cfg(test)]
            counters: RenderCounters::default(),
            #[cfg(test)]
            injected_rebuild_failures: 0,
            deferred_error: None,
            diagnostics: Vec::new(),
            focus_after_sync: false,
            file_dialog_presenter: None,
            file_dialog_active: false,
            context_menu_presenter: config
                .visible
                .then(|| Box::new(NativeContextMenuPresenter) as Box<dyn ContextMenuPresenter>),
            context_menu_active: false,
            tray: None,
            tray_presenter: None,
            taskbar_created: 0,
            clipboard: config
                .visible
                .then(|| Box::new(NativeClipboard) as Box<dyn ClipboardProvider>),
            ole_error: None,
            ole: None,
        };
        host.insert_window(WindowHost::new(
            window,
            menu,
            primary_info.window,
            primary_info.spec.title().to_owned(),
            surface,
            surface_config,
            blink_timer,
        ));
        let mut deferred = false;
        for info in &infos {
            if info.spec.key() != primary && host.add_window(info)? == WindowAdd::Deferred {
                deferred = true;
                break;
            }
        }
        if !deferred {
            host.revision = host.runtime.window_revision();
        }
        host.update_wake_target();
        host.install_tray()?;
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

    /// 当前 device 使用的适配器；用于日志、验收记录与设备丢失诊断。
    ///
    /// `fallback` 表示软件回退（WARP），由 `device_type` 为 `Cpu` 判定。该信息
    /// 不参与渲染能力协商。
    pub fn adapter_info(&self) -> GpuAdapterInfo {
        adapter_info_of(&self.adapter)
    }

    /// 宿主当前采用的系统偏好。
    ///
    /// 可见宿主在启动与 `WM_SETTINGCHANGE` 时采样；隐藏宿主默认不读取系统设置。
    pub const fn system_preferences(&self) -> SystemPreferences {
        self.system_preferences
    }

    /// 替换宿主的系统偏好；变化让所有存活窗口在下一帧重绘。
    ///
    /// 偏好属于宿主输入，不修改应用模型、不触发 `on:input`，也不改变呈现版本。
    pub fn set_system_preferences(&mut self, preferences: SystemPreferences) {
        self.refresh_system_preferences(preferences);
    }

    fn refresh_system_preferences(&mut self, preferences: SystemPreferences) -> bool {
        if self.system_preferences == preferences {
            return false;
        }
        self.system_preferences = preferences;
        for window in self.windows.values_mut() {
            window.paint_requested = true;
        }
        true
    }

    fn current_system_preferences(&self) -> SystemPreferences {
        #[cfg(test)]
        {
            if let Some(preferences) = self.settings_sample {
                return preferences;
            }
        }
        sample_system_preferences()
    }

    /// 指定窗口当前已提交的呈现描述；嵌入方与测试可据此按 Node ID 或标签定位。
    ///
    /// 场景几何与 `scene(key)` 使用同一份快照；隐藏窗口和已关闭窗口返回错误。
    pub fn presentation(&self, key: &str) -> Result<PresentationSnapshot, HostError> {
        let ctx = self.window_ctx(key)?;
        self.runtime
            .presentation_of(ctx)
            .map_err(HostError::Snapshot)
    }

    fn window_ctx(&self, key: &str) -> Result<WindowCtx, HostError> {
        self.windows
            .get(key)
            .map(|window| window.ctx)
            .ok_or_else(|| HostError::UnknownWindow(key.to_owned()))
    }

    /// 显示目录窗口：未打开时挂载并创建 HWND；已打开时聚焦或从隐藏恢复。
    pub fn show(&mut self, key: &'static str) -> Result<WindowCtx, HostError> {
        let window = self.runtime.show(key).map_err(HostError::Open)?;
        self.sync_windows()?;
        self.focus_native_focused_window()?;
        self.repaint_changed_windows()?;
        Ok(window)
    }

    /// 隐藏窗口：保留 activation 与本地状态，只隐藏原生窗口并停止输入/呈现。
    pub fn hide(&mut self, key: &str) -> Result<(), HostError> {
        let ctx = self.window_ctx(key)?;
        self.runtime.hide(ctx).map_err(HostError::Close)?;
        self.sync_windows()?;
        self.focus_native_focused_window()?;
        self.repaint_changed_windows()
    }

    /// 关闭窗口并释放它的 activation；其他窗口不受影响。
    pub fn close(&mut self, key: &str) -> Result<(), HostError> {
        let ctx = self.window_ctx(key)?;
        self.runtime.close_window(ctx).map_err(HostError::Close)?;
        self.sync_windows()?;
        self.focus_native_focused_window()?;
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

    /// 安装测试或嵌入方的文件选择器；未安装时可见宿主使用原生对话框。
    ///
    /// 注入的 presenter 会收到打开、保存与选择文件夹请求；它必须按
    /// [`FileDialogOptions::kind`] 返回合法路径数量。原生弹窗期间拒绝替换。
    /// 隐藏宿主没有注入 presenter 时，请求以 `Unsupported` 完成，不会打开
    /// 真实对话框。
    ///
    /// # Errors
    ///
    /// 弹窗进行中时返回 [`HostError::Window`]。
    pub fn set_file_dialog_presenter(
        &mut self,
        presenter: impl FileDialogPresenter + 'static,
    ) -> Result<(), HostError> {
        if self.file_dialog_active {
            return Err(HostError::Window(String::from(
                "cannot replace the file dialog presenter while a dialog is active",
            )));
        }
        self.file_dialog_presenter = Some(Box::new(presenter));
        Ok(())
    }

    /// 安装或替换剪贴板 provider。
    ///
    /// 可见宿主默认安装原生 `CF_UNICODETEXT` provider；隐藏宿主默认没有，
    /// 因此 CI 不触碰系统剪贴板。读失败、写失败与 provider panic 都被宿主
    /// 当作静默无操作，不产生 [`HostError`]。
    pub fn set_clipboard_provider(&mut self, provider: impl ClipboardProvider + 'static) {
        self.clipboard = Some(Box::new(provider));
    }

    /// 替换窗口级上下文菜单 presenter。
    ///
    /// 隐藏宿主默认没有 presenter，因此 CI 不会打开原生弹窗；测试或嵌入方可以
    /// 注入 fake 来记录请求并返回命令。presenter 在菜单关闭后归还宿主。弹窗由
    /// 本线程的模态循环持有，调用方无法在展示期间取得 `&mut Host`，因此不需要
    /// 运行时拒绝替换。
    pub fn set_context_menu_presenter(&mut self, presenter: impl ContextMenuPresenter + 'static) {
        self.context_menu_presenter = Some(Box::new(presenter));
    }

    /// 替换托盘 presenter；隐藏宿主不会自动安装原生通知区域。
    ///
    /// 已声明的托盘在安装时立即收到一次图标添加与 tooltip 更新；替换已有
    /// presenter 时先移除旧图标。隐藏测试可以用 fake 记录这些调用，CI 不会
    /// 触碰真实通知区域。没有托盘声明时只保存 presenter，不产生 shell 调用。
    ///
    /// # Errors
    ///
    /// presenter 的添加或 tooltip 失败时返回 [`HostError::Tray`]；旧 presenter
    /// 的移除失败按尽力而为忽略，新 presenter 仍会添加一次。presenter panic
    /// 时返回 [`HostError::TrayPresenter`]。
    pub fn set_tray_presenter(
        &mut self,
        presenter: impl TrayPresenter + 'static,
    ) -> Result<(), HostError> {
        let presenter: Box<dyn TrayPresenter> = Box::new(presenter);
        let mut previous = self.tray_presenter.replace(presenter);
        let added_hwnd = self
            .tray
            .as_mut()
            .and_then(|tray| std::mem::take(&mut tray.added).then_some(tray.window.hwnd));
        if let (Some(previous), Some(hwnd)) = (previous.as_mut(), added_hwnd) {
            // 旧 presenter 的移除是尽力而为；新 presenter 仍要添加一次。
            let _ = tray_presenter_call(|| previous.remove(hwnd));
        }
        self.sync_tray()
    }

    /// 创建托盘 HWND、共享图标与注册消息，并在可见宿主安装原生 presenter。
    fn install_tray(&mut self) -> Result<(), HostError> {
        let Some(spec) = self.runtime.tray() else {
            return Ok(());
        };
        // 安全：只注册系统广播消息，不解引用指针。
        let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
        if taskbar_created == 0 {
            return Err(HostError::Tray(String::from(
                "RegisterWindowMessageW(TaskbarCreated) failed",
            )));
        }
        let window = TrayMessageWindow::new(self.hinstance)?;
        // 安全：系统共享图标；presenter 与宿主都不销毁它。
        let icon = unsafe { LoadIconW(None, IDI_APPLICATION) }
            .map_err(|error| HostError::Tray(error.to_string()))?;
        self.tray = Some(TrayHost {
            window,
            spec,
            icon,
            added: false,
        });
        self.taskbar_created = taskbar_created;
        if self.show_windows {
            self.tray_presenter = Some(Box::new(NativeTrayPresenter));
        }
        self.present_tray_add()
    }

    /// 让当前 presenter 添加图标并更新 tooltip；没有 presenter 时什么也不做。
    fn present_tray_add(&mut self) -> Result<(), HostError> {
        let (Some(tray), Some(presenter)) = (self.tray.as_mut(), self.tray_presenter.as_mut())
        else {
            return Ok(());
        };
        tray_presenter_call(|| {
            presenter.add(TrayIconRequest::new(
                tray.window.hwnd,
                tray.icon,
                host_message::TRAY_CALLBACK,
            ))
        })?;
        tray.added = true;
        tray_presenter_call(|| {
            presenter.update(TrayTooltipRequest::new(
                tray.window.hwnd,
                tray.spec.tooltip(),
            ))
        })
    }

    /// 移除图标；移除成功后才清除 added，失败留给下一次同步或析构重试。
    fn remove_tray_icon(&mut self) {
        let Some(tray) = self.tray.as_ref() else {
            return;
        };
        if !tray.added {
            return;
        }
        let hwnd = tray.window.hwnd;
        let removed = self
            .tray_presenter
            .as_mut()
            .is_none_or(|presenter| tray_presenter_call(|| presenter.remove(hwnd)).is_ok());
        if removed && let Some(tray) = self.tray.as_mut() {
            tray.added = false;
        }
    }

    /// 会话结束时移除图标；常驻会话保留图标，并可在零窗口时恢复 primary。
    fn sync_tray(&mut self) -> Result<(), HostError> {
        if self.tray.is_none() {
            return Ok(());
        }
        if !self.session_running() {
            self.remove_tray_icon();
        } else if !self.tray.as_ref().is_some_and(|tray| tray.added) {
            self.present_tray_add()?;
        }
        Ok(())
    }

    /// Explorer 重建通知区域后重新添加图标；旧图标已随旧任务栏消失。
    fn re_add_tray(&mut self) -> Result<(), HostError> {
        let Some(tray) = self.tray.as_mut() else {
            return Ok(());
        };
        tray.added = false;
        self.present_tray_add()
    }

    fn is_tray_window(&self, hwnd: HWND) -> bool {
        self.tray
            .as_ref()
            .is_some_and(|tray| tray.window.hwnd == hwnd)
    }

    /// 托盘动作的窗口目标：可见焦点窗口优先，其次目录 primary，最后任一存活窗口。
    fn tray_target_window(&self) -> Option<WindowCtx> {
        let infos = self.runtime.windows();
        self.runtime
            .focused_window()
            .or_else(|| {
                let primary = self.runtime.primary_key()?;
                infos
                    .iter()
                    .find(|info| info.spec.key() == primary)
                    .map(|info| info.window)
            })
            .or_else(|| infos.first().map(|info| info.window))
    }

    /// 左键或键盘激活托盘：恢复焦点窗口；全部隐藏时恢复 primary。
    ///
    /// 只在 runtime 的窗口目录里选择目标，不重新打开已经关闭的窗口。原生显示
    /// 留给随后的同步阶段，避免在窗口过程里同步派发 `WM_SHOWWINDOW` 的重入；
    /// 已最小化的目标窗口是例外，恢复图标状态需要在同步前完成，且只在可见
    /// 宿主执行。
    fn restore_from_tray(&mut self) -> Result<(), HostError> {
        if !self.session_running() {
            return Ok(());
        }
        let key = self
            .tray_target_window()
            .map(|window| window.key())
            .or_else(|| {
                (self.runtime.session_policy() == SessionPolicy::StayResident)
                    .then(|| self.runtime.primary_key())
                    .flatten()
            });
        let Some(key) = key else {
            return Ok(());
        };
        self.runtime.show(key).map_err(HostError::Open)?;
        // runtime 的 show 对已打开窗口只更新焦点；用户最小化的原生窗口要先
        // 恢复图标状态，否则点击托盘只会让一个仍最小化的窗口“获得焦点”。
        if self.show_windows
            && let Some(window) = self.windows.get(key)
        {
            let hwnd = window.hwnd();
            self.without_routing(None, |_| unsafe {
                if IsIconic(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
            });
        }
        self.focus_after_sync = true;
        Ok(())
    }

    /// 从宿主取走一次右键菜单工作；菜单与 presenter 在展示期间离开宿主。
    fn begin_tray_menu(&mut self) -> Option<(Box<dyn TrayPresenter>, MenuSpec, Option<WindowCtx>)> {
        let menu = self.tray.as_ref()?.spec.menu().clone();
        let presenter = self.tray_presenter.take()?;
        Some((presenter, menu, self.tray_target_window()))
    }

    /// 菜单关闭后归还 presenter，并派发选中的托盘命令一次。
    ///
    /// presenter 只能返回托盘声明过的命令 id；未声明的 id 作为宿主错误上报。
    /// 菜单展示期间目标窗口关闭是正常竞态，不产生 stale 命令。
    fn finish_tray_menu(
        &mut self,
        presenter: Box<dyn TrayPresenter>,
        target: Option<WindowCtx>,
        result: Result<Option<&'static str>, HostError>,
    ) -> Result<(), HostError> {
        self.tray_presenter = Some(presenter);
        let Some(id) = result? else {
            return Ok(());
        };
        match self
            .runtime
            .dispatch_app_command(id, CommandSource::Tray, target)
        {
            Ok(()) | Err(CommandError::Stale) => Ok(()),
            Err(error) => Err(HostError::Command(error)),
        }
    }

    /// 显示所有打开的窗口（若配置为可见）并进入消息循环。
    ///
    /// 默认策略在最后一个窗口关闭后返回；`StayResident` 在零窗口时继续接收
    /// 托盘与 worker 消息，直到应用显式退出。没有声明托盘的常驻会话在
    /// [`Host::open`] 被拒绝。
    pub fn run(&mut self) -> Result<(), HostError> {
        self.reject_latched_loss()?;
        self.sync_windows()?;
        if !self.session_running() {
            return Ok(());
        }
        if !self.runtime.windows().is_empty() {
            self.render()?;
        }
        // 窗口过程要能在系统自己的拖动/缩放循环里重绘；指针只在循环期间安装。
        let host = self as *mut Self;
        let _pointer = HostPointer::install(host);
        // 安全：`host` 指向 `self`，地址在借用的 `run` 期间稳定；循环只通过
        // 裸指针访问宿主，派发消息时不持有引用。
        unsafe { run_loop(host) }
    }

    /// 处理所有已排队的窗口消息后立即返回；返回宿主会话是否已结束。
    ///
    /// 默认策略下会话在最后一个窗口关闭后结束；`StayResident` 的零窗口会话
    /// 返回 `false`，直到显式退出。把宿主嵌进别的消息循环，或在测试里驱动
    /// 一次输入而不阻塞时使用。
    pub fn pump(&mut self) -> Result<bool, HostError> {
        self.reject_latched_loss()?;
        self.sync_windows()?;
        let host = self as *mut Self;
        let _pointer = HostPointer::install(host);
        // 安全：与 `run` 相同。
        unsafe { pump_loop(host) }
    }

    /// 取走自上次调用以来的可恢复诊断；致命错误仍通过 [`Host::run`] /
    /// [`Host::pump`] 返回。
    ///
    /// 行工厂拒绝新的行窗口、行带失效等刷新失败记录在这里：宿主保留
    /// 已提交场景并继续运行（运行时已经把请求回滚到旧窗口）。最多保留
    /// 32 条，最旧的先丢弃。
    pub fn take_diagnostics(&mut self) -> Vec<String> {
        std::mem::take(&mut self.diagnostics)
    }

    /// 处理一条已经派发到窗口过程的消息。
    ///
    /// 输入处理放在窗口过程而不是消息循环里，因为系统的模态拖动/缩放循环会
    /// 直接调用窗口过程，消息循环在此期间收不到任何消息。注册表消息是宿主级
    /// 的：任一 HWND 收到唤醒都交付全部 worker 结果，再逐窗口比较快照重绘；
    /// 尚未实现的保留消息被显式忽略，不会落入窗口输入路由。
    fn handle_message(&mut self, hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) {
        if self.taskbar_created != 0 && message == self.taskbar_created {
            // Explorer 向所有顶层窗口广播；只有托盘 HWND 重新添加，避免每个
            // 窗口各添加一次。其它窗口直接认领，不落入普通输入路由。
            if self.is_tray_window(hwnd)
                && let Err(error) = self.re_add_tray()
            {
                self.deferred_error.get_or_insert(error);
            }
            return;
        }
        if message == host_message::UIA_ACTION {
            self.run_queued_accessibility_actions(hwnd);
            return;
        }
        if host_message::ALL.contains(&message) {
            if message == host_message::DROP {
                self.drain_drop_message(hwnd);
            } else if message == host_message::DEVICE_LOSS {
                self.handle_device_loss(wparam);
            } else if message == host_message::WAKE
                && let Err(error) = self.deliver_tasks()
            {
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
        if let Err(error) = self.settle_requests() {
            self.deferred_error.get_or_insert(error);
        }
        if let Err(error) = self.repaint_changed_windows() {
            self.deferred_error.get_or_insert(error);
        }
    }

    /// 处理设备丢失消息；代际不匹配或信号已被呈现清除时忽略该消息，
    /// 但仍在等待重建时补发一条已发布代际的恢复消息。
    fn handle_device_loss(&mut self, wparam: WPARAM) {
        self.loss.observed();
        let generation = wparam.0 as u64;
        if generation != self.device_generation {
            // 丢弃的 staging 设备或旧设备的迟到消息可能先占住合并标志，把失败
            // 尝试的重试挡掉；仍在等待重建时补发一条已发布代际的消息。
            if self.loss_phase == LossPhase::Pending && self.loss.is_lost() {
                self.loss.post_from_host(self.device_generation);
            }
            return;
        }
        if !self.loss.is_lost() {
            return;
        }
        self.loss_phase = LossPhase::Pending;
        // 可重试的失败已经排入恢复消息；只有锁存的终止错误进入消息循环。
        if let Err(error) = self.recover_device()
            && self.latched_loss.is_some()
        {
            self.deferred_error.get_or_insert(error);
        }
    }

    /// 结算运行时队列里的窗口与会话请求；有动作生效时重定向前台。
    ///
    /// 窗口请求先于会话动作结算；`StayResident` 的零窗口会话可以继续接收
    /// 托盘与 worker 消息。隐藏模式从不显示原生窗口，不抢前台；系统前台策略
    /// 拒绝时忽略错误，运行时焦点与输入路由不受影响。
    fn settle_requests(&mut self) -> Result<(), HostError> {
        let mut applied = 0;
        let mut first_error = None;
        match self.runtime.apply_window_requests() {
            Ok(count) => applied += count,
            Err(error) => {
                first_error.get_or_insert(HostError::WindowState(error));
            }
        }
        match self.runtime.apply_session_requests() {
            Ok(count) => applied += count,
            Err(error) => {
                first_error.get_or_insert(HostError::Session(error));
            }
        }
        if applied > 0 {
            // 启用/禁用与焦点恢复要等消息循环同步 HWND 之后再执行。
            self.focus_after_sync = true;
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// 宿主是否继续消息循环。
    ///
    /// 默认策略在最后一个窗口关闭后结束；`StayResident` 在零窗口时继续运行，
    /// 由显式退出或 [`Host::drop`] 结束。
    fn session_running(&self) -> bool {
        self.runtime.session_alive()
            && (self.runtime.session_policy() == SessionPolicy::StayResident
                || !self.runtime.windows().is_empty())
    }

    /// 第一次需要 OLE 时初始化 UI 线程的 STA；失败原因被缓存。
    ///
    /// 文件对话框与后续剪贴板/拖放共用这一个套间；注入 presenter 的隐藏
    /// 宿主不会走到这里。
    fn ensure_ole(&mut self) -> Result<(), OleError> {
        if self.ole.is_some() {
            return Ok(());
        }
        if let Some(error) = &self.ole_error {
            return Err(error.clone());
        }
        match OleApartment::initialize() {
            Ok(ole) => {
                self.ole = Some(ole);
                Ok(())
            }
            Err(error) => {
                self.ole_error = Some(error.clone());
                Err(error)
            }
        }
    }

    /// 校验锚点并取出 presenter；失败时不改变 Host 的 presenter 槽。
    fn file_dialog_work(
        &mut self,
        request: &FileDialogRequest,
    ) -> Result<FileDialogWork, FileDialogError> {
        let anchor = request.anchor();
        let Some(window) = self.windows.get(anchor.key()) else {
            return Err(FileDialogError::AnchorClosed);
        };
        if window.ctx != anchor || !self.runtime.window_accepts_input(anchor) {
            return Err(FileDialogError::AnchorClosed);
        }
        let hwnd = window.hwnd();
        let options = request.options().clone();
        let (presenter, restore) = match self.file_dialog_presenter.take() {
            Some(presenter) => (presenter, true),
            None if self.show_windows => {
                self.ensure_ole().map_err(file_dialog_ole_error)?;
                (
                    Box::new(NativeFileDialog) as Box<dyn FileDialogPresenter>,
                    false,
                )
            }
            None => return Err(FileDialogError::Unsupported),
        };
        self.file_dialog_active = true;
        Ok(FileDialogWork {
            presenter,
            restore,
            hwnd,
            options,
        })
    }

    /// 保存宿主结果并结算它引起的交付、窗口与重绘变化。
    fn finish_file_dialog(
        &mut self,
        request: FileDialogRequest,
        outcome: FileDialogOutcome,
        presenter: Option<Box<dyn FileDialogPresenter>>,
    ) -> Result<(), HostError> {
        if let Some(presenter) = presenter {
            self.file_dialog_presenter = Some(presenter);
        }
        self.file_dialog_active = false;
        let outcome = validated_file_dialog_outcome(request.options(), outcome);
        match self.runtime.complete_file_dialog_request(request, outcome) {
            Ok(()) | Err(FileDialogRequestError::Stale) => {}
            Err(error) => return Err(HostError::FileDialog(error)),
        }
        self.runtime.drive_tasks().map_err(HostError::Runtime)?;
        self.settle_requests()?;
        self.sync_windows()?;
        if std::mem::take(&mut self.focus_after_sync) {
            self.focus_native_focused_window()?;
        }
        self.repaint_changed_windows()?;
        if let Some(error) = self.deferred_error.take() {
            return Err(error);
        }
        Ok(())
    }

    /// 把运行时焦点窗口置前；只在应用已经持有前台时重定向，绝不从其它进程抢焦点。
    ///
    /// 隐藏模式或目标窗口已关闭时什么也不做。`SetForegroundWindow` 会同步派发
    /// 焦点消息；调用期间禁止路由，避免重入正被借出的宿主。被抑制的
    /// `WM_KILLFOCUS` 由这里显式结算，否则旧窗口的按下瞬态与 IME 组合会残留。
    fn focus_native_focused_window(&mut self) -> Result<(), HostError> {
        // 安全：只读取前台窗口句柄用于比较。
        let previous = unsafe { GetForegroundWindow() };
        if self.key_for(previous).is_none() {
            return Ok(());
        }
        let Some(key) = self.runtime.focused_window().map(|window| window.key()) else {
            return Ok(());
        };
        let Some(window) = self.windows.get(key) else {
            return Ok(());
        };
        if !window.native.shown || window.close_requested || !window.enabled {
            return Ok(());
        }
        let hwnd = window.hwnd();
        self.without_routing(None, |_| unsafe {
            let _ = SetForegroundWindow(hwnd);
        });
        if previous == hwnd {
            return Ok(());
        }
        let Some(previous_key) = self.key_for(previous) else {
            return Ok(());
        };
        let losing = self
            .windows
            .get(previous_key)
            .map(|window| (window.ctx, window.close_requested));
        if losing.is_none_or(|(ctx, closing)| closing || !self.runtime.window_visible(ctx)) {
            return Ok(());
        }
        self.with_window(previous_key, |host, window| host.focus_lost(window))
    }

    /// 一条指向某个窗口的消息；调用方已按 HWND 路由。
    fn handle_window_message(
        &mut self,
        window: &mut WindowHost,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Result<(), HostError> {
        if message == WM_SETTINGCHANGE {
            // 滚轮行数按 activation 缓存；系统广播设置变化时重新读取，
            // 值变化会丢弃未应用的 residue。
            window.wheel.set_lines(system_wheel_lines());
            // 高对比度与文本缩放也随广播变化；当前窗口已从 map 取出，单独标记。
            if self.refresh_system_preferences(self.current_system_preferences()) {
                window.paint_requested = true;
            }
            return Ok(());
        }
        if window.close_requested || !self.runtime.window_accepts_input(window.ctx) {
            // 关闭、隐藏或等待关闭确认的窗口不接受输入；迟到的唤醒由宿主的
            // 窗口级路径丢弃结果。
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
        if matches!(
            message,
            WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MOUSEMOVE | WM_MOUSEWHEEL | WM_KEYDOWN
        ) {
            // 输入必须命中当前滚动偏移的场景；surface 被遮挡时渲染可能没有
            // 更新 `window.scene`，这里先补一次只重建、不呈现的同步。
            self.refresh_input_scene(window)?;
        }
        if message == WM_LBUTTONDOWN {
            self.pointer_pressed(window, lparam)
        } else if message == WM_LBUTTONUP {
            self.pointer_released(window, lparam)
        } else if message == WM_MOUSEMOVE {
            self.pointer_moved(window, lparam)
        } else if message == WM_MOUSEWHEEL {
            self.wheel_scrolled(window, wparam)
        } else if message == WM_MOUSEHWHEEL {
            Ok(())
        } else if message == WM_MOUSELEAVE {
            self.pointer_left(window)
        } else if message == WM_CAPTURECHANGED {
            self.capture_lost(window)
        } else if message == WM_CHAR {
            self.char_typed(window, wparam, lparam)
        } else if message == WM_KEYDOWN {
            self.key_down(window, wparam, lparam)
        } else if message == WM_IME_STARTCOMPOSITION {
            self.ime_start(window)
        } else if message == WM_IME_COMPOSITION {
            self.ime_composition(window, lparam)
        } else if message == WM_IME_ENDCOMPOSITION {
            window.pressed = None;
            self.ime_cancel(window)
        } else if message == WM_KILLFOCUS {
            self.focus_lost(window)
        } else if message == WM_INITMENU || message == WM_INITMENUPOPUP {
            // 上下文菜单弹窗拥有自己的 HMENU 与命令编号；窗口菜单栏的状态回调
            // 不能按数字 id 命中它，否则两个菜单的同号命令会互相改写状态。
            if self.context_menu_active {
                return Ok(());
            }
            if message == WM_INITMENU {
                self.apply_menu_state(window, HMENU(wparam.0 as *mut c_void), false)
            } else {
                let system_menu = (lparam.0 >> 16) & 0xFFFF != 0;
                self.apply_menu_state(window, HMENU(wparam.0 as *mut c_void), system_menu)
            }
        } else if message == WM_COMMAND {
            self.menu_command(window, wparam, lparam)
        } else if message == WM_ENTERMENULOOP {
            self.menu_loop_entered(window)
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
            host.render_current(window)
        });
        if let Err(error) = result {
            self.deferred_error.get_or_insert(error);
        }
    }

    /// `WM_CLOSE` 的宿主处理：走目录关闭策略。
    ///
    /// Immediate 结束 activation，消息循环随后销毁 HWND；Confirm 建立 epoch
    /// 并显示确认窗口，父窗口保持存活但在下一轮同步中禁用。确认窗口自己的
    /// X 由运行时解释为 Deny。
    fn close_from_window(&mut self, hwnd: HWND, source: CloseSource) {
        let Some(key) = self.key_for(hwnd) else {
            return;
        };
        let Some(ctx) = self.windows.get(key).map(|window| window.ctx) else {
            return;
        };
        // 确认窗口自己的关闭/回复使用独立来源；父窗口 handler 的来源保持不变。
        let source = if self
            .runtime
            .confirmation_request(ctx)
            .is_ok_and(|request| request.is_some())
        {
            CloseSource::ConfirmationWindow
        } else {
            source
        };
        match self.runtime.request_close_from(ctx, source) {
            Ok(()) => {
                if !self.runtime.window_alive(ctx)
                    && let Some(window) = self.windows.get_mut(key)
                {
                    window.close_requested = true;
                    window.clear_pointer_transients();
                }
                self.focus_after_sync = true;
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
        // 交付期的 continuation 也可以打开、隐藏或关闭窗口。
        self.settle_requests()?;
        if report.delivered == 0 {
            return Ok(());
        }
        self.repaint_changed_windows()
    }

    /// 闪烁 caret；只有聚焦输入、非组合且窗口在前台时才翻转。
    fn blink_caret(&mut self, window: &mut WindowHost) -> Result<(), HostError> {
        let (focused, composing) = self.with_presentation(window, |snapshot| {
            (
                focused_input(snapshot).is_some(),
                focused_composition(snapshot),
            )
        })?;
        // 隐藏窗口无法成为前台窗口；测试可以注入前台状态覆盖真实查询。
        #[cfg(test)]
        let foreground = window
            .foreground
            .unwrap_or_else(|| unsafe { GetForegroundWindow() } == window.hwnd());
        // 安全：只读取前台窗口句柄用于比较。
        #[cfg(not(test))]
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
        // 菜单栏属于非客户区；与 WM_GETDPISCALEDSIZE 使用同一个原生查询。
        // 安全：hwnd 属于本窗口条目。
        let has_menu = !unsafe { GetMenu(hwnd) }.is_invalid();
        // 安全：rect 由本地值初始化；按窗口 DPI 计算含菜单栏的非客户区。
        unsafe {
            AdjustWindowRectExForDpi(
                &mut rect,
                WS_OVERLAPPEDWINDOW,
                has_menu,
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
        self.reject_latched_loss()?;
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

    /// 重绘当前窗口：运行时呈现未变时复用缓存快照，否则重建快照。
    fn render_current(&mut self, window: &mut WindowHost) -> Result<(), HostError> {
        let revision = self
            .runtime
            .presentation_revision_of(window.ctx)
            .unwrap_or(window.snapshot_revision);
        if window.last_snapshot.is_some() && window.snapshot_revision == revision {
            return self.render_cached(window);
        }
        self.render_fresh(window)
    }

    /// 读取当前呈现快照：版本未变时借用缓存（取出再放回），否则重建一次。
    fn with_presentation<T>(
        &mut self,
        window: &mut WindowHost,
        read: impl FnOnce(&PresentationSnapshot) -> T,
    ) -> Result<T, HostError> {
        let revision = self
            .runtime
            .presentation_revision_of(window.ctx)
            .unwrap_or(window.snapshot_revision);
        if window.last_snapshot.is_some() && window.snapshot_revision == revision {
            let Some(snapshot) = window.last_snapshot.take() else {
                unreachable!("the cache check guarantees a snapshot");
            };
            let value = read(&snapshot);
            window.last_snapshot = Some(snapshot);
            return Ok(value);
        }
        let snapshot = self
            .runtime
            .presentation_of(window.ctx)
            .map_err(HostError::Snapshot)?;
        Ok(read(&snapshot))
    }

    /// 输入前确保内存场景使用当前滚动偏移与显露结果。
    ///
    /// `render_window` 在 surface `Timeout`/`Occluded` 时不会更新 `window.scene`；
    /// 指针与键盘输入必须命中最新偏移的场景，因此这里只重建，不呈现。
    ///
    /// 这是纯构建：不运行行 factory，也不调用 `set_row_windows`。第一遍
    /// 场景连同行窗口目标一起返回，由 [`Host::refresh_input_scene`] 在场景
    /// 构建的调用栈之外完成结构 turn（ADR：结构变化不能藏在构建里）。
    fn ensure_scene(&mut self, window: &mut WindowHost) -> Result<Option<PendingScene>, HostError> {
        let fresh = window.scene.as_ref().is_some_and(|scene| {
            scene.scroll_offset() == window.scroll_offset && !window.reveal_requested
        });
        if fresh {
            return Ok(None);
        }
        // 运行时版本已经前进时缓存快照不再可信；行窗口更新也会改变版本。
        let revision = self
            .runtime
            .presentation_revision_of(window.ctx)
            .unwrap_or(window.snapshot_revision);
        if window.snapshot_revision != revision || window.last_snapshot.is_none() {
            match self.runtime.presentation_of(window.ctx) {
                Ok(snapshot) => {
                    window.last_snapshot = Some(snapshot);
                    window.snapshot_revision = revision;
                }
                Err(WindowError::Hidden) => return Ok(None),
                Err(error) => return Err(HostError::Snapshot(error)),
            }
        }
        // 快照先取出再放回：输入前同步场景不复制 snapshot。
        let Some(snapshot) = window.last_snapshot.take() else {
            return Ok(None);
        };
        let result = (|| {
            let (width, height) = client_size(window.hwnd())?;
            if width == 0 || height == 0 {
                window.accessibility.clear();
                return Ok(None);
            }
            let scene = self.build_scene_with_reveal(window, &snapshot, width, height, false)?;
            let row_windows = pending_row_windows(&scene);
            Ok(Some(PendingScene { scene, row_windows }))
        })();
        window.last_snapshot = Some(snapshot);
        window.snapshot_revision = self
            .runtime
            .presentation_revision_of(window.ctx)
            .unwrap_or(window.snapshot_revision);
        result
    }

    /// 输入前把场景推进到最终状态：纯构建之后，在构建调用栈之外应用行窗口。
    fn refresh_input_scene(&mut self, window: &mut WindowHost) -> Result<(), HostError> {
        let Some(pending) = self.ensure_scene(window)? else {
            return Ok(());
        };
        self.settle_input_scene(window, pending)
    }

    /// 结算 [`Host::ensure_scene`] 返回的第一遍场景：应用行窗口并提交最终场景。
    fn settle_input_scene(
        &mut self,
        window: &mut WindowHost,
        pending: PendingScene,
    ) -> Result<(), HostError> {
        let Some(mut snapshot) = window.last_snapshot.take() else {
            return Ok(());
        };
        let result = self.apply_row_windows(window, &mut snapshot, pending);
        match result {
            Ok(scene) => {
                window.snapshot_revision = self
                    .runtime
                    .presentation_revision_of(window.ctx)
                    .unwrap_or(window.snapshot_revision);
                commit_scene(window, &snapshot, scene);
                window.last_snapshot = Some(snapshot);
                Ok(())
            }
            Err(error) => {
                // 失败的批量 turn 可能已经推进过 revision（提交后再回滚）：
                // 不把缓存标记成最新，下一次绘制会重新读取快照。
                window.last_snapshot = Some(snapshot);
                Err(error)
            }
        }
    }

    /// 在第一遍场景之后应用批量行窗口，返回最终要呈现的场景。
    ///
    /// 行带目标与运行时当前请求相同（或没有窗口化 `For`）时直接复用第一遍
    /// 场景，不刷新、不建第二遍。任一 band 变化时用运行时的新快照重建第二遍；
    /// 批次被拒绝时保留第一遍场景并把错误报告给调用方。
    ///
    /// 聚焦行超过一行 grace 要离开窗口时，先用仍存活的旧结构完成 blur 并
    /// 取消组合（ADR-0011），已卸载节点不再派发事件。blur 处理器是用户代码，
    /// 可能已经改写树或行带几何；第一遍批次因此失效时用新快照重建并只重试
    /// 一次，而不是把一次正常滚动变成宿主错误。
    fn apply_row_windows(
        &mut self,
        window: &mut WindowHost,
        snapshot: &mut PresentationSnapshot,
        pending: PendingScene,
    ) -> Result<Scene, HostError> {
        let PendingScene {
            mut scene,
            row_windows,
        } = pending;
        if row_windows.is_empty() {
            return Ok(scene);
        }
        let focus_leaves = {
            let offset = scene.scroll_offset() as f32;
            snapshot.focus().is_some_and(|focused| {
                let Some(element) = scene.node(focused) else {
                    return false;
                };
                let content_y = element.rect().y + offset;
                row_windows.bands.iter().any(|(key, request)| {
                    let Some(band) = scene.row_bands().iter().find(|band| band.key() == *key)
                    else {
                        return false;
                    };
                    row_containing(*band, content_y).is_some_and(|row| {
                        row + 1 < request.first() || row > request.first() + request.count()
                    })
                })
            })
        };
        let mut blurred = false;
        if focus_leaves {
            if let Err(error) = self.runtime.blur_in(window.ctx) {
                // 窗口在消息派发中被关闭：没有可 blur 的树，条目马上移除。
                if matches!(error, WindowError::Stale) && !self.runtime.window_alive(window.ctx) {
                    return Ok(scene);
                }
                return Err(HostError::WindowState(error));
            }
            self.cancel_compositions(window)?;
            blurred = true;
        }
        let mut refreshed = false;
        let report = match self.runtime.set_row_windows(window.ctx, &row_windows.bands) {
            Ok(report) => report,
            // activation 已经结束：`settle_requests` 关闭窗口树之后，原生
            // 条目要到下一次 `sync_windows` 才移除。这一帧保留场景，不再
            // 为死窗口跑结构 turn，直到宿主销毁 HWND。
            Err(WindowError::Stale) if !self.runtime.window_alive(window.ctx) => return Ok(scene),
            // blur 改变了树：第一遍场景与批次都可能失效，焦点已经清除，
            // 不再 blur；用新快照重建第一遍并重试一次。
            Err(WindowError::Stale) if blurred => {
                *snapshot = self
                    .runtime
                    .presentation_of(window.ctx)
                    .map_err(HostError::Snapshot)?;
                let (width, height) = client_size(window.hwnd())?;
                if width == 0 || height == 0 {
                    return Ok(scene);
                }
                scene = self.build_scene_with_reveal(window, snapshot, width, height, false)?;
                refreshed = true;
                let retry = pending_row_windows(&scene);
                if retry.is_empty() {
                    return Ok(scene);
                }
                match self.runtime.set_row_windows(window.ctx, &retry.bands) {
                    Ok(report) => report,
                    Err(error @ (WindowError::WrongRuntime | WindowError::DuplicateRowBand)) => {
                        return Err(HostError::WindowState(error));
                    }
                    // 运行时已经回滚请求；宿主保留旧窗口并继续运行。
                    Err(error) => {
                        return self.recover_row_window_failure(window, snapshot, scene, error);
                    }
                }
            }
            // 运行时已经回滚请求；行工厂拒绝新窗口不是致命会话错误。
            Err(error @ (WindowError::WrongRuntime | WindowError::DuplicateRowBand)) => {
                return Err(HostError::WindowState(error));
            }
            Err(error) => {
                return self.recover_row_window_failure(window, snapshot, scene, error);
            }
        };
        if !report.changed() && (!blurred || refreshed) {
            // 未变化的批次复用第一遍；重试路径的第一遍已经来自 blur 后的
            // 新快照，不需要再为焦点清除建一遍。
            return Ok(scene);
        }
        // 结构 turn 已经提交新窗口；快照与第二遍场景都来自新版本。
        *snapshot = self
            .runtime
            .presentation_of(window.ctx)
            .map_err(HostError::Snapshot)?;
        let (width, height) = client_size(window.hwnd())?;
        if width == 0 || height == 0 {
            // 零尺寸没有可呈现的几何；保留第一遍场景直到尺寸恢复。
            return Ok(scene);
        }
        self.build_scene_with_reveal(window, snapshot, width, height, false)
    }

    /// 行窗口批次可恢复地失败：保留已提交场景并记录诊断。
    ///
    /// 运行时已经把请求回滚到旧窗口；宿主不把用户 factory 的失败升级成
    /// 会话错误。blur 或处理器写入可能已经改变树，第一遍场景用的快照过期
    /// 时用新快照重建第一遍，保证场景、快照与可访问性树来自同一版本。
    fn recover_row_window_failure(
        &mut self,
        window: &mut WindowHost,
        snapshot: &mut PresentationSnapshot,
        scene: Scene,
        error: WindowError,
    ) -> Result<Scene, HostError> {
        self.note_diagnostic(format!(
            "row window sync failed for {}: {error}",
            window.ctx.key()
        ));
        let fresh = self
            .runtime
            .presentation_of(window.ctx)
            .map_err(HostError::Snapshot)?;
        if fresh == *snapshot {
            return Ok(scene);
        }
        *snapshot = fresh;
        let (width, height) = client_size(window.hwnd())?;
        if width == 0 || height == 0 {
            return Ok(scene);
        }
        self.build_scene_with_reveal(window, snapshot, width, height, false)
    }

    fn note_diagnostic(&mut self, message: String) {
        if self.diagnostics.len() >= DIAGNOSTIC_LIMIT {
            self.diagnostics.remove(0);
        }
        self.diagnostics.push(message);
    }

    /// 把一个窗口的新快照渲染到它自己的 surface；共享 renderer 按目标尺寸切换。
    fn render_window(
        &mut self,
        window: &mut WindowHost,
        mut snapshot: PresentationSnapshot,
    ) -> Result<(), HostError> {
        let snapshot_changed = window.last_snapshot.as_ref() != Some(&snapshot);
        if self.render_snapshot(window, &mut snapshot, snapshot_changed)? {
            window.last_snapshot = Some(snapshot);
            window.snapshot_revision = self
                .runtime
                .presentation_revision_of(window.ctx)
                .unwrap_or(window.snapshot_revision);
        }
        Ok(())
    }

    /// 用缓存的快照重绘；滚动、悬停与 pressed 都是宿主瞬态，不需要重建运行时快照。
    ///
    /// 快照先取出再放回，整个路径不复制 snapshot。窗口化 `For` 在此发现需要
    /// 更换挂载窗口时，会用运行时的新快照替换缓存并更新版本。
    fn render_cached(&mut self, window: &mut WindowHost) -> Result<(), HostError> {
        let Some(mut snapshot) = window.last_snapshot.take() else {
            return Ok(());
        };
        let result = self.render_snapshot(window, &mut snapshot, false);
        window.last_snapshot = Some(snapshot);
        if result? {
            window.snapshot_revision = self
                .runtime
                .presentation_revision_of(window.ctx)
                .unwrap_or(window.snapshot_revision);
        }
        Ok(())
    }

    /// 构建场景并呈现；返回本次是否成功呈现。
    ///
    /// 只有成功呈现才提交场景并返回 `Ok(true)`；`Timeout` / `Occluded` /
    /// `Lost` 与零尺寸保留 `paint_requested` 并返回 `Ok(false)`，下一次
    /// 绘制重试同一份 damage，屏幕上继续显示上一帧。
    ///
    /// 窗口化 `For` 的第一遍场景提供行带几何；需要更换挂载窗口时，宿主先按
    /// 焦点策略结束焦点，再应用窗口并用新快照重建一次。一帧最多两遍场景。
    fn render_snapshot(
        &mut self,
        window: &mut WindowHost,
        snapshot: &mut PresentationSnapshot,
        snapshot_changed: bool,
    ) -> Result<bool, HostError> {
        self.reject_latched_loss()?;
        // 丢失回调先置位信号、再投递消息；消息处理前阶段仍是 `Clear`，
        // `AwaitingPresent` 也可能因重建设备再次丢失而落后于丢失序号。
        // 两种情况下这一帧都不能创建 surface，也不能呈现。
        let stale_proving =
            self.loss_phase == LossPhase::AwaitingPresent && self.loss.epoch() != self.loss_epoch;
        if (self.loss_phase == LossPhase::Clear && self.loss.is_lost()) || stale_proving {
            self.loss_phase = LossPhase::Pending;
            return Ok(false);
        }
        if self.loss_phase == LossPhase::Pending && !self.recovering {
            // 上一次尝试已失败并排入重试，surface 为空；这一帧留给重试之后。
            window.paint_requested = true;
            return Ok(false);
        }
        self.ensure_surface(window)?;
        let (width, height) = client_size(window.hwnd())?;
        if width == 0 || height == 0 {
            // 零尺寸没有可呈现的帧；保留 damage，尺寸恢复后再画。
            window.paint_requested = true;
            window.accessibility.clear();
            return Ok(false);
        }
        let scene =
            self.build_scene_with_reveal(window, snapshot, width, height, snapshot_changed)?;
        // 行带几何来自第一遍；行窗口结构 turn 在场景构建之外应用，只有真的
        // 变化时才用运行时的新快照建第二遍。行带顶端锚定在内容起点，两遍
        // 场景的偏移一致且精确。
        let row_windows = pending_row_windows(&scene);
        let scene =
            self.apply_row_windows(window, snapshot, PendingScene { scene, row_windows })?;
        let target = target_config(self.format, width, height)?;
        if self.renderer_target != target {
            self.renderer.resize(target);
            self.renderer_target = target;
        }
        self.renderer.set_palette(palette_for(
            snapshot.appearance(),
            self.system_preferences,
            high_contrast_colors,
        ));
        let mut reconfigured = false;
        loop {
            let status = self.next_surface_status(window)?;
            let frame = match status {
                wgpu::CurrentSurfaceTexture::Success(frame)
                | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    // 没有拿到可呈现的帧：保留 damage，下一次绘制重试。
                    window.paint_requested = true;
                    return Ok(false);
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
                wgpu::CurrentSurfaceTexture::Lost => {
                    window.paint_requested = true;
                    self.surface_lost()?;
                    return Ok(false);
                }
                wgpu::CurrentSurfaceTexture::Validation => {
                    window.paint_requested = true;
                    self.validation_status()?;
                    return Ok(false);
                }
            };
            let view = frame.texture.create_view(&Default::default());
            let mut encoder = self.device.create_command_encoder(&Default::default());
            if let Err(error) = self.renderer.render(&scene, &view, &mut encoder) {
                if reconfigured {
                    return Err(HostError::Render(error));
                }
                self.surface_lost()?;
                return Ok(false);
            }
            self.queue.submit([encoder.finish()]);
            self.queue.present(frame);
            #[cfg(test)]
            {
                self.counters.presents += 1;
            }
            self.presented_since_rebuild = true;
            if self.loss_phase == LossPhase::AwaitingPresent && self.loss.clear_if(self.loss_epoch)
            {
                self.loss_phase = LossPhase::Clear;
                self.loss_epoch = 0;
                // 第一次成功呈现补上了缺失的证明性呈现，本轮尝试到此结束。
                self.recovery_attempts = 0;
                self.attempt_adapter = None;
            }
            commit_scene(window, snapshot, scene);
            window.paint_requested = false;
            self.position_ime(window, snapshot);
            return Ok(true);
        }
    }

    /// 取下一帧的 surface 状态；测试可注入一次状态而不经过真实交换链。
    fn next_surface_status(
        &mut self,
        window: &WindowHost,
    ) -> Result<wgpu::CurrentSurfaceTexture, HostError> {
        #[cfg(test)]
        if let Some(status) = self.forced_surface_statuses.pop_front() {
            return Ok(status);
        }
        Ok(self.current_surface(window)?.get_current_texture())
    }

    /// 信号已置位时，`Validation` 是设备移除的级联错误；否则是 API misuse。
    fn validation_status(&mut self) -> Result<(), HostError> {
        if self.loss.is_lost() {
            self.surface_lost()?;
            return Ok(());
        }
        Err(HostError::Surface(String::from(
            "the surface failed validation; rebuilding would mask an API misuse",
        )))
    }

    /// 标记设备丢失；消息处理结束后由 [`Host::recover_device`] 统一重建。
    fn surface_lost(&mut self) -> Result<(), HostError> {
        if self.recovering {
            return Err(HostError::Surface(String::from(
                "the surface stayed lost after rebuilding",
            )));
        }
        self.loss.mark_lost();
        self.loss_phase = LossPhase::Pending;
        Ok(())
    }

    /// 在尝试上限内重建 device/queue/renderer 与全部 surface，然后重绘。
    ///
    /// 丢失信号跨重建保留；只有新设备成功呈现至少一帧且恢复期间没有新的
    /// 丢失/渲染失败时才清除。没有可见窗口时停在 `AwaitingPresent`，不会
    /// 因为重复消息反复创建设备。失败的尝试排入一条恢复消息并回到消息循环；
    /// 达到上限后锁存一个 [`HostError::DeviceLost`]，之后的调用不再创建设备。
    fn recover_device(&mut self) -> Result<(), HostError> {
        if self.recovering {
            return Ok(());
        }
        if let Some(error) = &self.latched_loss {
            return Err(HostError::DeviceLost(Box::new(error.clone())));
        }
        if self.loss_phase != LossPhase::Pending {
            return Ok(());
        }
        if self.recovery_attempts >= MAX_RECOVERY_ATTEMPTS {
            // 重建成功的设备在证明性呈现之前又丢了：本轮尝试已经用尽。
            return Err(self.latch_device_lost(String::from(
                "the device was lost again before a proving present",
            )));
        }
        self.recovery_attempts += 1;
        self.attempt_adapter = None;
        let epoch = self.loss.epoch();
        self.recovering = true;
        self.presented_since_rebuild = false;
        let result = self.rebuild_presentations().and_then(|()| self.render());
        self.recovering = false;
        match result {
            Ok(()) => {
                if self.loss.epoch() != epoch {
                    self.loss_phase = LossPhase::Pending;
                } else if self.presented_since_rebuild {
                    if self.loss.clear_if(epoch) {
                        self.loss_phase = LossPhase::Clear;
                        self.loss_epoch = 0;
                        self.recovery_attempts = 0;
                        self.attempt_adapter = None;
                    } else {
                        self.loss_phase = LossPhase::AwaitingPresent;
                        self.loss_epoch = epoch;
                    }
                } else {
                    self.loss_phase = LossPhase::AwaitingPresent;
                    self.loss_epoch = epoch;
                }
                Ok(())
            }
            Err(error) => {
                let message = error.to_string();
                if self.recovery_attempts >= MAX_RECOVERY_ATTEMPTS {
                    Err(self.latch_device_lost(message))
                } else {
                    // 信号保持置位；下一条消息在上限内重试，不阻塞 UI 线程。
                    self.loss.post_from_host(self.device_generation);
                    Err(error)
                }
            }
        }
    }

    /// 本轮丢失事件达到上限后的终止错误；之后的调用直接返回它。
    fn reject_latched_loss(&self) -> Result<(), HostError> {
        match &self.latched_loss {
            Some(error) => Err(HostError::DeviceLost(Box::new(error.clone()))),
            None => Ok(()),
        }
    }

    /// 锁存终止错误；之后不再创建设备，也不清除丢失信号。
    fn latch_device_lost(&mut self, message: String) -> HostError {
        let error = DeviceLostError {
            attempts: self.recovery_attempts,
            adapter: self
                .attempt_adapter
                .clone()
                .unwrap_or_else(|| self.adapter_info()),
            message,
        };
        self.latched_loss = Some(error.clone());
        HostError::DeviceLost(Box::new(error))
    }

    /// 当前 device 代是否可以创建 surface。
    ///
    /// 丢失回调先置位信号、再投递消息；消息处理前阶段仍是 `Clear`，但那时
    /// 的 device 已经失效。`AwaitingPresent` 只欠一次呈现，新窗口可以在已
    /// 重建的 device 上建 surface；丢失序号前进后（重建后的设备又丢了）
    /// 才再次禁止。
    fn can_create_surface(&self) -> bool {
        if self.latched_loss.is_some() {
            return false;
        }
        match self.loss_phase {
            LossPhase::Clear => !self.loss.is_lost(),
            LossPhase::Pending => false,
            LossPhase::AwaitingPresent => self.loss.epoch() == self.loss_epoch,
        }
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
        // 一次指针按下只属于一个窗口；其它窗口的按压瞬态立即结束。
        self.clear_pressed_except(window.ctx.key());
        window.char_suppression = None;
        let scale = window
            .scene
            .as_ref()
            .map_or(1.0, |scene| scene.viewport().scale());
        let point = pointer_point(lparam, scale);
        window.pointer = Some(point);
        window.paint_requested = true;

        // 覆盖层滚动条先于内容获得命中：拖动 thumb 或点击轨道只滚动，
        // 不移动焦点、不取消 IME 组合（ADR-0010：滚动永不提交或取消组合）。
        if let Some(bar) = window.scene.as_ref().and_then(Scene::scrollbar)
            && bar.hit(point)
        {
            window.hovered = None;
            window.pressed = None;
            return self.scrollbar_pressed(window, bar, point);
        }

        // 指针操作会移动焦点或 caret；先结束任何组合，避免 preedit 跟随到别处。
        self.cancel_compositions(window)?;
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

        // 按下与 headless 共用 Press Intent：最近可聚焦祖先获得焦点，text-input
        // 使用场景算出的字符偏移放置 caret；禁用目标不会走到这里。
        ignore_expected_dispatch(self.runtime.dispatch_intent(Intent::press(node, caret)))?;

        match visual {
            Visual::TextInput => {
                // 可见切片内的点击通常不需要移动；caret 落在视口外时由同一
                // 显露规则拉回。
                window.reveal_requested = true;
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
            }
        }
        Ok(())
    }

    /// 滚动条按下：thumb 开始拖动并捕获指针，轨道点击向指针一侧翻一页。
    fn scrollbar_pressed(
        &mut self,
        window: &mut WindowHost,
        bar: Scrollbar,
        point: Point,
    ) -> Result<(), HostError> {
        // 上一次拖动只会在捕获丢失后留下；按下总是重新开始一次手势。
        window.scrollbar_drag = None;
        let thumb = bar.thumb();
        if thumb.contains(point) {
            // 捕获指针：拖动离开轨道后松开仍回到本窗口。
            // 安全：捕获转移会同步向旧捕获窗口送出 WM_CAPTURECHANGED；
            // 在禁止重入的窗口里调用 SetCapture。
            self.without_routing(Some(window.hwnd()), |_| unsafe {
                SetCapture(window.hwnd());
            });
            window.scrollbar_drag = Some(point.y - thumb.y);
            return Ok(());
        }
        let Some(scene) = window.scene.as_ref() else {
            return Ok(());
        };
        let max = scene.max_scroll_offset();
        if max == 0 {
            return Ok(());
        }
        let step = page_step(Some(scene));
        let target = if point.y < thumb.y {
            window.scroll_offset.saturating_sub(step)
        } else {
            window.scroll_offset.saturating_add(step)
        }
        .clamp(0, max);
        self.explicit_scroll(window, target);
        Ok(())
    }

    /// thumb 拖动：按指针位置换算夹取后的偏移，不重新评估元素按压。
    fn drag_scrollbar(&mut self, window: &mut WindowHost, point: Point) -> Result<(), HostError> {
        let Some(grab) = window.scrollbar_drag else {
            return Ok(());
        };
        let Some(scene) = window.scene.as_ref() else {
            return Ok(());
        };
        let Some(bar) = scene.scrollbar() else {
            self.end_scrollbar_drag(window);
            return Ok(());
        };
        let max = scene.max_scroll_offset();
        if max == 0 {
            self.end_scrollbar_drag(window);
            return Ok(());
        }
        let Some(offset) = scrollbar_drag_offset(bar.track(), bar.thumb(), grab, point.y, max)
        else {
            return Ok(());
        };
        window.hovered = None;
        if window.scroll_offset != offset {
            self.explicit_scroll(window, offset);
        }
        Ok(())
    }

    /// 结束 thumb 拖动并安排释放捕获；没有拖动时是空操作。
    fn end_scrollbar_drag(&mut self, window: &mut WindowHost) -> bool {
        if window.scrollbar_drag.take().is_none() {
            return false;
        }
        window.release_capture = true;
        window.paint_requested = true;
        true
    }

    /// 显式滚动落点：丢弃待决显露与滚轮 residue，保留 capture-based 按压目标。
    fn explicit_scroll(&mut self, window: &mut WindowHost, offset: i32) {
        window.reveal_requested = false;
        window.wheel.reset();
        window.scroll_offset = offset;
        window.preserve_press_on_next_rebuild = true;
        window.paint_requested = true;
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
        if window.scrollbar_drag.is_some() {
            return self.drag_scrollbar(window, point);
        }

        let hovered = window
            .scene
            .as_ref()
            .and_then(|scene| hover_at(scene, point));
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

    /// 宿主当前是否接受这个窗口的指针、键盘或 UIA 输入。
    fn window_accepts_host_input(&self, window: &WindowHost) -> bool {
        !window.close_requested && window.enabled && self.runtime.window_accepts_input(window.ctx)
    }

    /// 一次滚轮事件：按 ADR-0010 累积、取整、夹取；正向 notch 减小 offset。
    fn wheel_scrolled(&mut self, window: &mut WindowHost, wparam: WPARAM) -> Result<(), HostError> {
        if !self.window_accepts_host_input(window) {
            return Ok(());
        }
        let delta = ((wparam.0 >> 16) as u16) as i16;
        if delta == 0 {
            return Ok(());
        }
        let page = page_step(window.scene.as_ref());
        let step = window.wheel.step(i32::from(delta), page);
        if step == 0 {
            return Ok(());
        }
        // 显式滚动优先：丢弃尚未应用的显露请求。
        window.reveal_requested = false;
        window.scroll_offset = window.scroll_offset.saturating_sub(step);
        window.preserve_press_on_next_rebuild = true;
        window.paint_requested = true;
        Ok(())
    }

    /// PageUp/PageDown/Home/End 的滚动步；键盘滚动重置滚轮 residue。
    fn scroll_by_key(
        &mut self,
        window: &mut WindowHost,
        key: VIRTUAL_KEY,
    ) -> Result<(), HostError> {
        self.refresh_input_scene(window)?;
        let Some(scene) = window.scene.as_ref() else {
            return Ok(());
        };
        let max = scene.max_scroll_offset();
        if max == 0 {
            // ADR：最大偏移为零时不消费滚动键；这里不改变任何状态。
            return Ok(());
        }
        // 显式滚动优先：丢弃尚未应用的显露请求。
        let step = page_step(Some(scene));
        let target = match key {
            VK_PRIOR => window.scroll_offset.saturating_sub(step),
            VK_NEXT => window.scroll_offset.saturating_add(step),
            VK_HOME => 0,
            VK_END => max,
            _ => return Ok(()),
        };
        self.explicit_scroll(window, target);
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
        if window.scrollbar_drag.take().is_some() {
            // 松开只结束拖动：不激活指针下的元素，也不改焦点。
            window.release_capture = true;
            window.hovered = window
                .scene
                .as_ref()
                .and_then(|scene| hover_at(scene, point));
            window.paint_requested = true;
            return Ok(());
        }
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
                Some(Visual::Button { .. }) => {
                    ignore_expected_dispatch(self.runtime.activate_node(node))?
                }
                Some(Visual::Checkbox { checked }) => {
                    ignore_expected_dispatch(self.runtime.change_node(node, !checked))?;
                }
                _ => {}
            }
        }

        window.hovered = window
            .scene
            .as_ref()
            .and_then(|scene| hover_at(scene, point));
        Ok(())
    }

    /// 捕获丢失（例如系统抢走指针）：按住状态与 thumb 拖动都不能残留。
    fn capture_lost(&mut self, window: &mut WindowHost) -> Result<(), HostError> {
        let drag = window.scrollbar_drag.take().is_some();
        if window.pressed.take().is_some() || drag {
            window.paint_requested = true;
        }
        Ok(())
    }

    /// 焦点丢失：清除按住瞬态、thumb 拖动与组合，并让下一帧不再绘制按下状态。
    fn focus_lost(&mut self, window: &mut WindowHost) -> Result<(), HostError> {
        window.char_suppression = None;
        let was_dragging = window.scrollbar_drag.take().is_some();
        let was_pressed = window.pressed.take().is_some();
        if was_pressed || was_dragging {
            window.release_capture = true;
        }
        let cancelled = self.cancel_compositions(window)?;
        if cancelled || was_pressed || was_dragging {
            window.paint_requested = true;
        }
        Ok(())
    }

    /// 目标消息窗口的快捷键表，以及本次按键是否必须让给编辑器或 IME。
    fn accelerator_for(&self, message: &MSG) -> Option<(HACCEL, bool)> {
        let window = self
            .windows
            .values()
            .find(|window| window.hwnd() == message.hwnd)?;
        let accelerator = window.menu.as_ref()?.accelerator()?;
        Some((
            accelerator.handle(),
            self.accelerator_shadowed(window, message),
        ))
    }

    /// 进行中的 IME 组合、或聚焦输入的编辑保留键先于菜单快捷键。
    ///
    /// 组合可能比焦点活得更久，因此扫描全部文本输入；编辑保留键只在焦点仍是
    /// 文本输入时生效。
    fn accelerator_shadowed(&self, window: &WindowHost, message: &MSG) -> bool {
        if message.message != WM_KEYDOWN && message.message != WM_SYSKEYDOWN {
            return false;
        }
        let Ok(snapshot) = self.runtime.presentation_of(window.ctx) else {
            return false;
        };
        let composing = any_composition(&snapshot);
        if !composing && focused_input(&snapshot).is_none() {
            return false;
        }
        // 安全：GetKeyState 只查询本线程最近一条键盘消息的修饰键状态。
        let control = unsafe { GetKeyState(i32::from(VK_CONTROL.0)) } < 0;
        accelerator_is_shadowed(VIRTUAL_KEY(message.wParam.0 as u16), control, composing)
    }

    /// 从窗口过程借出一次上下文菜单工作；presenter 与请求在菜单期间离开宿主。
    fn begin_context_menu(
        &mut self,
        hwnd: HWND,
        lparam: LPARAM,
    ) -> Option<(Box<dyn ContextMenuPresenter>, ContextMenuRequest)> {
        let key = self.key_for(hwnd)?;
        let (ctx, close_requested) = {
            let window = self.windows.get(key)?;
            (window.ctx, window.close_requested)
        };
        if close_requested || !self.runtime.window_accepts_input(ctx) {
            return None;
        }
        let presenter = self.context_menu_presenter.take()?;
        let Some(menu) = self.runtime.window_context_menu(ctx) else {
            self.context_menu_presenter = Some(presenter);
            return None;
        };
        self.context_menu_active = true;
        Some((
            presenter,
            ContextMenuRequest::new(ctx, hwnd, context_menu_point(hwnd, lparam), menu),
        ))
    }

    /// 菜单关闭后归还 presenter，并在窗口仍然接受输入时派发一次选择。
    ///
    /// 弹窗期间 owner 被关闭、隐藏或进入关闭确认时，返回值被丢弃，不产生 stale
    /// 命令。presenter 只能返回这个窗口声明过的命令 id；未声明的 id 是调用方
    /// 错误，作为宿主错误上报。
    fn finish_context_menu(
        &mut self,
        presenter: Box<dyn ContextMenuPresenter>,
        window: WindowCtx,
        result: Result<Option<&'static str>, HostError>,
    ) -> Result<(), HostError> {
        self.context_menu_active = false;
        self.context_menu_presenter = Some(presenter);
        let Some(id) = result? else {
            return Ok(());
        };
        if !self.runtime.window_accepts_input(window) {
            return Ok(());
        }
        self.dispatch_menu_command(window, id, CommandSource::ContextMenu)
    }

    /// 派发菜单命令；窗口状态的竞态是正常交互结果，其余错误交给消息循环。
    fn dispatch_menu_command(
        &mut self,
        window: WindowCtx,
        id: &'static str,
        source: CommandSource,
    ) -> Result<(), HostError> {
        match self.runtime.dispatch_command(window, id, source) {
            Ok(()) | Err(CommandError::Stale | CommandError::Hidden | CommandError::Suspended) => {
                Ok(())
            }
            Err(error) => Err(HostError::Command(error)),
        }
    }

    /// `WM_COMMAND` 的菜单或快捷键形式：HIWORD=0/1 且 lParam=0。
    ///
    /// 命令 id 必须是这个窗口菜单分配过的稳定项；隐藏、待决关闭与已关闭窗口已由
    /// `handle_window_message` 拒绝，运行时命令路径还会再验证一次。状态回调在
    /// 派发前再次求值，禁用项不运行 handler；窗口状态的竞态是正常交互结果，不
    /// 升级为宿主错误。
    fn menu_command(
        &mut self,
        window: &mut WindowHost,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Result<(), HostError> {
        if lparam.0 != 0 {
            return Ok(());
        }
        let source = match (wparam.0 >> 16) & 0xFFFF {
            0 => CommandSource::MenuBar,
            1 => CommandSource::Accelerator,
            _ => return Ok(()),
        };
        let id = (wparam.0 & 0xFFFF) as u16;
        let Some(command) = window
            .menu
            .as_ref()
            .and_then(|menu| menu.command(id))
            .cloned()
        else {
            return Ok(());
        };
        let state = self
            .runtime
            .menu_state(window.ctx, &command)
            .map_err(HostError::MenuState)?
            .unwrap_or_else(|| command.default_state());
        if !state.enabled() {
            return Ok(());
        }
        self.dispatch_menu_command(window.ctx, command.id(), source)
    }

    /// `WM_INITMENU`/`WM_INITMENUPOPUP`：按只读状态回调设置 enabled/checked。
    ///
    /// 先求值菜单中的全部命令，再统一应用；任一回调失败时菜单保持原状，错误交给
    /// 消息循环。系统菜单不属于应用声明，直接跳过。
    fn apply_menu_state(
        &mut self,
        window: &mut WindowHost,
        menu: HMENU,
        system_menu: bool,
    ) -> Result<(), HostError> {
        if system_menu || menu.is_invalid() {
            return Ok(());
        }
        let Some(native_menu) = window.menu.as_ref() else {
            return Ok(());
        };
        // 安全：menu 由宿主持有的 HWND 拥有。
        let count = unsafe { GetMenuItemCount(Some(menu)) };
        if count <= 0 {
            return Ok(());
        }
        let mut states = Vec::with_capacity(count as usize);
        for position in 0..count {
            // 安全：position 在菜单项范围内；分隔线返回 0，子菜单返回 u32::MAX。
            let id = unsafe { GetMenuItemID(menu, position) };
            if id == 0 || id == u32::MAX {
                continue;
            }
            let Ok(id) = u16::try_from(id) else {
                continue;
            };
            let Some(command) = native_menu.command(id) else {
                continue;
            };
            let state = self
                .runtime
                .menu_state(window.ctx, command)
                .map_err(HostError::MenuState)?
                .unwrap_or_else(|| command.default_state());
            states.push((id, state));
        }
        for (id, state) in states {
            let enable = if state.enabled() {
                MF_ENABLED
            } else {
                MF_GRAYED
            };
            let check = if state.checked() {
                MF_CHECKED.0
            } else {
                MF_UNCHECKED.0
            };
            // 安全：菜单由窗口拥有；命令 id 来自它的原生项。
            unsafe {
                let _ = EnableMenuItem(menu, u32::from(id), MF_BYCOMMAND | enable);
                let _ = CheckMenuItem(menu, u32::from(id), MF_BYCOMMAND.0 | check);
            }
        }
        Ok(())
    }

    /// `WM_ENTERMENULOOP`：菜单接管鼠标与输入，取消组合并清除指针瞬态。
    ///
    /// 按住与 thumb 拖动持有的捕获在宿主借出结束后释放；菜单关闭后由下一次
    /// 指针移动重新建立 hover。
    fn menu_loop_entered(&mut self, window: &mut WindowHost) -> Result<(), HostError> {
        let pressed = window.pressed.take().is_some();
        let dragging = window.scrollbar_drag.take().is_some();
        let hovered = window.hovered.take().is_some();
        window.pointer = None;
        window.tracking_leave = false;
        if pressed || dragging {
            window.release_capture = true;
        }
        let composing = self.cancel_compositions(window)?;
        if pressed || dragging || hovered || composing {
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
    ///
    /// 设备丢失期间返回 [`HostError::Surface`]；读回不会在旧设备上继续。
    pub fn read_frame(&mut self, key: &str) -> Result<Frame, HostError> {
        self.reject_latched_loss()?;
        let stale_proving =
            self.loss_phase == LossPhase::AwaitingPresent && self.loss.epoch() != self.loss_epoch;
        if self.loss_phase == LossPhase::Pending
            || (self.loss_phase == LossPhase::Clear && self.loss.is_lost())
            || stale_proving
        {
            return Err(HostError::Surface(String::from(
                "the device is lost; readback needs the rebuilt device",
            )));
        }
        let hwnd = self
            .hwnd(key)
            .ok_or_else(|| HostError::UnknownWindow(key.to_owned()))?;
        let key = self
            .key_for(hwnd)
            .expect("the HWND belongs to a live window");
        self.with_window(key, |host, window| host.read_frame_inner(window))
    }

    /// 返回窗口当前发布的只读辅助功能树。
    ///
    /// 隐藏窗口、关闭中的窗口以及尚未提交场景的窗口没有发布树，返回
    /// `None`。该入口供隐藏 smoke 与测试检查树形状；可见窗口的 UIA 结果以
    /// Inspect 为准。
    pub fn accessibility_tree(&self, key: &str) -> Option<Arc<AccessibilityTree>> {
        self.windows
            .get(key)
            .and_then(|window| window.accessibility.tree())
    }

    /// 在窗口对应的 UI 线程上执行 provider 排队的动作，再结算请求与重绘。
    fn run_queued_accessibility_actions(&mut self, hwnd: HWND) {
        let Some(key) = self.key_for(hwnd) else {
            return;
        };
        let result = self.with_window(key, |host, window| host.run_accessibility_actions(window));
        if let Err(error) = result {
            self.deferred_error.get_or_insert(error);
        }
        if let Err(error) = self.settle_requests() {
            self.deferred_error.get_or_insert(error);
        }
        if let Err(error) = self.repaint_changed_windows() {
            self.deferred_error.get_or_insert(error);
        }
    }

    /// 按入队顺序执行辅助技术动作；每条都按最新快照重验。
    ///
    /// provider 已按调用时的树过滤过一次；这里重新读取运行时快照，覆盖排队期间
    /// 的卸载、禁用、隐藏与组合状态变化。被拒绝的动作是正常的竞态结果，不产生
    /// 宿主错误；Runtime 的命令路径还会再验证 Node ID 与窗口状态。
    fn run_accessibility_actions(&mut self, window: &mut WindowHost) -> Result<(), HostError> {
        let mut failure = None;
        for action in window.accessibility.take_actions() {
            let action = match action {
                AccessibilityAction::Scroll(command) => {
                    self.apply_accessibility_scroll(window, command);
                    continue;
                }
                action => action,
            };
            let Some(node) = action.node() else {
                continue;
            };
            let Ok(snapshot) = self.runtime.presentation_of(window.ctx) else {
                continue;
            };
            let Some(element) = snapshot.node(node) else {
                continue;
            };
            if element.disabled() {
                continue;
            }
            let dispatched = match action {
                AccessibilityAction::Invoke(_) => {
                    if !element.semantics().has_action(SnapshotAction::Click) {
                        continue;
                    }
                    ignore_expected_dispatch(self.runtime.click_node(node))
                }
                AccessibilityAction::Toggle(_) => {
                    let Some(checked) = element.toggle_state() else {
                        continue;
                    };
                    if !element.semantics().has_action(SnapshotAction::Change) {
                        continue;
                    }
                    ignore_expected_dispatch(self.runtime.change_node(node, !checked))
                }
                AccessibilityAction::SetValue(_, value) => {
                    let editable = element.semantics().has_action(SnapshotAction::Input)
                        && element
                            .edit()
                            .is_some_and(|edit| edit.composition().is_none());
                    if !editable {
                        continue;
                    }
                    ignore_expected_dispatch(self.runtime.replace_text_node(node, &value))
                }
                AccessibilityAction::SetSelection(_, start, end) => {
                    let editable = element.semantics().has_action(SnapshotAction::Input)
                        && element
                            .edit()
                            .is_some_and(|edit| edit.composition().is_none());
                    if !editable {
                        continue;
                    }
                    ignore_expected_dispatch(self.runtime.set_selection_node(node, start, end))
                }
                AccessibilityAction::SetFocus(_) => {
                    if !element.semantics().focusable()
                        || !accessibility::window_is_foreground(window.hwnd())
                    {
                        continue;
                    }
                    ignore_expected_dispatch(self.runtime.focus_node(node))
                }
                AccessibilityAction::Scroll(_) => continue,
            };
            // 继续执行剩余动作；真正失败仍报告给宿主，但不丢弃已经应答的动作。
            if let Err(error) = dispatched {
                failure.get_or_insert(error);
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// UIA 滚动动作的 UI 线程落点：只改宿主呈现偏移，不刷新 runtime。
    ///
    /// provider 已按发布树过滤；这里按当前场景重验窗口与最大偏移，并在两端
    /// 夹取。被拒绝的动作不读取运行时快照，也不触发 reveal。
    fn apply_accessibility_scroll(&mut self, window: &mut WindowHost, command: ScrollCommand) {
        if !self.window_accepts_host_input(window) {
            return;
        }
        let Some(scene) = window.scene.as_ref() else {
            return;
        };
        let max = scene.max_scroll_offset();
        if max == 0 {
            return;
        }
        let page = page_step(Some(scene));
        let line = window.wheel.line_step(page);
        let Some(target) = scroll_target(window.scroll_offset, max, command, page, line) else {
            return;
        };
        self.explicit_scroll(window, target);
    }

    /// `WM_GETOBJECT` 的 provider 交付；只有已发布树的窗口应答根对象。
    fn accessibility_provider(&mut self, hwnd: HWND) -> Option<IRawElementProviderSimple> {
        self.ensure_ole().ok()?;
        let key = self.key_for(hwnd)?;
        let window = self.windows.get(key)?;
        window.accessibility.tree()?;
        Some(accessibility::fragment_root(&window.accessibility))
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
        let snapshot_changed = window.last_snapshot.as_ref() != Some(&snapshot);
        let scene = self.build_scene(window, &snapshot, width, height, snapshot_changed)?;
        let target = target_config(self.format, width, height)?;
        if self.renderer_target != target {
            self.renderer.resize(target);
            self.renderer_target = target;
        }
        self.renderer.set_palette(palette_for(
            snapshot.appearance(),
            self.system_preferences,
            high_contrast_colors,
        ));
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
        // 离屏读回不是呈现：不清除 damage，屏幕上仍是上一帧。场景与缓存
        // 同步供输入与后续绘制使用，避免下一次呈现把同一快照当成新快照
        // 并再次清空滚轮 residue。
        commit_scene(window, &snapshot, scene);
        window.snapshot_revision = self
            .runtime
            .presentation_revision_of(window.ctx)
            .unwrap_or(window.snapshot_revision);
        window.last_snapshot = Some(snapshot);
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
    fn char_typed(
        &mut self,
        window: &mut WindowHost,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Result<(), HostError> {
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
        // 高代理项已经提前返回并保留令牌；其余任何 WM_CHAR 都消费令牌，
        // 未匹配时按规范清除，避免迟到字符被旧按键误伤。
        let suppression = window.char_suppression.take();
        let Some(character) = character else {
            return Ok(());
        };
        if character.is_control() {
            return Ok(());
        }
        let scan_code = scan_code(lparam);
        let snapshot = self
            .runtime
            .presentation_of(window.ctx)
            .map_err(HostError::Snapshot)?;
        if let Some(suppression) = suppression
            && suppression.scan_code == scan_code
            && suppression.at.elapsed() <= CHAR_SUPPRESSION_WINDOW
            && focused_input(&snapshot) == Some(suppression.target)
        {
            return Ok(());
        }
        let Some(focused) = focused_input(&snapshot) else {
            return Ok(());
        };
        let mut buffer = [0_u8; 4];
        let text = character.encode_utf8(&mut buffer);
        // 部分 IME 把候选确认作为 WM_CHAR 送达，而不是 GCS_RESULTSTR；
        // 组合中的字符必须走 commit，否则普通编辑会被 reducer 拒绝。
        if is_composing(&snapshot, focused) {
            ignore_expected_dispatch(self.runtime.compose_commit_node(focused, text))?;
        } else {
            // 可打印字符是宿主 L5 默认，但插入本身在 Event turn 内完成：
            // 一次 `on:input`、一个 `TurnReport`，不形成 Input intent。
            ignore_expected_dispatch(self.runtime.type_text_node(focused, text))?;
        }
        window.reveal_requested = true;
        window.paint_requested = true;
        Ok(())
    }

    /// 一次 `WM_KEYDOWN`：从线程消息队列读取修饰键后进入映射本体。
    fn key_down(
        &mut self,
        window: &mut WindowHost,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Result<(), HostError> {
        // 任何新的 keydown 都让上一条抑制令牌失效；只有本次 turn 消费了按键
        // 才在结算后重新安装。
        window.char_suppression = None;
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
        // AltGr 在 Windows 上表现为 Ctrl+Alt；不能把它当成编辑快捷键，
        // 否则使用 AltGr 布局的字符会被剪切/粘贴吞掉。
        let alt = unsafe { GetKeyState(i32::from(VK_MENU.0)) } < 0;
        if alt {
            // Alt 组合留给系统菜单与加速键预译；AltGr 的字符仍由 WM_CHAR 输入。
            return Ok(());
        }
        let report = self.key_down_with(window, key, shift, control)?;
        if (report.outcome().consumed() || report.outcome().prevent_default())
            && let Ok(snapshot) = self.runtime.presentation_of(window.ctx)
            && let Some(focused) = focused_input(&snapshot)
        {
            // WM_CHAR 只按 scan code 匹配，不按字符；dead key/IME 组合字符
            // 由同一次 keydown 的消费状态决定是否抑制。
            window.char_suppression = Some(CharSuppression {
                target: focused,
                scan_code: scan_code(lparam),
                at: Instant::now(),
            });
        }
        Ok(())
    }

    /// 键盘映射本体：runtime keydown turn 之前的宿主层（组合/IME、编辑快捷键）
    /// 与之后仅存的滚动/绘制；控件默认动作全部由 runtime 结算。
    fn key_down_with(
        &mut self,
        window: &mut WindowHost,
        key: VIRTUAL_KEY,
        shift: bool,
        control: bool,
    ) -> Result<TurnReport, HostError> {
        let modifiers = Modifiers::new(shift, control, false, false);
        if key == VK_TAB {
            self.cancel_compositions(window)?;
            let report = self.dispatch_key_intent(window, Key::Tab, modifiers)?;
            window.reveal_requested = true;
            window.paint_requested = true;
            return Ok(report);
        }
        if key == VK_ESCAPE
            && self
                .runtime
                .confirmation_request(window.ctx)
                .is_ok_and(|request| request.is_some())
        {
            // 组合中的 Escape 只取消组合；否则确认窗口的 Escape 等价于 Deny。
            if self.cancel_compositions(window)? {
                window.paint_requested = true;
                return Ok(TurnReport::default());
            }
            self.runtime
                .request_close(window.ctx)
                .map_err(HostError::Close)?;
            self.focus_after_sync = true;
            window.paint_requested = true;
            return Ok(TurnReport::default());
        }

        let snapshot = self
            .runtime
            .presentation_of(window.ctx)
            .map_err(HostError::Snapshot)?;
        if matches!(key, VK_PRIOR | VK_NEXT) {
            let report = self.dispatch_key_intent(window, host_key(key), modifiers)?;
            if focused_input(&snapshot).is_some_and(|node| is_composing(&snapshot, node))
                || key_default_blocked(&report)
            {
                // 组合期间翻页键归 IME；被 handler 消费的翻页键也不滚动。
                return Ok(report);
            }
            self.scroll_by_key(window, key)?;
            return Ok(report);
        }
        let Some(focused) = focused_input(&snapshot) else {
            // 没有文本输入焦点时仍把 keydown 交给 runtime：焦点控件、窗口
            // fallback handler 与 Tab/Enter/Space 默认都在那里结算。
            let report = self.dispatch_key_intent(window, host_key(key), modifiers)?;
            if matches!(key, VK_HOME | VK_END) && !key_default_blocked(&report) {
                self.scroll_by_key(window, key)?;
            }
            return Ok(report);
        };
        let composing = is_composing(&snapshot, focused);
        if key == VK_ESCAPE {
            // L0：组合中的第一个 Escape 只取消组合（连同 OS IME）；组合结束后
            // Escape 才作为用户 keydown 进入 runtime。
            if composing && self.cancel_compositions(window)? {
                window.paint_requested = true;
                return Ok(TurnReport::default());
            }
            let report = self.dispatch_key_intent(window, Key::Escape, modifiers)?;
            window.paint_requested = true;
            return Ok(report);
        }
        // 组合期间编辑键归 IME：未被 IME 消费而送到宿主的箭头、Home/End、
        // Backspace/Delete 与选择键在这里丢弃，不编辑也不触发显露。
        if composing {
            return Ok(TurnReport::default());
        }
        if let Some(action) = editor_shortcut(key, control, true, composing) {
            match action {
                EditorAction::SelectAll => {
                    ignore_expected_dispatch(self.runtime.select_all_node(focused))?;
                    window.reveal_requested = true;
                    window.paint_requested = true;
                }
                EditorAction::Undo => {
                    ignore_expected_dispatch(self.runtime.undo_node(focused))?;
                    window.reveal_requested = true;
                    window.paint_requested = true;
                }
                EditorAction::Redo => {
                    ignore_expected_dispatch(self.runtime.redo_node(focused))?;
                    window.reveal_requested = true;
                    window.paint_requested = true;
                }
                EditorAction::Copy => self.copy_selection(&snapshot, focused),
                EditorAction::Paste => self.paste_text(window, focused)?,
                EditorAction::Cut => self.cut_selection(window, &snapshot, focused)?,
            }
            return Ok(TurnReport::default());
        }
        // L3 用户 keydown → L4 控件默认全部在 runtime 内完成；宿主只保留
        // L1 快捷键、IME 与 L5 滚动/绘制。
        let report = self.dispatch_key_intent(window, host_key(key), modifiers)?;
        window.reveal_requested = true;
        window.paint_requested = true;
        Ok(report)
    }

    fn dispatch_key_intent(
        &mut self,
        window: &WindowHost,
        key: Key,
        modifiers: Modifiers,
    ) -> Result<TurnReport, HostError> {
        match self
            .runtime
            .dispatch_intent(Intent::keydown_window(window.ctx, key, modifiers))
        {
            Ok(report) => Ok(report),
            Err(
                DispatchError::WrongKind { .. }
                | DispatchError::Rejected { .. }
                | DispatchError::Suspended(_)
                | DispatchError::StaleNode(_)
                | DispatchError::TargetClosed,
            ) => Ok(TurnReport::default()),
            Err(error) => Err(HostError::Input(error)),
        }
    }

    /// 把聚焦输入的已提交选区写入剪贴板；不派发 `on:input`。
    fn copy_selection(&mut self, snapshot: &PresentationSnapshot, focused: NodeId) {
        let _ = self.write_selection(snapshot, focused);
    }

    /// 写入聚焦输入的已提交选区；provider 确认成功时返回 `true`。
    ///
    /// 折叠 selection、没有 provider、provider 失败或 panic 都返回 `false`，
    /// 且不派发 `on:input`。
    fn write_selection(&mut self, snapshot: &PresentationSnapshot, focused: NodeId) -> bool {
        let Some(element) = snapshot.node(focused) else {
            return false;
        };
        let Some((start, end)) = element.edit().and_then(|edit| edit.selection()) else {
            return false;
        };
        if start >= end {
            return false;
        }
        let Some(value) = element.text_value() else {
            return false;
        };
        let selected: String = value.chars().skip(start).take(end - start).collect();
        if selected.is_empty() {
            return false;
        }
        let Some(provider) = self.clipboard.as_mut() else {
            return false;
        };
        // provider 属于嵌入方；panic 与失败一样按无操作处理。
        matches!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                provider.write_text(&selected)
            })),
            Ok(Ok(()))
        )
    }

    /// 先写入选区，成功后再用 Backspace 语义删除它；写入失败不改模型。
    ///
    /// 删除只在 provider 确认后发生，因此剪贴板失败不会丢失文本。成功时
    /// 恰好一次 `on:input`，caret 落在原选区的起点。
    fn cut_selection(
        &mut self,
        window: &mut WindowHost,
        snapshot: &PresentationSnapshot,
        focused: NodeId,
    ) -> Result<(), HostError> {
        if !self.write_selection(snapshot, focused) {
            return Ok(());
        }
        ignore_expected_dispatch(self.runtime.backspace_node(focused))?;
        window.reveal_requested = true;
        window.paint_requested = true;
        Ok(())
    }

    /// 读取剪贴板文本，过滤单行输入不接受的控制字符后插入聚焦输入。
    ///
    /// 空文本、非文本、读取失败或 provider panic 都不改模型；成功时只调用一次
    /// `type_text_node`，因此恰好一次 `on:input`。
    fn paste_text(&mut self, window: &mut WindowHost, focused: NodeId) -> Result<(), HostError> {
        let Some(provider) = self.clipboard.as_mut() else {
            return Ok(());
        };
        let text = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| provider.read_text()))
            .ok()
            .and_then(Result::ok)
            .flatten();
        let Some(text) = text else {
            return Ok(());
        };
        let filtered = single_line_text(&text);
        if filtered.is_empty() {
            return Ok(());
        }
        ignore_expected_dispatch(self.runtime.type_text_node(focused, &filtered))?;
        window.reveal_requested = true;
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
            ignore_expected_dispatch(self.runtime.compose_start_node(node))?;
        }
        window.reveal_requested = true;
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
        window.reveal_requested = true;
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
                        ignore_expected_dispatch(self.runtime.compose_start_node(node))?;
                    }
                    ignore_expected_dispatch(self.runtime.compose_commit_node(node, &text))?;
                }
                None if composing => {
                    ignore_expected_dispatch(self.runtime.compose_cancel_node(node))?;
                }
                None => {}
            }
            composing = false;
        }
        if request.update {
            let text = update.unwrap_or_default();
            if !composing {
                ignore_expected_dispatch(self.runtime.compose_start_node(node))?;
            }
            ignore_expected_dispatch(self.runtime.compose_update_node(node, &text))?;
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
        let composing = self.with_presentation(window, composing_nodes)?;
        let mut cancelled = false;
        for node in composing {
            match self.runtime.compose_cancel_node(node) {
                Ok(()) => cancelled = true,
                Err(error) => ignore_expected_dispatch::<()>(Err(error))?,
            }
        }
        if cancelled {
            self.notify_ime_cancel(window);
            window.reveal_requested = true;
        }
        Ok(cancelled)
    }

    /// 运行时已无聚焦组合、OS IME 组合串仍在时丢弃 OS 组合串。
    ///
    /// 外部 reset（共享 Signal、worker 交付、文件对话框结果等）只在运行时取消
    /// preedit；OS IME 不知道这次取消，会继续接收按键，下一次 `COMPSTR` 把
    /// preedit 重新带回编辑器，候选确认时提交已取消的内容。正常组合路径里
    /// 两者同进同退，不会触碰 IME。
    fn reconcile_ime_composition(&mut self, window: &mut WindowHost, runtime_composing: bool) {
        if !runtime_composing && self.os_composition_active(window) {
            self.notify_ime_cancel(window);
        }
    }

    fn notify_ime_cancel(&mut self, window: &mut WindowHost) {
        #[cfg(test)]
        {
            window.ime_cancel_notifications += 1;
        }
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
        #[cfg(test)]
        if let Some(active) = window.os_composition {
            return active;
        }
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
    /// 组合起点 caret 离开视口时把两个窗口移出工作区，隐藏候选窗。
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
        let point = caret
            .and_then(|caret| ime_candidate_point(scene.viewport(), caret))
            .unwrap_or(HIDDEN_IME_POINT);
        // 安全：HIMC 由 ImmGetContext 获取；两个结构都是本地值。
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

    /// 为新 device 分配代际并安装丢失回调；每次尝试都消耗一个代际。
    fn install_device_callback(&mut self, device: &wgpu::Device) -> u64 {
        let generation = self.next_device_generation.wrapping_add(1).max(1);
        self.next_device_generation = generation;
        install_loss_callback(device, &self.loss, generation);
        generation
    }

    /// 测试注入一次重建失败；只存在于测试构建。
    #[cfg(test)]
    fn inject_rebuild_failure(&mut self) -> Result<(), HostError> {
        if self.injected_rebuild_failures == 0 {
            return Ok(());
        }
        self.injected_rebuild_failures -= 1;
        Err(HostError::Device(String::from("injected rebuild failure")))
    }

    /// 设备丢失后重建 device/queue/renderer 与每个窗口的 surface。
    ///
    /// 先释放所有旧交换链；用第一个窗口的 surface 请求新 device，再让其余
    /// 窗口共享同一 adapter/device/format。失败时保持 surface 为空，由
    /// [`Host::recover_device`] 在尝试上限内重试；达到上限后锁存终止错误。
    fn rebuild_presentations(&mut self) -> Result<(), HostError> {
        let Some(first_key) = self.windows.keys().next().copied() else {
            // 零窗口的常驻宿主没有 surface 可绑定；仍换掉丢失的 device，
            // 保持旧格式与 1×1 目标，窗口重开时再建 surface。
            #[cfg(test)]
            self.inject_rebuild_failure()?;
            let (adapter, device, queue) = request_backend_without_surface(&self._instance)?;
            self.attempt_adapter = Some(adapter_info_of(&adapter));
            let generation = self.install_device_callback(&device);
            let format = self.format;
            let target = target_config(format, 1, 1)?;
            let renderer =
                GpuRenderer::from_parts(&device, &queue, target, RenderPalette::default());
            self.adapter = adapter;
            self.device = device;
            self.queue = queue;
            self.renderer = renderer;
            self.renderer_target = target;
            self.device_generation = generation;
            return Ok(());
        };
        // 旧 surface 属于丢失的 device；先释放，允许同一 HWND 重建交换链。
        for window in self.windows.values_mut() {
            window.surface = None;
        }
        // 注入点在释放之后：测试覆盖的是真实失败留下的空 surface，而不是早退。
        #[cfg(test)]
        self.inject_rebuild_failure()?;
        let first_hwnd = self.windows[&first_key].hwnd();
        let first_surface = create_surface(&self._instance, first_hwnd, self.hinstance)?;
        let (adapter, device, queue) = request_backend(&self._instance, &first_surface)?;
        self.attempt_adapter = Some(adapter_info_of(&adapter));
        let generation = self.install_device_callback(&device);
        let first_config = surface_configuration(&adapter, &first_surface, first_hwnd)?;
        first_surface.configure(&device, &first_config);
        let format = first_config.format;
        let target = target_config(format, first_config.width, first_config.height)?;
        let renderer = GpuRenderer::from_parts(&device, &queue, target, RenderPalette::default());
        let mut staged = Vec::with_capacity(self.windows.len());
        staged.push((first_key, first_surface, first_config));
        for key in self.windows.keys().copied().collect::<Vec<_>>() {
            if key == first_key {
                continue;
            }
            let hwnd = self.windows[&key].hwnd();
            let surface = create_surface(&self._instance, hwnd, self.hinstance)?;
            let config = surface_configuration(&adapter, &surface, hwnd)?;
            surface.configure(&device, &config);
            staged.push((key, surface, config));
        }
        // 全部窗口的 surface 都建好后才替换字段；失败时旧 device 仍在，但
        // 丢失信号会阻止任何旧 surface 的创建或呈现。
        self.adapter = adapter;
        self.device = device;
        self.queue = queue;
        self.format = format;
        self.renderer = renderer;
        self.renderer_target = target;
        for (key, surface, config) in staged {
            let window = self.windows.get_mut(key).expect("the window stays live");
            window.surface = Some(surface);
            window.surface_config = config;
        }
        self.device_generation = generation;
        Ok(())
    }

    fn ensure_surface(&mut self, window: &mut WindowHost) -> Result<(), HostError> {
        let (width, height) = client_size(window.hwnd())?;
        if width == 0 || height == 0 {
            return Ok(());
        }
        if window.surface.is_none() {
            self.reject_latched_loss()?;
            if !self.can_create_surface() {
                return Err(HostError::Surface(String::from(
                    "the device is lost; a surface cannot be created on it",
                )));
            }
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
        snapshot_changed: bool,
    ) -> Result<Scene, HostError> {
        #[cfg(test)]
        {
            self.counters.scene_builds += 1;
        }
        // 安全：HWND 在窗口条目析构前有效。
        let dpi = unsafe { GetDpiForWindow(window.hwnd()) };
        let scale = if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 };
        let viewport = Viewport::new(width as f32 / scale, height as f32 / scale, scale)
            .map_err(HostError::Scene)?;
        if window.last_viewport != Some(viewport) {
            window.last_viewport = Some(viewport);
            window.wheel.reset();
        }
        // 快照变化（增删、过滤、重排）也丢弃未应用的滚轮 residue。
        if snapshot_changed {
            window.wheel.reset();
        }
        let requested = window.scroll_offset;
        let preserve_press = window.preserve_press_on_next_rebuild;
        let mut scene = Scene::build_cached(
            snapshot,
            viewport,
            &scene_env(window, self.system_preferences),
            &mut self.scene_cache,
        )
        .map_err(HostError::Scene)?;
        adopt_scroll_offset(window, &scene, requested);
        // 新场景可能已经移走或卸载了指针下的节点；失效目标不能继续高亮。
        // 滚动触发的重建保留 capture-based 的按压目标。
        if self.revalidate_pointer(window, &scene, !preserve_press) {
            scene = Scene::build_cached(
                snapshot,
                viewport,
                &scene_env(window, self.system_preferences),
                &mut self.scene_cache,
            )
            .map_err(HostError::Scene)?;
            let requested = window.scroll_offset;
            adopt_scroll_offset(window, &scene, requested);
        }
        window.preserve_press_on_next_rebuild = false;
        // 组合期间不绘制 caret，草稿由 preedit 下划线标识；闪烁只在非组合状态生效。
        let composing = focused_composition(snapshot);
        scene.set_caret_visible(window.caret_visible && !composing);
        Ok(scene)
    }

    /// 构建场景，并在需要时按 ADR-0010 把聚焦元素或 caret 显露到视口内。
    ///
    /// 先在当前偏移构建一次；需要移动时重建一次，只有第一遍用了元素边框且
    /// 重建后出现 caret 几何时才做第二遍。显露本身最多两次重建。
    fn build_scene_with_reveal(
        &mut self,
        window: &mut WindowHost,
        snapshot: &PresentationSnapshot,
        width: u32,
        height: u32,
        snapshot_changed: bool,
    ) -> Result<Scene, HostError> {
        let mut scene = self.build_scene(window, snapshot, width, height, snapshot_changed)?;
        if !window.reveal_requested {
            return Ok(scene);
        }
        window.reveal_requested = false;
        let Some(focused) = snapshot.focus() else {
            return Ok(scene);
        };
        let before = window.scroll_offset;
        let used_caret = scene.caret(focused).is_some();
        if let Some(target) = reveal_target(&scene, focused) {
            window.scroll_offset = target;
            scene = self.build_scene(window, snapshot, width, height, false)?;
            if !used_caret
                && scene.caret(focused).is_some()
                && let Some(target) = reveal_target(&scene, focused)
            {
                window.scroll_offset = target;
                scene = self.build_scene(window, snapshot, width, height, false)?;
            }
        }
        // 显露不是滚轮输入：它改变偏移时丢弃尚未应用的滚轮距离，
        // 与键盘滚动和内容重新夹取遵循同一条 ADR-0010 规则。
        if window.scroll_offset != before {
            window.wheel.reset();
        }
        Ok(scene)
    }

    /// 用新场景重新命中最后的指针位置；返回是否需要按新瞬态重建。
    ///
    /// `clear_press` 为 false 时只更新 hover，保留 capture-based 的按压目标，
    /// 用于滚动触发的重建：滚动不重新评估按压，松开时仍按命中结果决定激活。
    fn revalidate_pointer(
        &mut self,
        window: &mut WindowHost,
        scene: &Scene,
        clear_press: bool,
    ) -> bool {
        let Some(point) = window.pointer else {
            return false;
        };
        // 覆盖层滚动条优先：指针在它上面时没有元素 hover，按压也不保留。
        let hit = (!scene.scrollbar().is_some_and(|bar| bar.hit(point)))
            .then(|| scene.hit_test(point))
            .flatten();
        let enabled = hit.filter(|node| scene.node(*node).is_some_and(|e| !e.disabled()));
        let hover_changed = enabled != window.hovered;
        let press_cleared = clear_press
            && window.pressed.is_some_and(|pressed| {
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
    /// 条目让借用检查器看到两者互不重叠。动作期间的窗口过程一律不路由：
    /// IMM 等 Win32 调用可能在动作内同步派发消息，再次借出宿主会在动作仍
    /// 持有 `&mut Host` 时产生第二个可变引用，并把取出的窗口当成已关闭。
    fn with_window<R>(
        &mut self,
        key: &'static str,
        action: impl FnOnce(&mut Host, &mut WindowHost) -> Result<R, HostError>,
    ) -> Result<R, HostError> {
        let Some(mut window) = self.windows.remove(key) else {
            return Err(HostError::UnknownWindow(key.to_owned()));
        };
        let hwnd = window.hwnd();
        let result = self.without_routing(Some(hwnd), |host| action(host, &mut window));
        self.windows.insert(key, window);
        if result.is_ok() && self.loss_phase == LossPhase::Pending && !self.recovering {
            // 可重试的失败已经排入恢复消息；只有锁存的终止错误向上传播。
            if let Err(error) = self.recover_device()
                && self.latched_loss.is_some()
            {
                return Err(error);
            }
        }
        result
    }

    /// 在会同步派发窗口消息的 Win32 调用期间清除所有窗口过程的宿主地址。
    ///
    /// `extra` 用于 `with_window` 已从映射取出的当前窗口。`remove_window`
    /// 销毁窗口时使用自己的清除/恢复路径，不经过这里。可以嵌套：内层动作
    /// 不提前恢复路由，否则外层动作仍持有 `&mut Host` 时会重新开放重入。
    fn without_routing<R>(
        &mut self,
        extra: Option<HWND>,
        action: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let pointer = self.dispatching.filter(|_| !self.routing_suppressed);
        let Some(pointer) = pointer else {
            return action(self);
        };
        self.routing_suppressed = true;
        for hwnd in self.routing_hwnds(extra) {
            // 安全：只清除窗口过程的宿主地址槽。
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
        }
        let result = action(self);
        // `extra` 是已从映射取出的当前窗口；调用方保证它在动作后仍存活
        // （销毁窗口走 `remove_window` 的专用路径）。
        for hwnd in self.routing_hwnds(extra) {
            // 安全：恢复派发期间的宿主地址。
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, pointer as isize);
            }
        }
        self.routing_suppressed = false;
        result
    }

    fn routing_hwnds(&self, extra: Option<HWND>) -> Vec<HWND> {
        let mut hwnds: Vec<HWND> = self.windows.values().map(WindowHost::hwnd).collect();
        if let Some(tray) = &self.tray {
            hwnds.push(tray.window.hwnd);
        }
        if let Some(extra) = extra
            && !hwnds.contains(&extra)
        {
            hwnds.push(extra);
        }
        hwnds
    }

    fn mark_all_windows_dirty(&mut self) {
        for window in self.windows.values_mut() {
            window.paint_requested = true;
        }
    }

    fn clear_pressed_except(&mut self, key: &'static str) {
        for (other_key, other) in &mut self.windows {
            if *other_key == key {
                continue;
            }
            let pressed = other.pressed.take().is_some();
            let drag = other.scrollbar_drag.take().is_some();
            if pressed || drag {
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
        if let Some(pointer) = self.dispatching.filter(|_| !self.routing_suppressed) {
            // 安全：窗口过程只在派发期间读取这个槽。
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, pointer as isize);
            }
        }
        self.hwnds.insert(hwnd.0 as isize, key);
        self.windows.insert(key, window);
    }

    fn add_window(&mut self, info: &WindowInfo) -> Result<WindowAdd, HostError> {
        self.reject_latched_loss()?;
        if !self.can_create_surface() {
            // 丢失信号可能先于 `DEVICE_LOSS` 消息置位，消息也可能随旧目标
            // 丢弃；`AwaitingPresent` 后重建设备再次丢失时同样没有消息推动。
            // 新窗口必须在有效设备上创建 surface，先补一次重建。
            self.loss_phase = LossPhase::Pending;
            if let Err(error) = self.recover_device() {
                if self.latched_loss.is_some() {
                    return Err(error);
                }
                // 重建失败已排入恢复消息；请求留到下一次同步重试。
                return Ok(WindowAdd::Deferred);
            }
            if !self.can_create_surface() {
                // 重建期间又发生丢失：信号保持置位，请求留到下一次同步重试。
                return Ok(WindowAdd::Deferred);
            }
        }
        let menu = build_window_menu(&self.runtime, info.window)?;
        let (window, menu) = NativeWindow::new(info.spec, self.hinstance, menu)?;
        let menu = complete_window_menu(menu)?;
        let hwnd = window.hwnd;
        // 安全：HWND 在窗口条目析构前有效；定时器随窗口销毁。
        let blink_timer = unsafe { SetTimer(Some(hwnd), CARET_BLINK_TIMER, CARET_BLINK_MS, None) };
        let blink_timer = (blink_timer != 0).then_some(blink_timer);
        let surface = create_surface(&self._instance, hwnd, self.hinstance)?;
        let config = surface_configuration(&self.adapter, &surface, hwnd)?;
        if config.format != self.format {
            // 零窗口恢复时用旧格式建了过渡 renderer；窗口的 surface 格式可能
            // 不同，必须在第一次绘制前换到新格式。
            self.format = config.format;
            let target = target_config(self.format, config.width, config.height)?;
            self.renderer = GpuRenderer::from_parts(
                &self.device,
                &self.queue,
                target,
                RenderPalette::default(),
            );
            self.renderer_target = target;
        }
        surface.configure(&self.device, &config);
        self.insert_window(WindowHost::new(
            window,
            menu,
            info.window,
            info.spec.title().to_owned(),
            surface,
            config,
            blink_timer,
        ));
        Ok(WindowAdd::Added)
    }

    fn remove_window(&mut self, key: &'static str) {
        let Some(window) = self.windows.remove(key) else {
            return;
        };
        self.hwnds.remove(&(window.hwnd().0 as isize));
        // DestroyWindow 会同步派发 WM_DESTROY 并可能激活其它窗口；整个销毁
        // 期间清除所有窗口过程的宿主地址。外层已抑制路由时不要提前恢复。
        let pointer = self.dispatching.filter(|_| !self.routing_suppressed);
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
        self.mark_all_windows_dirty();
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
            let mut deferred = false;
            for info in &infos {
                if !self.windows.contains_key(info.spec.key())
                    && self.add_window(info)? == WindowAdd::Deferred
                {
                    deferred = true;
                    break;
                }
            }
            // 只有全部窗口都就绪才提交修订号；失败或推迟时下一次同步会重试。
            if !deferred {
                self.revision = revision;
            }
        }
        self.sync_visibility();
        self.sync_enabled()?;
        self.sync_drop_targets()?;
        self.sync_accessibility();
        self.update_wake_target();
        self.sync_tray()?;
        Ok(())
    }

    /// 隐藏与关闭中的窗口不发布可访问性树；缓存的 provider 解析失败。
    fn sync_accessibility(&mut self) {
        let states: Vec<(&'static str, bool)> = self
            .windows
            .iter()
            .map(|(key, window)| {
                (
                    *key,
                    !window.close_requested && self.runtime.window_visible(window.ctx),
                )
            })
            .collect();
        for (key, live) in states {
            if !live && let Some(window) = self.windows.get_mut(key) {
                window.accessibility.clear();
            }
        }
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
        window.wheel.reset();
        // 隐藏保留焦点与偏移；显示按 ADR 不触发显露，因此丢弃待处理请求。
        window.reveal_requested = false;
        window.paint_requested = true;
        if !show {
            window.clear_pointer_transients();
        }
        // 路由关闭期间其它窗口可能收到但跳过了重绘消息；统一标记后重绘。
        self.mark_all_windows_dirty();
    }

    /// 把原生启用状态对齐到运行时输入仲裁；待决关闭的父窗口被禁用。
    fn sync_enabled(&mut self) -> Result<(), HostError> {
        let states: Vec<(&'static str, bool)> = self
            .windows
            .iter()
            .map(|(key, window)| (*key, self.runtime.window_accepts_input(window.ctx)))
            .collect();
        for (key, enabled) in states {
            self.set_native_enabled(key, enabled)?;
        }
        Ok(())
    }

    /// 只给可见且注册了 `on_drop` 的窗口安装 OLE drop target。
    ///
    /// 隐藏宿主从不初始化 OLE，也不注册 target；窗口隐藏、关闭或失去处理器时
    /// 撤销注册。重新显示或重开会创建新的 target 与空队列。启用状态由 COM
    /// 回调在每次拖动时检查，窗口重新启用不必重建 target。
    fn sync_drop_targets(&mut self) -> Result<(), HostError> {
        if !self.show_windows {
            for window in self.windows.values_mut() {
                window.drop_target = None;
            }
            return Ok(());
        }
        let states: Vec<(&'static str, bool, HWND)> = self
            .windows
            .iter()
            .map(|(key, window)| {
                (
                    *key,
                    should_register(
                        self.show_windows,
                        self.runtime.window_visible(window.ctx),
                        self.runtime.window_has_drop_handler(window.ctx),
                    ),
                    window.hwnd(),
                )
            })
            .collect();
        for (key, should, hwnd) in states {
            if !should {
                if let Some(window) = self.windows.get_mut(key) {
                    window.drop_target = None;
                }
                continue;
            }
            if self
                .windows
                .get(key)
                .is_some_and(|window| window.drop_target.is_some())
            {
                continue;
            }
            self.ensure_ole()
                .map_err(|error| HostError::DragDrop(DragDropError::from_ole(error)))?;
            let target = DropRegistration::register(hwnd).map_err(HostError::DragDrop)?;
            if let Some(window) = self.windows.get_mut(key) {
                window.drop_target = Some(target);
            }
        }
        Ok(())
    }

    /// 派发一条 `DROP`：执行本窗口排队的交付，再结算处理器产生的请求与重绘。
    fn drain_drop_message(&mut self, hwnd: HWND) {
        let Some(key) = self.key_for(hwnd) else {
            return;
        };
        let result = self.with_window(key, |host, window| host.drain_drop(window));
        if let Err(error) = result {
            self.deferred_error.get_or_insert(error);
        }
        if let Err(error) = self.settle_requests() {
            self.deferred_error.get_or_insert(error);
        }
        if let Err(error) = self.repaint_changed_windows() {
            self.deferred_error.get_or_insert(error);
        }
    }

    /// 取走 OLE target 排队的路径并交给运行时；一次 drop 只运行一次处理器。
    fn drain_drop(&mut self, window: &mut WindowHost) -> Result<(), HostError> {
        let paths = window
            .drop_target
            .as_ref()
            .and_then(DropRegistration::take_pending);
        let Some(paths) = paths else {
            return Ok(());
        };
        // OLE 接受与这条消息的派发之间，窗口可能被隐藏、挂起或关闭；
        // 运行时拒绝这些已经无效的投递是正常的竞态结果，不是宿主失败。
        match self.runtime.drop_files(window.ctx, paths) {
            Ok(())
            | Err(
                zenui::DropError::Stale
                | zenui::DropError::Hidden
                | zenui::DropError::Suspended
                | zenui::DropError::Empty
                | zenui::DropError::NoHandler,
            ) => {}
            Err(error) => return Err(HostError::Drop(error)),
        }
        window.reveal_requested = true;
        window.paint_requested = true;
        Ok(())
    }

    /// 切换一个窗口的原生启用状态；禁用前取消 IME 组合并清除指针瞬态。
    fn set_native_enabled(&mut self, key: &'static str, enabled: bool) -> Result<(), HostError> {
        let Some(window) = self.windows.get(key) else {
            return Ok(());
        };
        if window.enabled == enabled {
            return Ok(());
        }
        if !enabled && self.runtime.window_visible(window.ctx) {
            self.with_window(key, |host, window| host.cancel_compositions(window))?;
        }
        let Some(hwnd) = self.windows.get(key).map(WindowHost::hwnd) else {
            return Ok(());
        };
        // EnableWindow 可能同步派发焦点/启用消息；调用期间禁止路由。
        self.without_routing(None, |_| unsafe {
            let _ = EnableWindow(hwnd, enabled);
        });
        let Some(window) = self.windows.get_mut(key) else {
            return Ok(());
        };
        window.enabled = enabled;
        if !enabled {
            window.clear_pointer_transients();
        }
        self.mark_all_windows_dirty();
        Ok(())
    }

    fn update_wake_target(&self) {
        let target = self
            .windows
            .values()
            .next()
            .map(WindowHost::hwnd)
            // 常驻会话在零窗口时仍要交付应用任务；托盘 HWND 是唯一的消息入口。
            .or_else(|| self.tray.as_ref().map(|tray| tray.window.hwnd));
        let changed = self.wake.set_target(target);
        // 有唤醒在途而目标被销毁时，把那条唤醒转投到仍存活的窗口；
        // 否则 pending 会永远为 true，后续完成不再投递。
        if changed
            && self.wake.is_pending()
            && let Some(target) = target
            && unsafe { PostMessageW(Some(target), host_message::WAKE, WPARAM(0), LPARAM(0)) }
                .is_err()
        {
            self.wake.observed();
        }
        // 丢失消息也跟随当前目标；旧目标上的在途消息可能随窗口销毁而丢失，
        // 因此换目标时释放合并标志，只在等待重建时向新目标重发。
        let loss_target_changed = self.loss.set_target(target);
        if loss_target_changed {
            self.loss.observed();
            // `AwaitingPresent` 若仍是同一丢失序号，只需要等首次呈现；
            // 新序号才代表重建后的设备又丢了。
            let needs_rebuild = self.loss_phase != LossPhase::AwaitingPresent
                || self.loss.epoch() != self.loss_epoch;
            if self.loss.is_lost() && needs_rebuild {
                self.loss.post_from_host(self.device_generation);
            }
        }
    }

    /// 逐窗口比较最新快照与瞬时重绘请求；只重绘确有变化的窗口。
    ///
    /// 运行时呈现版本没有变化时复用缓存快照：滚动、hover、pressed 与 caret
    /// 闪烁都是宿主瞬态，不重建运行时快照，也不复制它。
    fn repaint_changed_windows(&mut self) -> Result<(), HostError> {
        for key in self.windows.keys().copied().collect::<Vec<_>>() {
            let Some(window) = self.windows.get(&key) else {
                continue;
            };
            let revision = self
                .runtime
                .presentation_revision_of(window.ctx)
                .unwrap_or(window.snapshot_revision);
            if window.last_snapshot.is_some() && window.snapshot_revision == revision {
                if window.paint_requested {
                    self.with_window(key, |host, window| host.render_cached(window))?;
                }
                continue;
            }
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
            let composing = focused_composition(&snapshot);
            self.with_window(key, |host, window| {
                // 快照已在上面提交外部 reset；先让 OS IME 与运行时组合状态一致，
                // 再呈现这一帧。
                host.reconcile_ime_composition(window, composing);
                host.render_window(window, snapshot)
            })?;
        }
        Ok(())
    }
}

impl WindowHost {
    fn new(
        native: NativeWindow,
        menu: Option<WindowMenu>,
        ctx: WindowCtx,
        title: String,
        surface: wgpu::Surface<'static>,
        surface_config: wgpu::SurfaceConfiguration,
        blink_timer: Option<usize>,
    ) -> Self {
        let accessibility = Arc::new(WindowAccessibility::new(native.hwnd));
        Self {
            surface: Some(surface),
            surface_config,
            drop_target: None,
            native,
            menu,
            ctx,
            scene: None,
            pointer: None,
            hovered: None,
            pressed: None,
            release_capture: false,
            tracking_leave: false,
            keyboard_modality: false,
            pending_high_surrogate: None,
            char_suppression: None,
            caret_visible: true,
            blink_timer,
            close_requested: false,
            enabled: true,
            scroll_offset: 0,
            wheel: WheelScroll::from_system(),
            last_viewport: None,
            preserve_press_on_next_rebuild: false,
            scrollbar_drag: None,
            reveal_requested: false,
            paint_requested: true,
            last_snapshot: None,
            snapshot_revision: 0,
            title,
            accessibility,
            accessibility_ids: AccessibilityIds::new(),
            #[cfg(test)]
            os_composition: None,
            #[cfg(test)]
            ime_cancel_notifications: 0,
            #[cfg(test)]
            foreground: None,
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

    /// 指针瞬态在隐藏、禁用或销毁后不能残留。
    fn clear_pointer_transients(&mut self) {
        self.pointer = None;
        self.hovered = None;
        self.pressed = None;
        self.scrollbar_drag = None;
        self.release_capture = false;
        self.tracking_leave = false;
    }
}

/// 文件对话框把宿主 OLE 失败映射为它的公开错误。
fn file_dialog_ole_error(error: OleError) -> FileDialogError {
    match error {
        OleError::ChangedMode => FileDialogError::Unsupported,
        OleError::Failed(message) => FileDialogError::DialogFailed(message),
    }
}

/// 聚焦元素的显露目标：优先 caret 几何，否则用元素边框。
///
/// 返回新的偏移；`None` 表示不需要移动。几何已经是场景坐标（已按当前偏移
/// 平移）。
fn reveal_target(scene: &Scene, focused: NodeId) -> Option<i32> {
    let element = scene.node(focused)?;
    let rect = scene.caret(focused).unwrap_or_else(|| element.rect());
    reveal_offset(
        scene.viewport().height(),
        rect,
        scene.scroll_offset(),
        scene.max_scroll_offset(),
    )
}

/// ADR-0010 的显露规则：顶部优先，只做垂直移动，夹到 `[0, max]`。
fn reveal_offset(viewport_height: f32, rect: Rect, applied: i32, max: i32) -> Option<i32> {
    let delta = if rect.y < 0.0 {
        rect.y.floor() as i32
    } else if rect.bottom() > viewport_height {
        (rect.bottom() - viewport_height).ceil() as i32
    } else {
        return None;
    };
    let target = applied.saturating_add(delta).clamp(0, max);
    (target != applied).then_some(target)
}

/// ADR-0010 的翻页步长：保留 24 逻辑像素重叠，至少 1 像素。
fn page_step(scene: Option<&Scene>) -> i32 {
    scene.map_or(1, |scene| {
        ((scene.viewport().height() - 24.0).floor() as i32).max(1)
    })
}

/// 一次 UIA 滚动请求的夹取落点；返回 `None` 表示无需改变偏移。
fn scroll_target(
    current: i32,
    max: i32,
    request: ScrollCommand,
    page: i32,
    line: i32,
) -> Option<i32> {
    let target = match request {
        ScrollCommand::PageUp => current.saturating_sub(page),
        ScrollCommand::PageDown => current.saturating_add(page),
        ScrollCommand::LineUp => current.saturating_sub(line),
        ScrollCommand::LineDown => current.saturating_add(line),
        ScrollCommand::Percent(percent) => {
            if !percent.is_finite() || !(0.0..=100.0).contains(&percent) {
                return None;
            }
            percent_offset(percent, max)
        }
    };
    let target = target.clamp(0, max);
    (target != current).then_some(target)
}

/// 0..=100 映射到 `[0, max]`，按最近整数逻辑像素四舍五入，半值远离零。
fn percent_offset(percent: f64, max: i32) -> i32 {
    (f64::from(max) * (percent / 100.0)).round() as i32
}

/// 指针下的悬停目标；覆盖层滚动条优先，落在它上面时没有元素 hover。
fn hover_at(scene: &Scene, point: Point) -> Option<NodeId> {
    if scene.scrollbar().is_some_and(|bar| bar.hit(point)) {
        return None;
    }
    let node = scene.hit_test(point)?;
    (!scene.node(node)?.disabled()).then_some(node)
}

/// thumb 拖动落点：指针位置与抓取偏移换算为 `[0, max]` 内的滚动偏移。
///
/// 没有行程（thumb 填满轨道）或内容不可滚动时返回 `None`，拖动保持原偏移。
fn scrollbar_drag_offset(
    track: Rect,
    thumb: Rect,
    grab: f32,
    pointer_y: f32,
    max: i32,
) -> Option<i32> {
    let travel = track.height - thumb.height;
    if travel <= 0.0 || max <= 0 {
        return None;
    }
    let fraction = ((pointer_y - grab - track.y) / travel).clamp(0.0, 1.0);
    Some((fraction * max as f32).round() as i32)
}

/// 采用场景夹取后的偏移；被夹取时丢弃滚轮 residue。
fn adopt_scroll_offset(window: &mut WindowHost, scene: &Scene, requested: i32) {
    let applied = scene.scroll_offset();
    if applied != requested {
        window.scroll_offset = applied;
        window.wheel.reset();
    }
}

fn scene_env(window: &WindowHost, preferences: SystemPreferences) -> SceneEnv {
    SceneEnv::default()
        .with_hovered(window.hovered)
        .with_pressed(window.pressed)
        .with_focus_visible(window.keyboard_modality)
        .with_scroll_offset(window.scroll_offset)
        .with_text_scale_percent(preferences.text_scale_percent())
}

impl Drop for Host {
    fn drop(&mut self) {
        // Drop 不能返回错误；先移除通知区域图标，再阻断 worker 唤醒并释放
        // 每个窗口的 activation。
        self.remove_tray_icon();
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

/// 当前 device 使用的适配器诊断信息。
///
/// 用于日志、验收记录与 [`DeviceLostError`]；不参与渲染能力协商。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuAdapterInfo {
    /// 适配器名称。
    pub name: String,
    /// 后端（`Dx12`、`Vulkan` 等）。
    pub backend: String,
    /// 设备类型（`DiscreteGpu`、`IntegratedGpu`、`Cpu` 等）。
    pub device_type: String,
    /// 驱动名称。
    pub driver: String,
    /// 驱动版本或附加信息。
    pub driver_info: String,
    /// 是否是软件回退适配器（WARP）；`device_type` 为 `Cpu`。
    pub fallback: bool,
}

/// 三次恢复尝试后设备仍不可用时的终止错误。
#[derive(Clone, Debug)]
pub struct DeviceLostError {
    /// 本轮丢失事件消耗的恢复尝试次数。
    pub attempts: u32,
    /// 最后一次尝试使用的适配器；没有拿到适配器时是当前 device 的适配器。
    pub adapter: GpuAdapterInfo,
    /// 最后一次尝试的错误描述。
    pub message: String,
}

impl fmt::Display for DeviceLostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "device lost after {} attempts on {} ({}): {}",
            self.attempts, self.adapter.name, self.adapter.backend, self.message
        )
    }
}

impl Error for DeviceLostError {}

/// 一次丢失事件允许的恢复尝试次数。
const MAX_RECOVERY_ATTEMPTS: u32 = 3;

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
    /// 设备在三次恢复尝试后仍然不可用；后续调用返回同一个错误。
    ///
    /// 载荷装箱：`HostError` 出现在每帧都返回的 `Result` 里，不能让诊断字段
    /// 撑大所有调用点的返回值。
    DeviceLost(Box<DeviceLostError>),
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
    /// 宿主完成文件选择请求失败；stale 结果已在内部忽略。
    FileDialog(FileDialogRequestError),
    /// 原生菜单构建失败。
    Menu(MenuError),
    /// 原生快捷键表构建失败。
    Accelerator(MenuError),
    /// 窗口级上下文菜单构建失败。
    ContextMenu(MenuError),
    /// 上下文菜单 presenter 失败；panic 不能穿过窗口过程的系统边界。
    ContextMenuPresenter(String),
    /// 托盘窗口、广播注册或通知区域操作失败。
    Tray(String),
    /// 托盘右键菜单构建失败。
    TrayMenu(MenuError),
    /// 托盘 presenter 失败；panic 不能穿过窗口过程的系统边界。
    TrayPresenter(String),
    /// 应用会话策略、显式退出或会话动作失败。
    Session(SessionError),
    /// 菜单命令在运行时命令路径上失败；窗口状态竞态已在调用方过滤。
    Command(CommandError),
    /// 菜单状态回调失败；回调内的写入已被只读作用域拒绝。
    MenuState(MenuStateError),
    /// 文本资产在创建窗口之前校验失败。
    TextAssets(zenui_render::TextAssetError),
    /// 场景构建失败。
    Scene(zenui_render::SceneError),
    /// GPU 渲染失败。
    Render(zenui_render::RenderError),
    /// 指针命中后的运行时派发失败。
    Input(DispatchError),
    /// OLE drop target 注册失败。
    DragDrop(DragDropError),
    /// 拖放已交给运行时但处理器或刷新失败。
    Drop(zenui::DropError),
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
            Self::DeviceLost(error) => write!(f, "device lost: {error}"),
            Self::UnknownWindow(key) => write!(f, "window key {key} has no native window"),
            Self::Open(error) => write!(f, "opening the window failed: {error}"),
            Self::Close(error) => write!(f, "closing the window failed: {error}"),
            Self::Snapshot(error) => write!(f, "reading the window snapshot failed: {error}"),
            Self::WindowState(error) => write!(f, "window state change failed: {error}"),
            Self::FileDialog(error) => write!(f, "file dialog request failed: {error}"),
            Self::Menu(error) => write!(f, "menu creation failed: {error}"),
            Self::Accelerator(error) => write!(f, "accelerator table creation failed: {error}"),
            Self::ContextMenu(error) => write!(f, "context menu creation failed: {error}"),
            Self::ContextMenuPresenter(message) => {
                write!(f, "context menu presentation failed: {message}")
            }
            Self::Tray(message) => write!(f, "tray icon failed: {message}"),
            Self::TrayMenu(error) => write!(f, "tray menu creation failed: {error}"),
            Self::TrayPresenter(message) => {
                write!(f, "tray presentation failed: {message}")
            }
            Self::Session(error) => write!(f, "application session failed: {error}"),
            Self::Command(error) => write!(f, "menu command failed: {error}"),
            Self::MenuState(error) => write!(f, "menu state query failed: {error}"),
            Self::TextAssets(error) => write!(f, "text assets failed: {error}"),
            Self::Scene(error) => write!(f, "scene failed: {error}"),
            Self::Render(error) => write!(f, "render failed: {error}"),
            Self::Input(error) => write!(f, "pointer dispatch failed: {error}"),
            Self::DragDrop(error) => write!(f, "file drop target failed: {error}"),
            Self::Drop(error) => write!(f, "dropped files failed: {error}"),
            Self::MessageLoop => f.write_str("the Win32 message loop failed"),
        }
    }
}

impl Error for HostError {}

/// 从目录声明构建窗口菜单；空声明不创建原生菜单栏，也不占非客户区。
fn build_window_menu(
    runtime: &Runtime,
    window: WindowCtx,
) -> Result<Option<NativeMenu>, HostError> {
    runtime
        .window_menu(window)
        .filter(|menu| !menu.items().is_empty())
        .map(|menu| NativeMenu::build(&menu))
        .transpose()
        .map_err(HostError::Menu)
}

/// 为已挂接 HWND 的菜单补上同一代的快捷键表；没有菜单时为 `None`。
fn complete_window_menu(menu: Option<NativeMenu>) -> Result<Option<WindowMenu>, HostError> {
    menu.map(WindowMenu::build)
        .transpose()
        .map_err(HostError::Accelerator)
}

/// 应用托盘：自己的顶层 HWND、共享图标与声明。
///
/// 顶层 popup 让 Explorer 的 `TaskbarCreated` 广播与托盘回调都能到达；它从不
/// 显示，只承载通知区域句柄。`added` 记录 presenter 侧是否仍有图标，保证
/// 移除幂等。
struct TrayHost {
    window: TrayMessageWindow,
    spec: TraySpec,
    icon: HICON,
    added: bool,
}

/// 托盘消息窗口；宿主在析构前先移除图标，再销毁这个 HWND。
struct TrayMessageWindow {
    hwnd: HWND,
}

impl TrayMessageWindow {
    fn new(hinstance: HINSTANCE) -> Result<Self, HostError> {
        // 安全：类已注册；无父窗口的顶层 popup 接收 Explorer 广播，且保持隐藏。
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("ZenUIHostWindow"),
                w!("ZenUI Tray"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(hinstance),
                None,
            )
        }
        .map_err(|error| HostError::Tray(error.to_string()))?;
        Ok(Self { hwnd })
    }
}

impl Drop for TrayMessageWindow {
    fn drop(&mut self) {
        // 安全：HWND 由本结构独占；重复销毁只返回错误。
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

struct NativeWindow {
    hwnd: HWND,
    /// 原生窗口当前是否由 `ShowWindow(SW_SHOW)` 显示；隐藏模式始终为 false。
    shown: bool,
}

impl NativeWindow {
    fn new(
        spec: WindowSpec,
        hinstance: HINSTANCE,
        mut menu: Option<NativeMenu>,
    ) -> Result<(Self, Option<NativeMenu>), HostError> {
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
        // 安全：rect 与 style 来自本地值；菜单栏计入非客户区。
        unsafe {
            AdjustWindowRectExForDpi(
                &mut rect,
                style,
                menu.is_some(),
                WINDOW_EX_STYLE::default(),
                dpi.max(96),
            )
        }
        .map_err(|error| HostError::Window(error.to_string()))?;
        // 安全：类名已在 register_class 注册；标题、菜单与实例句柄在调用期间有效。
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
                menu.as_ref().map(NativeMenu::handle),
                Some(hinstance),
                None,
            )
        }
        .map_err(|error| HostError::Window(error.to_string()))?;
        // 窗口创建成功后菜单归 Win32 所有；创建失败时菜单仍由局部值注销。
        if let Some(menu) = menu.as_mut() {
            menu.assigned_to_window();
        }
        Ok((Self { hwnd, shown: false }, menu))
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
    request_backend_inner(instance, Some(surface))
}

/// 零窗口恢复没有 surface 可兼容；仍要换掉已丢失的 device，等窗口重开再建
/// surface。适配器选择与有 surface 时相同。
fn request_backend_without_surface(
    instance: &wgpu::Instance,
) -> Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue), HostError> {
    request_backend_inner(instance, None)
}

fn request_backend_inner(
    instance: &wgpu::Instance,
    surface: Option<&wgpu::Surface<'_>>,
) -> Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue), HostError> {
    let mut options = wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: surface,
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

/// 读取适配器的诊断信息；`Cpu` 设备类型表示软件回退（WARP）。
fn adapter_info_of(adapter: &wgpu::Adapter) -> GpuAdapterInfo {
    let info = adapter.get_info();
    GpuAdapterInfo {
        name: info.name,
        backend: format!("{:?}", info.backend),
        device_type: format!("{:?}", info.device_type),
        driver: info.driver,
        driver_info: info.driver_info,
        fallback: info.device_type == wgpu::DeviceType::Cpu,
    }
}

/// Presenter 可能返回与请求 kind 不匹配的路径数量；宿主不能把无效的
/// `Picked` 交给应用。单路径 kind 必须恰好一个，打开多个至少一个。
fn validated_file_dialog_outcome(
    options: &FileDialogOptions,
    outcome: FileDialogOutcome,
) -> FileDialogOutcome {
    if let FileDialogOutcome::Picked(paths) = &outcome {
        let valid = if options.allows_multiple() {
            !paths.is_empty()
        } else {
            paths.len() == 1
        };
        if !valid {
            let expected = if options.allows_multiple() {
                "at least one path"
            } else {
                "exactly one path"
            };
            return FileDialogOutcome::Failed(FileDialogError::DialogFailed(format!(
                "the file dialog returned {} paths; {expected} is required for {:?}",
                paths.len(),
                options.kind()
            )));
        }
    }
    outcome
}

/// 命中元素可以没有对应 handler（例如静态按钮）、处于不可聚焦、挂起或已卸载
/// 状态，或在派发过程中被 handler 重挂载而失效；这些拒绝是正常交互结果，
/// 不是宿主失败。指针、键盘与 UIA 动作共用这一个判定。
fn ignore_expected_dispatch<T>(result: Result<T, DispatchError>) -> Result<(), HostError> {
    match result {
        Ok(_)
        | Err(
            DispatchError::WrongKind { .. }
            | DispatchError::Rejected { .. }
            | DispatchError::Suspended(_)
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

/// 一次被消费的 keydown；下一条同 target/scan code 的 WM_CHAR 必须丢弃。
#[derive(Clone, Copy)]
struct CharSuppression {
    target: NodeId,
    scan_code: u32,
    at: Instant,
}

const CHAR_SUPPRESSION_WINDOW: Duration = Duration::from_millis(50);

/// 扫描码来自键盘消息的 LPARAM 16..23 位；测试消息通常为 0。
const fn scan_code(lparam: LPARAM) -> u32 {
    ((lparam.0 >> 16) & 0xff) as u32
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

/// 把已应用的滚动偏移与视口高度换算成行窗口（ADR-0011）。
///
/// `first` 是视口顶端所在行，`count` 覆盖视口内的完整/部分行；行带完整地
/// 在视口外时仍挂载一行，给焦点与显露保留锚点。行带总范围不受窗口影响。
fn row_window_for(band: SceneRowBand, offset: f32, viewport_height: f32) -> Option<(usize, usize)> {
    let row_height = band.row_height();
    let total = band.total_rows();
    if !row_height.is_finite() || row_height <= 0.0 || total == 0 {
        return None;
    }
    let local_top = offset - band.top();
    let visible_end = local_top + viewport_height.max(0.0);
    let total_f = total as f32;
    let first = (local_top / row_height).floor().clamp(0.0, total_f) as usize;
    let end = (visible_end / row_height).ceil().clamp(0.0, total_f) as usize;
    Some(if end > first {
        (first, end - first)
    } else if first >= total {
        (total - 1, 1)
    } else {
        (first, 1)
    })
}

/// `content_y` 在行带里所属的行；行带外或退化行高返回 `None`。
fn row_containing(band: SceneRowBand, content_y: f32) -> Option<usize> {
    let row_height = band.row_height();
    if !row_height.is_finite() || row_height <= 0.0 {
        return None;
    }
    let row = (content_y - band.top()) / row_height;
    (row >= 0.0 && row < band.total_rows() as f32).then_some(row.floor() as usize)
}

/// 第一遍场景之后、在场景构建调用栈之外应用的行窗口集合。
///
/// pending 是**按 band 的集合**而不是单个 `Option<RowWindow>`：同一屏可以
/// 有多个窗口化 `For`，其中还可以有嵌套行带。
struct PendingRowWindows {
    bands: Vec<(u64, RowWindow)>,
}

impl PendingRowWindows {
    fn is_empty(&self) -> bool {
        self.bands.is_empty()
    }
}

/// 第一遍场景与需要随后结算的行窗口目标。
struct PendingScene {
    scene: Scene,
    row_windows: PendingRowWindows,
}

/// 从第一遍场景的行带几何计算每个窗口化 `For` 的目标挂载窗口。
///
/// 纯计算：不触碰 runtime，也不运行任何用户 factory。目标与当前请求相同
/// 时由 `set_row_windows` 的快速路径吸收，不会触发刷新。
fn pending_row_windows(scene: &Scene) -> PendingRowWindows {
    let offset = scene.scroll_offset() as f32;
    let height = scene.viewport().height();
    let bands = scene
        .row_bands()
        .iter()
        .filter_map(|band| {
            row_window_for(*band, offset, height)
                .map(|(first, count)| (band.key(), RowWindow::new(first, count)))
        })
        .collect();
    PendingRowWindows { bands }
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

/// 任意文本输入是否仍在组合；焦点移动不会结束组合。
fn any_composition(snapshot: &PresentationSnapshot) -> bool {
    snapshot.elements().any(|element| {
        element.tag() == "text-input"
            && element
                .edit()
                .is_some_and(|edit| edit.composition().is_some())
    })
}

/// 快照里所有仍在组合的文本输入。
fn composing_nodes(snapshot: &PresentationSnapshot) -> Vec<NodeId> {
    snapshot
        .elements()
        .filter(|element| element.tag() == "text-input")
        .filter(|element| {
            element
                .edit()
                .is_some_and(|edit| edit.composition().is_some())
        })
        .map(SnapshotElement::id)
        .collect()
}

/// 隐藏候选窗的锚点：远离所有显示器工作区。
const HIDDEN_IME_POINT: POINT = POINT {
    x: -32_000,
    y: -32_000,
};

/// 组合窗与候选窗的客户区物理锚点；`None` 表示组合起点 caret 已完全离开视口。
///
/// 锚点取 caret 左上角并 clamp 进客户区：部分越界的 caret 仍把候选窗留在窗口内，
/// 完全离开视口时调用方隐藏候选窗，而不是沿用上一次场景的锚点（ADR-0010）。
fn ime_candidate_point(viewport: Viewport, caret: Rect) -> Option<POINT> {
    if caret.right() <= 0.0
        || caret.bottom() <= 0.0
        || caret.x >= viewport.width()
        || caret.y >= viewport.height()
    {
        return None;
    }
    let scale = viewport.scale();
    Some(POINT {
        x: (caret.x * scale)
            .round()
            .clamp(0.0, viewport.width() * scale) as i32,
        y: (caret.y * scale)
            .round()
            .clamp(0.0, viewport.height() * scale) as i32,
    })
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

/// 宿主虚拟键到 runtime `Key` 的映射；未建模的键交给用户 handler 作为
/// `Key::Other`，宿主自身不再为它们提供默认动作。
const fn host_key(key: VIRTUAL_KEY) -> Key {
    match key.0 {
        k if k == VK_ESCAPE.0 => Key::Escape,
        k if k == VK_RETURN.0 => Key::Enter,
        k if k == VK_TAB.0 => Key::Tab,
        k if k == VK_SPACE.0 => Key::Space,
        k if k == VK_BACK.0 => Key::Backspace,
        k if k == VK_DELETE.0 => Key::Delete,
        k if k == VK_LEFT.0 => Key::ArrowLeft,
        k if k == VK_RIGHT.0 => Key::ArrowRight,
        k if k == VK_UP.0 => Key::ArrowUp,
        k if k == VK_DOWN.0 => Key::ArrowDown,
        k if k == VK_HOME.0 => Key::Home,
        k if k == VK_END.0 => Key::End,
        k if k == VK_PRIOR.0 => Key::PageUp,
        k if k == VK_NEXT.0 => Key::PageDown,
        _ => Key::Other,
    }
}

/// runtime 结算后，宿主是否仍应执行自己的原生默认动作（滚动等）。
fn key_default_blocked(report: &TurnReport) -> bool {
    !report.is_ok() || report.outcome().consumed() || report.outcome().prevent_default()
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

fn palette_for(
    appearance: Appearance,
    preferences: SystemPreferences,
    colors: impl FnOnce() -> HighContrastColors,
) -> RenderPalette {
    if preferences.high_contrast() {
        return RenderPalette::high_contrast(colors());
    }
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
            for hwnd in (*host).routing_hwnds(None) {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, host as isize);
            }
        }
        Self { host }
    }
}

impl Drop for HostPointer {
    fn drop(&mut self) {
        // 安全：循环已返回，重新通过裸指针访问宿主。
        unsafe {
            for hwnd in (*self.host).routing_hwnds(None) {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            (*self.host).dispatching = None;
        }
    }
}

/// 取回窗口过程对应的宿主裸指针；未安装时返回 `None`。
///
/// # Safety
///
/// 指针由 `HostPointer` 安装在 `run`/`pump` 期间，指向不会移动的 `Host`；
/// 通过它访问宿主时必须保证同一时刻没有其它引用。
unsafe fn host_pointer(hwnd: HWND) -> Option<*mut Host> {
    // 安全：读取窗口的 user data 槽；未安装时为零。
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Host;
    (!pointer.is_null()).then_some(pointer)
}

/// 在窗口过程里取回宿主并执行一次动作；未安装指针时什么也不做。
///
/// # Safety
///
/// 动作执行期间不得通过其他引用访问宿主。`run`/`pump` 满足这一点：它们在
/// 派发消息时只持有裸指针，派发返回后才重新借出宿主。
unsafe fn with_host(hwnd: HWND, action: impl FnOnce(&mut Host)) {
    // 安全：指针有效；调用方不在动作执行期间访问宿主。
    if let Some(pointer) = unsafe { host_pointer(hwnd) } {
        let host = unsafe { &mut *pointer };
        action(host);
    }
}

/// 取走窗口过程的延迟捕获释放请求；调用时不得持有宿主引用。
///
/// # Safety
///
/// 调用方必须在窗口过程借出宿主结束后调用，期间不得有其它引用指向宿主。
unsafe fn take_capture_release(hwnd: HWND) -> bool {
    // 安全：指针有效；调用方不持有宿主引用。
    match unsafe { host_pointer(hwnd) } {
        Some(pointer) => unsafe { (&mut *pointer).take_release_capture(hwnd) },
        None => false,
    }
}

/// `WM_CONTEXTMENU` 的屏幕坐标；键盘调用（-1,-1）放在客户区中心。
fn context_menu_point(hwnd: HWND, lparam: LPARAM) -> (i32, i32) {
    let x = (lparam.0 & 0xFFFF) as u16 as i16;
    let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16;
    if (x, y) != (-1, -1) {
        return (i32::from(x), i32::from(y));
    }
    let mut rect = RECT::default();
    // 安全：hwnd 由宿主持有；rect 是本地输出缓冲。
    if unsafe { GetClientRect(hwnd, &mut rect) }.is_err() {
        return (0, 0);
    }
    let mut point = POINT {
        x: (rect.right - rect.left) / 2,
        y: (rect.bottom - rect.top) / 2,
    };
    // 安全：同一 HWND 的客户区坐标转屏幕坐标；失败时退回原点。
    if !unsafe { ClientToScreen(hwnd, &mut point) }.as_bool() {
        return (0, 0);
    }
    (point.x, point.y)
}

/// 托盘回调的屏幕坐标；通知区域版本 4 把鼠标位置放进 `wParam`。
fn tray_menu_point(wparam: WPARAM) -> (i32, i32) {
    let x = (wparam.0 & 0xFFFF) as u16 as i16;
    let y = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16;
    if (x, y) != (0, 0) {
        return (i32::from(x), i32::from(y));
    }
    let mut point = POINT::default();
    // 安全：point 是本地输出缓冲。
    if unsafe { GetCursorPos(&mut point) }.is_ok() {
        return (point.x, point.y);
    }
    (0, 0)
}

/// 执行一次托盘 presenter 调用；panic 转为宿主错误，不穿过消息循环或 Drop。
fn tray_presenter_call<T>(action: impl FnOnce() -> Result<T, HostError>) -> Result<T, HostError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(action)).unwrap_or_else(|_| {
        Err(HostError::TrayPresenter(String::from(
            "the presenter panicked",
        )))
    })
}

/// 托盘回调：左键/键盘恢复窗口，右键在宿主借用之外展示声明菜单。
///
/// # Safety
///
/// 调用时不持有宿主引用；presenter 的菜单循环可以重入窗口过程。
unsafe fn handle_tray_callback(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    // 安全：指针由 HostPointer 安装在重入安全的派发期间。
    let Some(pointer) = (unsafe { host_pointer(hwnd) }) else {
        return;
    };
    if !unsafe { &*pointer }.is_tray_window(hwnd) {
        return;
    }
    // 版本 4：事件在 `lParam` 低字，高字是图标 id；鼠标位置在 `wParam`。
    match (lparam.0 & 0xFFFF) as u32 {
        NIN_SELECT | NIN_KEYSELECT => {
            // 安全：回调期间不持有宿主引用；恢复只更新运行时目录。
            let host = unsafe { &mut *pointer };
            if let Err(error) = host.restore_from_tray() {
                host.deferred_error.get_or_insert(error);
            }
        }
        WM_CONTEXTMENU => {
            let point = tray_menu_point(wparam);
            // 安全：presenter 与菜单在展示期间离开宿主。
            let work = unsafe { (&mut *pointer).begin_tray_menu() };
            let Some((mut presenter, menu, target)) = work else {
                return;
            };
            // 安全：菜单打开期间不持有宿主引用，presenter 可以重入窗口过程。
            let result = tray_presenter_call(|| {
                presenter.show_menu(TrayMenuRequest::new(hwnd, point, &menu))
            });
            // 安全：菜单已关闭且没有其它引用；重新借出宿主结算这次打开。
            let host = unsafe { &mut *pointer };
            let mut error = host.finish_tray_menu(presenter, target, result).err();
            if let Err(failure) = host.settle_requests() {
                error.get_or_insert(failure);
            }
            if let Err(failure) = host.repaint_changed_windows() {
                error.get_or_insert(failure);
            }
            if let Some(failure) = error {
                host.deferred_error.get_or_insert(failure);
            }
        }
        _ => {}
    }
}

/// 翻译并派发一条消息；快捷键先于字符翻译与窗口过程。
///
/// # Safety
///
/// `host` 指向在消息循环期间不会移动的 `Host`；调用时不持有它的引用。
unsafe fn dispatch_message(host: *mut Host, message: &MSG) {
    if unsafe { translate_accelerator(host, message) } {
        return;
    }
    // 安全：消息来自本线程队列，窗口由 Host 持有。
    unsafe {
        let _ = TranslateMessage(message);
        DispatchMessageW(message);
    }
}

/// 把一条键盘消息交给窗口的 `HACCEL`；匹配时 `TranslateAcceleratorW` 已经同步
/// 派发过 `WM_COMMAND`，调用方不得再翻译或派发这条消息。
///
/// 组合中的聚焦输入与编辑保留键先于快捷键：这些键继续走正常输入路径。
///
/// # Safety
///
/// `host` 指向在消息循环期间不会移动的 `Host`；调用时不持有它的引用。
unsafe fn translate_accelerator(host: *mut Host, message: &MSG) -> bool {
    let Some((haccel, shadowed)) = unsafe { &*host }.accelerator_for(message) else {
        return false;
    };
    if shadowed {
        return false;
    }
    // 安全：haccel 属于这条消息的目标窗口；TranslateAcceleratorW 同步派发
    // WM_COMMAND，窗口过程可以安全地重入宿主。
    unsafe { TranslateAcceleratorW(message.hwnd, haccel, message) != 0 }
}

/// 在顶层消息回合之外服务文件选择请求。
///
/// 一次只弹一个对话框，调用 presenter 期间不持有任何 `&mut Host`；shell 的
/// 模态循环可以正常把消息派发回窗口过程。每个结果随后走任务交付、窗口同步
/// 与重绘，continuation 新排队的请求在同一循环里继续处理。
///
/// # Safety
///
/// `host` 指向在循环期间不会移动的 `Host`；调用方不持有它的引用。
unsafe fn service_file_dialogs(host: *mut Host) -> Result<bool, HostError> {
    let mut serviced = false;
    loop {
        let requests = unsafe { (&mut *host).runtime.take_file_dialog_requests() };
        if requests.is_empty() {
            return Ok(serviced);
        }
        serviced = true;
        for request in requests {
            let work = unsafe { (&mut *host).file_dialog_work(&request) };
            let (outcome, presenter) = match work {
                Ok(work) => {
                    let FileDialogWork {
                        mut presenter,
                        restore,
                        hwnd,
                        options,
                    } = work;
                    let outcome =
                        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            presenter.present(hwnd, &options)
                        })) {
                            Ok(outcome) => outcome,
                            Err(_) => FileDialogOutcome::Failed(FileDialogError::DialogFailed(
                                String::from("the file dialog presenter panicked"),
                            )),
                        };
                    (outcome, restore.then_some(presenter))
                }
                Err(error) => (FileDialogOutcome::Failed(error), None),
            };
            unsafe { (&mut *host).finish_file_dialog(request, outcome, presenter)? };
        }
    }
}

/// 派发一条消息后的统一结算；返回宿主会话是否已结束。
///
/// 同步窗口、恢复前台焦点、重绘变化窗口、取出延迟错误、结算文件对话框；
/// `run_loop` 与 `pump_loop` 共用这一条路径。
///
/// # Safety
///
/// `host` 指向在循环期间不会移动的 `Host`；派发已返回，不持有它的引用。
unsafe fn after_dispatch(host: *mut Host) -> Result<bool, HostError> {
    {
        // 安全：派发已返回，重新借出宿主同步窗口并读取处理结果。
        let host = unsafe { &mut *host };
        host.sync_windows()?;
        if std::mem::take(&mut host.focus_after_sync) {
            host.focus_native_focused_window()?;
        }
        host.repaint_changed_windows()?;
        if let Some(error) = host.deferred_error.take() {
            return Err(error);
        }
        if !host.session_running() {
            return Ok(true);
        }
    }
    unsafe { service_file_dialogs(host)? };
    Ok(!unsafe { &*host }.session_running())
}

/// 没有消息可派发时的结算；返回宿主会话是否已结束。
///
/// # Safety
///
/// 与 [`run_loop`] 相同。
unsafe fn settle_idle(host: *mut Host) -> Result<bool, HostError> {
    if !unsafe { &*host }.session_running() {
        // 确保关闭时排队的请求不会留下 Running 任务。
        let _ = unsafe { &mut *host }.runtime.take_file_dialog_requests();
        return Ok(true);
    }
    unsafe { service_file_dialogs(host)? };
    Ok(!unsafe { &*host }.session_running())
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
        unsafe { dispatch_message(host, &message) };
        if unsafe { after_dispatch(host)? } {
            return Ok(());
        }
    }
}

/// `pump` 的消息循环；返回宿主会话是否已结束。
///
/// # Safety
///
/// 与 [`run_loop`] 相同。
unsafe fn pump_loop(host: *mut Host) -> Result<bool, HostError> {
    loop {
        if unsafe { settle_idle(host)? } {
            return Ok(true);
        }
        let mut message = MSG::default();
        // 安全：只取本线程队列里的消息；窗口由 Host 持有。
        let result = unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) };
        if result.0 == 0 {
            return Ok(false);
        }
        unsafe { dispatch_message(host, &message) };
        if unsafe { after_dispatch(host)? } {
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
    if message == host_message::TRAY_CALLBACK {
        // 托盘菜单自己的模态循环在窗口过程内运行；展示期间不持有宿主引用，
        // 与上下文菜单走同一条重入安全路径。
        unsafe { handle_tray_callback(hwnd, wparam, lparam) };
        // shell 可能用 SendMessage 投递托盘回调，外层消息循环因此收不到一次
        // 可结算的消息。排队的空消息保证恢复/退出后的窗口同步与重绘发生。
        // 安全：hwnd 是宿主自己的托盘窗口，消息只排队。
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        }
        return LRESULT(0);
    }
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
        WM_SYSCOMMAND if (wparam.0 & 0xFFF0) == SC_CLOSE as usize => {
            // Alt+F4 与系统菜单都走 SC_CLOSE；Alt 仍按下时归类为 Alt+F4。
            // 安全：GetKeyState 只查询本线程最近一条输入消息的修饰键状态。
            let source = if unsafe { GetKeyState(i32::from(VK_MENU.0)) } < 0 {
                CloseSource::AltF4
            } else {
                CloseSource::SystemMenu
            };
            unsafe { with_host(hwnd, |host| host.close_from_window(hwnd, source)) };
            LRESULT(0)
        }
        WM_CLOSE => {
            // 安全：派发期间不持有宿主引用；结束该窗口的 activation。
            // HWND 由消息循环在派发返回后同步销毁，最后一个窗口关闭时循环退出。
            unsafe {
                with_host(hwnd, |host| {
                    host.close_from_window(hwnd, CloseSource::Programmatic);
                })
            };
            LRESULT(0)
        }
        WM_CONTEXTMENU => {
            // 菜单自己的模态循环在窗口过程内运行，不会再回到消息循环。presenter
            // 打开菜单期间不持有宿主借用，因此到达的唤醒和其它窗口消息照常重入。
            let Some(pointer) = (unsafe { host_pointer(hwnd) }) else {
                return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
            };
            // 安全：派发期间不持有宿主引用，窗口过程借出一次。
            let work = unsafe { (&mut *pointer).begin_context_menu(hwnd, lparam) };
            if let Some((mut presenter, request)) = work {
                let window = request.window();
                // 安全：菜单打开期间不持有宿主引用，presenter 可以重入窗口过程。
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    presenter.show(request)
                }))
                .unwrap_or_else(|_| {
                    Err(HostError::ContextMenuPresenter(String::from(
                        "the presenter panicked",
                    )))
                });
                // 安全：菜单已关闭且没有其它引用；重新借出宿主结算这次打开。
                let host = unsafe { &mut *pointer };
                let mut error = host.finish_context_menu(presenter, window, result).err();
                if let Err(failure) = host.settle_requests() {
                    error.get_or_insert(failure);
                }
                if let Err(failure) = host.repaint_changed_windows() {
                    error.get_or_insert(failure);
                }
                if let Some(failure) = error {
                    host.deferred_error.get_or_insert(failure);
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // 销毁由宿主在同步阶段发起；循环以“没有存活窗口”判断退出，
            // 这里不投递 WM_QUIT，避免关闭一个附属窗口时结束其他窗口的消息循环。
            LRESULT(0)
        }
        WM_GETOBJECT => {
            // UIA 把根对象标识放在 lParam；其它对象标识仍由默认过程决定。
            // Provider 在宿主借用结束后才交回 UIA，回调期间不再持有 Host 引用。
            if lparam.0 as i32 != UiaRootObjectId {
                return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
            }
            let mut provider = None;
            // 安全：派发期间不持有宿主引用，窗口过程借出一次。
            unsafe {
                with_host(hwnd, |host| provider = host.accessibility_provider(hwnd));
            }
            match provider {
                // 安全：窗口仍由宿主持有；provider 的生命周期转交 UIA。
                Some(provider) => unsafe {
                    UiaReturnRawElementProvider(hwnd, wparam, lparam, &provider)
                },
                None => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
            }
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
            // 菜单栏属于非客户区；直接查询 HWND 的菜单，避免在 DPI 计算里借出宿主。
            // 安全：hwnd 来自系统回调。
            let has_menu = !unsafe { GetMenu(hwnd) }.is_invalid();
            // 安全：outer 是本地值；样式与窗口创建时一致。
            if unsafe {
                AdjustWindowRectExForDpi(
                    &mut outer,
                    WS_OVERLAPPEDWINDOW,
                    has_menu,
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
    #![allow(unused_braces)]

    use super::*;
    use crate::accessibility::ControlType;
    use crate::clipboard::ClipboardError;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::System::Com::SAFEARRAY;
    use windows::Win32::System::Ole::{
        SafeArrayDestroy, SafeArrayGetElement, SafeArrayGetLBound, SafeArrayGetUBound,
    };
    use windows::Win32::System::Variant::VARIANT;
    use windows::Win32::UI::Accessibility::{
        IInvokeProvider, IRawElementProviderFragment, IScrollProvider, ITextProvider,
        ITextRangeProvider, IToggleProvider, IValueProvider, NavigateDirection_FirstChild,
        NavigateDirection_NextSibling, ProviderOptions_ServerSideProvider, ScrollAmount,
        ScrollAmount_LargeDecrement, ScrollAmount_LargeIncrement, ScrollAmount_NoAmount,
        ScrollAmount_SmallDecrement, ScrollAmount_SmallIncrement, SupportedTextSelection_Single,
        TextPatternRangeEndpoint_End, TextPatternRangeEndpoint_Start, TextUnit_Character,
        ToggleState_Off, ToggleState_On, UIA_E_ELEMENTNOTAVAILABLE, UIA_E_INVALIDOPERATION,
        UIA_E_NOTSUPPORTED, UIA_FontNameAttributeId, UIA_InvokePatternId, UIA_NamePropertyId,
        UIA_ScrollItemPatternId, UIA_ScrollPatternId, UIA_ScrollPatternNoScroll, UIA_TextPatternId,
        UIA_TogglePatternId, UIA_ValuePatternId, UiaAppendRuntimeId, UiaPoint,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, IsWindowEnabled, SetKeyboardState, VK_A, VK_C, VK_MENU, VK_NEXT, VK_S,
        VK_V, VK_X, VK_Y, VK_Z,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CopyAcceleratorTableW, FindWindowW, GetMenuItemCount, GetMenuItemID, GetMenuState,
        GetMenuStringW, GetSubMenu, HMENU, IsMenu, MENU_ITEM_FLAGS, MF_BYCOMMAND, MF_BYPOSITION,
        MF_CHECKED, MF_GRAYED, SendMessageW,
    };
    use windows::core::{BSTR, HRESULT, Interface, w};
    use zenui_render::{DecorationKind, FontAsset};

    const MAIN: &str = "main";

    fn wheel(lines: WheelLines) -> WheelScroll {
        WheelScroll {
            lines,
            accumulator: 0,
            anchor: 0,
        }
    }

    #[test]
    fn wheel_sub_notches_accumulate_without_drift() {
        // 高分辨率滚轮：每个事件 24 单位（1/5 notch），3 行 × 24 px = 72 px/notch。
        // 逐事件截断会得到 5 × 14 = 70 px；累积器必须得到 14,14,15,14,15 = 72。
        let mut forward = wheel(WheelLines::Lines(3));
        let steps: Vec<i32> = (0..5).map(|_| forward.step(24, 100)).collect();
        assert_eq!(steps, [14, 14, 15, 14, 15], "sub-notch residue is carried");
        assert_eq!(steps.iter().sum::<i32>(), 72, "five fifths make one notch");

        let mut backward = wheel(WheelLines::Lines(3));
        let steps: Vec<i32> = (0..5).map(|_| backward.step(-24, 100)).collect();
        assert_eq!(steps, [-14, -14, -15, -14, -15]);
        assert_eq!(steps.iter().sum::<i32>(), -72);

        let mut whole = wheel(WheelLines::Lines(3));
        assert_eq!(whole.step(120, 100), 72, "one notch is three lines");
        assert_eq!(whole.step(-120, 100), -72);
    }

    #[test]
    fn wheel_page_and_disabled_settings_map_to_fixed_steps() {
        assert_eq!(wheel_lines_from(0), WheelLines::Disabled);
        assert_eq!(wheel_lines_from(WHEEL_PAGESCROLL), WheelLines::Page);
        assert_eq!(wheel_lines_from(1), WheelLines::Lines(1));
        assert_eq!(wheel_lines_from(3), WheelLines::Lines(3));

        let mut page = wheel(WheelLines::Page);
        assert_eq!(page.step(120, 500), 500);
        assert_eq!(page.step(-120, 500), -500);

        let mut disabled = wheel(WheelLines::Disabled);
        assert_eq!(disabled.step(120, 500), 0);
        let mut zero_lines = wheel(WheelLines::Lines(0));
        assert_eq!(zero_lines.step(120, 500), 0);
    }

    #[test]
    fn wheel_line_changes_discard_unapplied_residue() {
        let mut wheel = wheel(WheelLines::Lines(3));
        // 1 单位在 3 行设置下只有 0.6 px，留下 residue。
        assert_eq!(wheel.step(1, 100), 0);
        wheel.set_lines(WheelLines::Lines(2));
        assert_eq!(
            wheel.step(1, 100),
            0,
            "the pre-change residue cannot add a phantom pixel"
        );
        assert_eq!(wheel.step(120, 100), 48, "the new line count applies");
    }

    #[test]
    fn wheel_extreme_deltas_stay_representable() {
        let mut wheel = wheel(WheelLines::Lines(1));
        let up = wheel.step(i32::MAX, 100);
        assert!(up > 0);
        let down = wheel.step(i32::MIN, 100);
        assert!(down < 0);
    }

    #[test]
    fn uia_scroll_percent_rounds_to_the_nearest_pixel_and_clamps() {
        assert_eq!(percent_offset(0.0, 101), 0);
        assert_eq!(percent_offset(50.0, 101), 51);
        assert_eq!(percent_offset(100.0, 101), 101);
        assert_eq!(
            scroll_target(0, 100, ScrollCommand::Percent(50.0), 40, 72),
            Some(50)
        );
        assert_eq!(
            scroll_target(100, 100, ScrollCommand::Percent(100.0), 40, 72),
            None,
            "the maximum is a no-op"
        );
        assert_eq!(
            scroll_target(0, 100, ScrollCommand::Percent(-1.0), 40, 72),
            None,
            "NoScroll is not a domain percent"
        );
        assert_eq!(
            scroll_target(0, 100, ScrollCommand::Percent(f64::NAN), 40, 72),
            None
        );
    }

    #[test]
    fn uia_scroll_amounts_use_page_and_line_steps_and_clamp() {
        assert_eq!(
            scroll_target(0, 100, ScrollCommand::PageDown, 40, 72),
            Some(40)
        );
        assert_eq!(
            scroll_target(80, 100, ScrollCommand::PageDown, 40, 72),
            Some(100),
            "large increments clamp at the bottom"
        );
        assert_eq!(
            scroll_target(0, 100, ScrollCommand::PageUp, 40, 72),
            None,
            "page up at the top is a no-op"
        );
        assert_eq!(
            scroll_target(0, 100, ScrollCommand::LineDown, 40, 72),
            Some(72)
        );
        assert_eq!(
            scroll_target(72, 100, ScrollCommand::LineDown, 40, 72),
            Some(100)
        );
        assert_eq!(
            scroll_target(100, 100, ScrollCommand::LineDown, 40, 72),
            None
        );
        assert_eq!(
            scroll_target(0, 100, ScrollCommand::LineDown, 40, 0),
            None,
            "a disabled wheel line setting maps to no line movement"
        );
    }

    #[test]
    fn reveal_offsets_are_minimal_vertical_moves() {
        // Above the viewport: add the (floored) top.
        assert_eq!(
            reveal_offset(700.0, Rect::new(0.0, -50.0, 100.0, 20.0), 100, 1000),
            Some(50)
        );
        // Below: add the ceiling of the overflow.
        assert_eq!(
            reveal_offset(700.0, Rect::new(0.0, 690.0, 100.0, 20.0), 100, 1000),
            Some(110)
        );
        // Fully visible: no movement.
        assert_eq!(
            reveal_offset(700.0, Rect::new(0.0, 10.0, 100.0, 20.0), 100, 1000),
            None
        );
        // Taller than the viewport: the top edge wins.
        assert_eq!(
            reveal_offset(700.0, Rect::new(0.0, -50.0, 100.0, 900.0), 100, 1000),
            Some(50)
        );
        // Clamped at both ends.
        assert_eq!(
            reveal_offset(700.0, Rect::new(0.0, -50.0, 100.0, 20.0), 10, 1000),
            Some(0)
        );
        assert_eq!(
            reveal_offset(700.0, Rect::new(0.0, -50.0, 100.0, 20.0), 0, 1000),
            None
        );
        assert_eq!(
            reveal_offset(700.0, Rect::new(0.0, 690.0, 100.0, 20.0), 1000, 1000),
            None
        );
        // Fractional geometry rounds away from the viewport edge.
        assert_eq!(
            reveal_offset(700.0, Rect::new(0.0, -49.5, 100.0, 20.0), 100, 1000),
            Some(50)
        );
        assert_eq!(
            reveal_offset(700.0, Rect::new(0.0, 699.5, 100.0, 1.0), 100, 1000),
            Some(101)
        );
    }

    #[test]
    fn ime_anchors_follow_the_translated_caret_and_clamp_into_the_client_area() {
        let viewport = Viewport::new(900.0, 700.0, 1.5).expect("viewport");
        // Scene caret to client pixels at 150%.
        assert_eq!(
            ime_candidate_point(viewport, Rect::new(96.0, 62.0, 1.0, 19.0)),
            Some(POINT { x: 144, y: 93 })
        );
        // A partially clipped caret stays inside the client area.
        assert_eq!(
            ime_candidate_point(viewport, Rect::new(-0.5, -10.0, 1.0, 19.0)),
            Some(POINT { x: 0, y: 0 })
        );
        assert_eq!(
            ime_candidate_point(viewport, Rect::new(899.5, 695.0, 1.0, 19.0)),
            Some(POINT { x: 1349, y: 1043 })
        );
    }

    #[test]
    fn a_caret_that_leaves_the_viewport_hides_the_candidate() {
        let viewport = Viewport::new(900.0, 700.0, 1.5).expect("viewport");
        // Fully above, below, left and right; the half-open edge itself is outside.
        assert_eq!(
            ime_candidate_point(viewport, Rect::new(96.0, -20.0, 1.0, 19.0)),
            None
        );
        assert_eq!(
            ime_candidate_point(viewport, Rect::new(96.0, 700.0, 1.0, 19.0)),
            None
        );
        assert_eq!(
            ime_candidate_point(viewport, Rect::new(-1.0, 20.0, 1.0, 19.0)),
            None
        );
        assert_eq!(
            ime_candidate_point(viewport, Rect::new(900.0, 20.0, 1.0, 19.0)),
            None
        );
    }

    #[test]
    fn scrollbar_drag_offsets_follow_the_pointer_and_clamp() {
        let track = Rect::new(888.0, 4.0, 8.0, 692.0);
        let thumb = Rect::new(888.0, 4.0, 8.0, 100.0);
        // 抓在 thumb 顶部下方 10 像素，指针落在轨道行程的中点。
        let travel = track.height - thumb.height;
        let middle = track.y + travel / 2.0 + 10.0;
        assert_eq!(
            scrollbar_drag_offset(track, thumb, 10.0, middle, 1000),
            Some(500)
        );
        assert_eq!(
            scrollbar_drag_offset(track, thumb, 10.0, -1000.0, 1000),
            Some(0)
        );
        assert_eq!(
            scrollbar_drag_offset(track, thumb, 10.0, 10_000.0, 1000),
            Some(1000)
        );
        // 轨道高度等于 thumb 时没有行程；不可滚动的场景没有可拖的偏移。
        // 两者都返回 None，调用方保持现有偏移不动。
        assert_eq!(scrollbar_drag_offset(track, track, 0.0, 320.0, 1000), None);
        assert_eq!(scrollbar_drag_offset(track, thumb, 0.0, 320.0, 0), None);
    }

    #[test]
    fn host_messages_are_distinct_and_reserved_in_the_app_range() {
        use windows::Win32::UI::WindowsAndMessaging::WM_APP;

        let mut sorted = host_message::ALL;
        sorted.sort_unstable();
        assert!(
            sorted.windows(2).all(|pair| pair[0] < pair[1]),
            "every registered message has its own value"
        );
        assert!(
            host_message::ALL
                .iter()
                .all(|message| (WM_APP..WM_APP + 16).contains(message)),
            "the host registry stays inside the private message range"
        );
        assert_eq!(host_message::WAKE, WM_APP + 1);
        assert_ne!(host_message::TRAY_CALLBACK, host_message::WAKE);
    }

    #[derive(Default)]
    struct TrayCalls {
        adds: Vec<(HWND, HICON, u32)>,
        updates: Vec<(HWND, String)>,
        removes: Vec<HWND>,
        menus: Vec<(HWND, (i32, i32), MenuSpec)>,
    }

    /// 记录通知区域调用并返回固定菜单命令的 fake。
    struct FakeTrayPresenter {
        calls: Rc<RefCell<TrayCalls>>,
        result: Option<&'static str>,
        panic_first: Rc<Cell<bool>>,
    }

    impl TrayPresenter for FakeTrayPresenter {
        fn add(&mut self, request: TrayIconRequest) -> Result<(), HostError> {
            self.calls.borrow_mut().adds.push((
                request.hwnd(),
                request.icon(),
                request.callback_message(),
            ));
            Ok(())
        }

        fn update(&mut self, request: TrayTooltipRequest<'_>) -> Result<(), HostError> {
            self.calls
                .borrow_mut()
                .updates
                .push((request.hwnd(), request.tooltip().to_owned()));
            Ok(())
        }

        fn remove(&mut self, hwnd: HWND) -> Result<(), HostError> {
            self.calls.borrow_mut().removes.push(hwnd);
            Ok(())
        }

        fn show_menu(
            &mut self,
            request: TrayMenuRequest<'_>,
        ) -> Result<Option<&'static str>, HostError> {
            if self.panic_first.replace(false) {
                panic!("the fake tray presenter failed");
            }
            self.calls.borrow_mut().menus.push((
                request.hwnd(),
                request.point(),
                request.menu().clone(),
            ));
            Ok(self.result)
        }
    }

    type TrayRuns = Rc<RefCell<Vec<(CommandSource, Option<WindowCtx>, Option<NodeId>)>>>;

    struct TrayHarness {
        host: Host,
        calls: Rc<RefCell<TrayCalls>>,
        runs: TrayRuns,
    }

    fn tray_declaration() -> TraySpec {
        TraySpec::new("ZenUI tray").with_menu(
            MenuSpec::new()
                .command("app.one", "One")
                .separator()
                .command("app.two", "Two"),
        )
    }

    fn tray_host(result: Option<&'static str>, with_presenter: bool) -> TrayHarness {
        let runs: TrayRuns = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = Runtime::new();
        for id in ["app.one", "app.two"] {
            let runs = runs.clone();
            runtime
                .on_app_command(id, move |event| {
                    runs.borrow_mut()
                        .push((event.source(), event.window(), event.target()));
                })
                .expect("register the tray app command");
        }
        runtime
            .declare_tray(tray_declaration())
            .expect("declare the tray");
        runtime
            .declare_windows(|catalog| {
                catalog.window(WindowSpec::new(MAIN).primary(), |_| {
                    view! {
                        <view>
                            <text aria:label="status">"Ready"</text>
                        </view>
                    }
                })?;
                Ok(())
            })
            .expect("declare the tray window");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open the tray host");
        let calls = Rc::new(RefCell::new(TrayCalls::default()));
        if with_presenter {
            host.set_tray_presenter(FakeTrayPresenter {
                calls: calls.clone(),
                result,
                panic_first: Rc::new(Cell::new(false)),
            })
            .expect("install the fake tray presenter");
        }
        TrayHarness { host, calls, runs }
    }

    fn tray_hwnd(host: &Host) -> HWND {
        host.tray.as_ref().expect("a declared tray").window.hwnd
    }

    /// 同步派发托盘回调，并在返回后完成宿主的同步/重绘结算。
    fn send_tray_callback(host: &mut Host, event: u32, point: (i16, i16)) -> Option<HostError> {
        let hwnd = tray_hwnd(host);
        let (x, y) = point;
        // 版本 4：鼠标位置在 wParam，事件和图标 id 在 lParam。
        let wparam = WPARAM((x as u16 as usize) | ((y as u16 as usize) << 16));
        let lparam = LPARAM(((1u32 << 16) | event) as isize);
        {
            let _pointer = HostPointer::install(host as *mut Host);
            // 安全：SendMessageW 在调用线程同步派发；调用期间不使用 host。
            unsafe {
                SendMessageW(
                    hwnd,
                    host_message::TRAY_CALLBACK,
                    Some(wparam),
                    Some(lparam),
                );
            }
        }
        host.sync_windows().expect("sync windows");
        if std::mem::take(&mut host.focus_after_sync) {
            host.focus_native_focused_window()
                .expect("focus the active window");
        }
        host.repaint_changed_windows().expect("repaint");
        host.deferred_error.take()
    }

    #[test]
    fn the_fake_tray_presenter_records_one_add_update_menu_and_delete() {
        let TrayHarness {
            mut host,
            calls,
            runs,
        } = tray_host(Some("app.one"), true);
        let hwnd = tray_hwnd(&host);
        let main = host.windows[MAIN].ctx;
        {
            let calls = calls.borrow();
            assert_eq!(calls.adds.len(), 1, "installation adds one icon");
            assert_eq!(calls.adds[0].0, hwnd);
            assert!(!calls.adds[0].1.is_invalid(), "the shared system icon");
            assert_eq!(
                calls.adds[0].2,
                host_message::TRAY_CALLBACK,
                "the callback message is the tray value, not worker wake"
            );
            assert_eq!(calls.updates.len(), 1, "one tooltip update follows the add");
            assert_eq!(
                calls.updates[0],
                (hwnd, String::from("ZenUI tray")),
                "the declared tooltip reaches the presenter"
            );
            assert!(calls.menus.is_empty());
            assert!(calls.removes.is_empty());
        }

        assert!(send_tray_callback(&mut host, WM_CONTEXTMENU, (30, 40)).is_none());
        {
            let calls = calls.borrow();
            assert_eq!(calls.menus.len(), 1, "one right-click opens one menu");
            assert_eq!(calls.menus[0].0, hwnd);
            assert_eq!(calls.menus[0].1, (30, 40));
            assert_eq!(calls.menus[0].2, tray_declaration().menu().clone());
        }
        assert_eq!(
            runs.borrow().len(),
            1,
            "the selected command runs exactly once"
        );
        let (source, window, target) = runs.borrow()[0];
        assert_eq!(source, CommandSource::Tray);
        assert_eq!(window, Some(main), "the menu exposes the preferred window");
        assert_eq!(target, None);

        drop(host);
        let calls = calls.borrow();
        assert_eq!(calls.removes, [hwnd], "Host drop removes the icon once");
    }

    #[test]
    fn a_hidden_tray_without_a_presenter_does_not_add_an_icon() {
        let TrayHarness { host, calls, .. } = tray_host(None, false);
        assert!(host.tray_presenter.is_none());
        assert!(
            !host.tray.as_ref().expect("declared tray").added,
            "no presenter means no presenter-side icon state"
        );
        drop(host);
        assert!(
            calls.borrow().adds.is_empty(),
            "hidden mode makes no shell call without a presenter"
        );
    }

    #[test]
    fn the_tray_callback_routes_only_through_the_tray_hwnd() {
        let TrayHarness {
            mut host,
            calls,
            runs,
        } = tray_host(Some("app.one"), true);
        let main_hwnd = host.hwnd(MAIN).expect("the main window");
        {
            let _pointer = HostPointer::install(&mut host as *mut Host);
            // 安全：同步派发到普通窗口；托盘分支必须拒绝非托盘 HWND。
            unsafe {
                SendMessageW(
                    main_hwnd,
                    host_message::TRAY_CALLBACK,
                    Some(WPARAM(0)),
                    Some(LPARAM(WM_CONTEXTMENU as isize)),
                );
            }
        }
        assert!(
            calls.borrow().menus.is_empty(),
            "a window HWND is not the tray"
        );
        assert!(runs.borrow().is_empty());

        assert!(send_tray_callback(&mut host, WM_CONTEXTMENU, (5, 6)).is_none());
        assert_eq!(calls.borrow().menus.len(), 1);
        assert_eq!(runs.borrow().len(), 1);
    }

    #[test]
    fn clicking_the_tray_restores_the_primary_window() {
        let TrayHarness { mut host, runs, .. } = tray_host(Some("app.one"), true);
        let main = host.windows[MAIN].ctx;
        host.hide(MAIN).expect("hide the only window");
        assert_eq!(
            host.runtime.focused_window(),
            None,
            "a hidden window is not a focused tray target"
        );

        assert!(send_tray_callback(&mut host, NIN_SELECT, (0, 0)).is_none());
        assert_eq!(
            host.runtime.focused_window().map(|window| window.key()),
            Some(MAIN),
            "a left click restores and focuses the primary window"
        );
        assert!(host.runtime.window_alive(main));
        assert!(runs.borrow().is_empty(), "restoring is not a command");
    }

    #[test]
    fn taskbar_created_re_adds_and_the_last_close_removes_once() {
        let TrayHarness {
            mut host,
            calls,
            runs,
        } = tray_host(None, true);
        let hwnd = tray_hwnd(&host);
        let taskbar_created = host.taskbar_created;
        assert_ne!(
            taskbar_created, 0,
            "Explorer's broadcast message is registered"
        );

        // Explorer 向所有顶层窗口广播；只有托盘 HWND 负责重新添加。
        let main_hwnd = host.hwnd(MAIN).expect("the main window");
        {
            let _pointer = HostPointer::install(&mut host as *mut Host);
            // 安全：同步派发注册消息；调用期间不使用 host。
            unsafe {
                SendMessageW(hwnd, taskbar_created, Some(WPARAM(0)), Some(LPARAM(0)));
                SendMessageW(main_hwnd, taskbar_created, Some(WPARAM(0)), Some(LPARAM(0)));
            }
        }
        {
            let calls = calls.borrow();
            assert_eq!(
                calls.adds.len(),
                2,
                "the tray HWND re-adds and a normal window does not add a second icon"
            );
            assert_eq!(calls.updates.len(), 2, "the tooltip follows every add");
        }
        assert!(runs.borrow().is_empty());

        host.close(MAIN).expect("close the last window");
        assert_eq!(
            calls.borrow().removes,
            [hwnd],
            "the last close removes the icon"
        );
        drop(host);
        assert_eq!(
            calls.borrow().removes.len(),
            1,
            "Host drop does not delete the icon a second time"
        );
    }

    struct SessionTrayHarness {
        host: Host,
        calls: Rc<RefCell<TrayCalls>>,
        runs: Rc<Cell<u32>>,
    }

    /// 常驻会话 + 托盘的隐藏宿主；菜单选中 app.show 或 app.quit。
    fn session_tray_host(result: Option<&'static str>) -> SessionTrayHarness {
        let runs = Rc::new(Cell::new(0));
        let show_runs = runs.clone();
        let quit_runs = runs.clone();
        let mut runtime = Runtime::new();
        let session = runtime.session();
        runtime
            .on_app_command("app.show", move |_| {
                show_runs.set(show_runs.get() + 1);
                session.request_show(MAIN).expect("queue the show");
            })
            .expect("register the show command");
        runtime
            .on_app_command("app.quit", move |_| {
                quit_runs.set(quit_runs.get() + 1);
                session.request_quit().expect("queue the quit");
            })
            .expect("register the quit command");
        runtime
            .declare_session(SessionPolicy::StayResident)
            .expect("declare the resident session");
        runtime
            .declare_tray(
                TraySpec::new("ZenUI Tray").with_menu(
                    MenuSpec::new()
                        .command("app.show", "Show window")
                        .command("app.quit", "Quit"),
                ),
            )
            .expect("declare the tray");
        runtime
            .declare_windows(|catalog| {
                catalog.window(WindowSpec::new(MAIN).primary(), |_| {
                    view! {
                        <view>
                            <text aria:label="status">"Ready"</text>
                        </view>
                    }
                })?;
                Ok(())
            })
            .expect("declare the window");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open the resident host");
        let calls = Rc::new(RefCell::new(TrayCalls::default()));
        host.set_tray_presenter(FakeTrayPresenter {
            calls: calls.clone(),
            result,
            panic_first: Rc::new(Cell::new(false)),
        })
        .expect("install the fake tray presenter");
        SessionTrayHarness { host, calls, runs }
    }

    #[test]
    fn a_resident_session_keeps_the_tray_and_click_reopens_the_last_window() {
        let SessionTrayHarness {
            mut host,
            calls,
            runs,
        } = session_tray_host(None);
        let hwnd = tray_hwnd(&host);
        let first = host.windows[MAIN].ctx;
        host.close(MAIN).expect("close the last window");
        assert!(host.runtime.windows().is_empty());
        assert!(host.hwnd(MAIN).is_none());
        assert_eq!(
            calls.borrow().removes,
            [],
            "a resident session keeps the icon after the last close"
        );

        assert!(send_tray_callback(&mut host, NIN_SELECT, (0, 0)).is_none());
        assert_eq!(
            host.runtime.windows().len(),
            1,
            "a tray click reopens the primary window"
        );
        assert!(host.hwnd(MAIN).is_some());
        assert_ne!(
            host.windows[MAIN].ctx, first,
            "reopening is a new activation"
        );
        assert_eq!(
            calls.borrow().adds.len(),
            1,
            "the kept icon is not re-added"
        );
        assert_eq!(calls.borrow().removes, []);
        assert_eq!(runs.get(), 0, "reopening is not a command");
        drop(host);
        assert_eq!(calls.borrow().removes, [hwnd], "drop removes the kept icon");
    }

    #[test]
    fn a_resident_session_show_command_reopens_zero_windows() {
        let SessionTrayHarness {
            mut host,
            calls,
            runs,
        } = session_tray_host(Some("app.show"));
        host.close(MAIN).expect("close the last window");
        assert!(host.runtime.windows().is_empty());

        assert!(send_tray_callback(&mut host, WM_CONTEXTMENU, (10, 20)).is_none());
        assert_eq!(runs.get(), 1);
        assert_eq!(host.runtime.windows().len(), 1);
        assert!(host.hwnd(MAIN).is_some());
        assert_eq!(calls.borrow().adds.len(), 1, "the resident icon stays");
        assert_eq!(calls.borrow().removes, []);
    }

    #[test]
    fn a_tray_quit_ends_the_resident_session_and_removes_the_icon_once() {
        let SessionTrayHarness {
            mut host,
            calls,
            runs,
        } = session_tray_host(Some("app.quit"));
        let hwnd = tray_hwnd(&host);
        assert!(!host.runtime.windows().is_empty());

        assert!(send_tray_callback(&mut host, WM_CONTEXTMENU, (10, 20)).is_none());
        assert_eq!(runs.get(), 1);
        assert!(
            host.runtime.windows().is_empty(),
            "quit closes every window"
        );
        assert!(!host.runtime.session_alive());
        assert_eq!(calls.borrow().removes, [hwnd], "the icon is removed once");
        assert!(host.pump().expect("settle the ended session"));
        drop(host);
        assert_eq!(
            calls.borrow().removes.len(),
            1,
            "drop does not delete the icon twice"
        );
    }

    #[test]
    fn a_resident_session_requires_a_declared_tray() {
        let mut runtime = Runtime::new();
        runtime
            .declare_session(SessionPolicy::StayResident)
            .expect("declare the resident policy");
        runtime
            .declare_windows(|catalog| {
                catalog.window(WindowSpec::new(MAIN).primary(), |_| view! { <view /> })?;
                Ok(())
            })
            .expect("declare the window");
        let error = match Host::open(runtime, HostConfig::hidden()) {
            Ok(_) => panic!("a resident host without a tray must be rejected"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            HostError::Session(SessionError::TrayRequired)
        ));
    }

    #[test]
    fn reopening_after_the_last_close_adds_the_icon_again() {
        let TrayHarness {
            mut host,
            calls,
            runs,
        } = tray_host(None, true);
        let hwnd = tray_hwnd(&host);
        host.close(MAIN).expect("close the last window");
        assert_eq!(calls.borrow().removes, [hwnd]);
        host.show(MAIN).expect("reopen the catalog window");
        {
            let calls = calls.borrow();
            assert_eq!(calls.adds.len(), 2, "a live window adds the icon again");
            assert_eq!(calls.updates.len(), 2, "the tooltip follows the new add");
        }
        drop(host);
        assert_eq!(calls.borrow().removes.len(), 2, "drop removes the new icon");
        assert!(runs.borrow().is_empty(), "lifecycle changes run no command");
    }

    #[test]
    fn the_tray_uses_the_shared_system_icon() {
        let TrayHarness { host, calls, .. } = tray_host(None, true);
        let icon = calls.borrow().adds[0].1;
        drop(host);
        // 安全：只查询系统共享图标的句柄；宿主从不 DestroyIcon。
        let again = unsafe { LoadIconW(None, IDI_APPLICATION) }.expect("reload the system icon");
        assert_eq!(
            again, icon,
            "the shared system icon survives the host instead of being destroyed"
        );
    }

    #[test]
    fn a_panicking_tray_presenter_is_contained_and_restored() {
        let TrayHarness {
            mut host,
            calls,
            runs,
        } = tray_host(Some("app.one"), true);
        let presenter = host.tray_presenter.take().expect("installed presenter");
        // 重新安装一个 panic-first 的 fake，保留同一个记录器。
        host.set_tray_presenter(FakeTrayPresenter {
            calls: calls.clone(),
            result: Some("app.one"),
            panic_first: Rc::new(Cell::new(true)),
        })
        .expect("replace the fake");
        drop(presenter);

        let error =
            send_tray_callback(&mut host, WM_CONTEXTMENU, (1, 2)).expect("the panic is reported");
        assert!(matches!(error, HostError::TrayPresenter(_)));
        assert!(host.tray_presenter.is_some(), "the presenter is restored");
        assert!(runs.borrow().is_empty(), "a panicking menu runs no command");

        assert!(send_tray_callback(&mut host, WM_CONTEXTMENU, (3, 4)).is_none());
        assert_eq!(
            runs.borrow().len(),
            1,
            "the next open runs the command once"
        );
    }

    #[test]
    fn a_tray_menu_cannot_return_an_undeclared_command() {
        let TrayHarness { mut host, runs, .. } = tray_host(Some("app.undeclared"), true);
        let error = send_tray_callback(&mut host, WM_CONTEXTMENU, (1, 2))
            .expect("the undeclared id is a host error");
        assert!(matches!(
            error,
            HostError::Command(CommandError::MissingHandler("app.undeclared"))
        ));
        assert!(runs.borrow().is_empty());
    }

    #[test]
    fn a_tray_declaration_requires_application_handlers_before_the_host_opens() {
        let mut runtime = Runtime::new();
        runtime
            .declare_tray(
                TraySpec::new("ZenUI tray")
                    .with_menu(MenuSpec::new().command("app.missing", "Missing")),
            )
            .expect("declare the tray");
        runtime
            .declare_windows(|catalog| {
                catalog.window(WindowSpec::new(MAIN).primary(), |_| view! { <view /> })?;
                Ok(())
            })
            .expect("declare the window");
        let error = match Host::open(runtime, HostConfig::hidden()) {
            Ok(_) => panic!("an unhandled tray command must fail the open"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("app.missing"),
            "the host names the missing tray command: {error}"
        );
    }

    #[test]
    fn ole_failures_map_to_the_file_dialog_contract() {
        assert_eq!(
            file_dialog_ole_error(OleError::ChangedMode),
            FileDialogError::Unsupported
        );
        assert_eq!(
            file_dialog_ole_error(OleError::Failed(String::from("boom"))),
            FileDialogError::DialogFailed(String::from("boom"))
        );
    }

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

    /// 已提交场景的粗略签名：元素几何与字形数量。
    fn scene_signature(host: &Host) -> Vec<(f32, f32, f32, f32, usize)> {
        host.scene(MAIN)
            .expect("scene")
            .elements()
            .iter()
            .map(|element| {
                let rect = element.rect();
                (
                    rect.x,
                    rect.y,
                    rect.width,
                    rect.height,
                    element.text().iter().map(|run| run.glyphs().len()).sum(),
                )
            })
            .collect()
    }

    fn assert_skipped_present_keeps_damage(status: wgpu::CurrentSurfaceTexture) {
        let mut host = open_host();
        host.runtime
            .focus("What needs to be done?")
            .expect("focus the draft");
        host.render().expect("present the initial frame");
        let before = scene_signature(&host);
        let presented = host.windows.get(MAIN).expect("window").snapshot_revision;

        host.runtime
            .input("What needs to be done?", "A")
            .expect("change the snapshot");
        assert_ne!(
            host.runtime.presentation_revision(),
            presented,
            "the runtime snapshot changed"
        );

        host.forced_surface_statuses.push_back(status);
        host.render().expect("a skipped present is not an error");

        let window = host.windows.get(MAIN).expect("window");
        assert_eq!(
            window.snapshot_revision, presented,
            "the skipped render does not advance the cache revision"
        );
        assert!(window.paint_requested, "the skipped frame stays pending");
        assert_eq!(
            scene_signature(&host),
            before,
            "the presented scene stays on screen"
        );

        host.repaint_changed_windows()
            .expect("the pending paint presents the new frame");
        let window = host.windows.get(MAIN).expect("window");
        assert_eq!(
            window.snapshot_revision,
            host.runtime.presentation_revision(),
            "a successful present advances the revision"
        );
        assert!(!window.paint_requested, "a present clears the damage");
        assert_ne!(scene_signature(&host), before, "the new frame is presented");
    }

    #[test]
    fn a_timeout_present_keeps_the_scene_and_paint_pending() {
        assert_skipped_present_keeps_damage(wgpu::CurrentSurfaceTexture::Timeout);
    }

    #[test]
    fn an_occluded_present_keeps_the_scene_and_paint_pending() {
        assert_skipped_present_keeps_damage(wgpu::CurrentSurfaceTexture::Occluded);
    }

    #[test]
    fn a_lost_present_rebuilds_and_presents_the_pending_frame() {
        let mut host = open_host();
        host.runtime
            .focus("What needs to be done?")
            .expect("focus the draft");
        host.render().expect("present the initial frame");
        let before = scene_signature(&host);
        let generation = host.device_generation;

        host.runtime
            .input("What needs to be done?", "A")
            .expect("change the snapshot");
        host.forced_surface_statuses
            .push_back(wgpu::CurrentSurfaceTexture::Lost);
        host.render()
            .expect("the lost surface recovers and presents");

        assert_eq!(host.device_generation, generation + 1);
        assert!(!host.windows.get(MAIN).expect("window").paint_requested);
        assert_ne!(
            scene_signature(&host),
            before,
            "the rebuild presents the pending frame"
        );
    }

    #[test]
    fn a_zero_size_window_keeps_paint_pending() {
        let mut host = open_host();
        host.runtime
            .focus("What needs to be done?")
            .expect("focus the draft");
        host.render().expect("present the initial frame");
        let presented = host.windows.get(MAIN).expect("window").snapshot_revision;

        host.runtime
            .input("What needs to be done?", "A")
            .expect("change the snapshot");
        host.resize(MAIN, 0, 0).expect("zero resize");

        let window = host.windows.get(MAIN).expect("window");
        assert_eq!(
            window.snapshot_revision, presented,
            "no frame is presented at zero size"
        );
        assert!(window.paint_requested, "the zero-size frame stays pending");

        host.resize(MAIN, 320, 240).expect("restore the size");
        let window = host.windows.get(MAIN).expect("window");
        assert_eq!(
            window.snapshot_revision,
            host.runtime.presentation_revision(),
            "the restored window presents the pending frame"
        );
        assert!(
            !window.paint_requested,
            "the restore present clears the damage"
        );
    }

    #[test]
    fn a_readback_does_not_consume_the_skipped_damage() {
        let mut host = open_host();
        host.runtime
            .focus("What needs to be done?")
            .expect("focus the draft");
        host.render().expect("present the initial frame");
        let presented = host.windows.get(MAIN).expect("window").snapshot_revision;
        host.runtime
            .input("What needs to be done?", "A")
            .expect("change the snapshot");

        host.forced_surface_statuses
            .push_back(wgpu::CurrentSurfaceTexture::Timeout);
        host.render().expect("a skipped present is not an error");
        assert_eq!(
            host.windows.get(MAIN).expect("window").snapshot_revision,
            presented
        );
        assert!(host.windows.get(MAIN).expect("window").paint_requested);

        host.read_frame(MAIN)
            .expect("read the pending frame offscreen");
        let window = host.windows.get(MAIN).expect("window");
        assert!(
            window.paint_requested,
            "an offscreen readback does not consume the damage"
        );
        assert_eq!(
            window.snapshot_revision,
            host.runtime.presentation_revision(),
            "the readback syncs the build cache"
        );

        host.repaint_changed_windows()
            .expect("the pending paint presents after the readback");
        assert!(!host.windows.get(MAIN).expect("window").paint_requested);
    }

    #[test]
    fn a_transient_paint_survives_a_skipped_present() {
        let mut host = open_host();
        host.render().expect("present the initial frame");
        let (center, scale) = {
            let scene = host.scene(MAIN).expect("scene");
            let viewport = scene.viewport();
            (
                Point::new(viewport.width() / 2.0, viewport.height() / 2.0),
                viewport.scale(),
            )
        };
        let x = (center.x * scale) as i32;
        let y = (center.y * scale) as i32;
        let lparam = LPARAM((x as isize & 0xffff) | ((y as isize & 0xffff) << 16));

        // 消息触发的重绘正好遇到遮挡：hover 请求保留到下一次绘制。
        host.forced_surface_statuses
            .push_back(wgpu::CurrentSurfaceTexture::Occluded);
        // `send_msg` 会跑两遍重绘：窗口过程内一次，返回后再一次。
        host.forced_surface_statuses
            .push_back(wgpu::CurrentSurfaceTexture::Occluded);
        send_msg(
            &mut host,
            MSG {
                message: WM_MOUSEMOVE,
                lParam: lparam,
                ..Default::default()
            },
        );
        assert!(
            host.windows.get(MAIN).expect("window").paint_requested,
            "the skipped hover stays pending"
        );
        assert!(
            !host
                .scene(MAIN)
                .expect("scene")
                .elements()
                .iter()
                .any(|element| element.hovered()),
            "the hover is not committed before a present"
        );

        host.repaint_changed_windows()
            .expect("the transient paint presents on the next pass");
        assert!(!host.windows.get(MAIN).expect("window").paint_requested);
        assert!(
            host.scene(MAIN)
                .expect("scene")
                .elements()
                .iter()
                .any(|element| element.hovered()),
            "the hover visual is presented"
        );
    }

    #[test]
    fn accessibility_tree_maps_roles_names_and_states() {
        let mut host = open_host();
        host.render().expect("render");

        let tree = host.accessibility_tree(MAIN).expect("published tree");
        assert_eq!(tree.root().control_type(), ControlType::Window);
        assert_eq!(tree.root().name(), "ZenUI Todo");
        assert!(tree.root().node_id().is_none());

        let form = tree.find_name("Add task").expect("the add form");
        assert_eq!(form.control_type(), ControlType::Group);
        let input = tree
            .find_name("What needs to be done?")
            .expect("the draft input");
        assert_eq!(input.control_type(), ControlType::Edit);
        assert!(input.focusable(), "a mounted text input is focusable");

        let toolbar = tree
            .iter()
            .find(|node| node.control_type() == ControlType::ToolBar)
            .expect("a toolbar");
        assert!(
            tree.children(toolbar)
                .any(|node| node.name() == "Reverse" && node.control_type() == ControlType::Button),
            "toolbar children expose their text as names"
        );

        assert_eq!(
            tree.find_name("All").and_then(|node| node.toggle_state()),
            Some(true),
            "aria:pressed maps to the toggle state"
        );
        assert_eq!(
            tree.find_name("Active")
                .and_then(|node| node.toggle_state()),
            Some(false)
        );
        let checkbox = tree
            .find_name("Complete Write tests")
            .expect("the completed task checkbox");
        assert_eq!(checkbox.control_type(), ControlType::CheckBox);
        assert_eq!(checkbox.toggle_state(), Some(true));
        assert!(tree.find_name("Delete Write tests").is_some());
        assert_eq!(
            tree.find_name("Write tests")
                .map(|node| node.control_type()),
            Some(ControlType::Text),
            "text elements keep their own accessible node"
        );
        assert!(tree.find_name("Nothing here yet").is_none());

        let bounds = tree.bounds(form, (0, 0));
        let logical = tree.logical_rect(form);
        assert!((bounds.width - f64::from(logical.width) * f64::from(tree.scale())).abs() < 0.01);
    }

    #[test]
    fn scrolling_updates_accessibility_bounds_and_keeps_runtime_ids() {
        let mut host = open_host();
        host.resize(MAIN, 320, 100).expect("shrink the window");
        host.render().expect("render");
        let max = host.scene(MAIN).expect("scene").max_scroll_offset();
        assert!(
            max >= 100,
            "the demo content is taller than the small viewport: {max}"
        );

        let before = host.accessibility_tree(MAIN).expect("published tree");
        let form = before.find_name("Add task").expect("the add form");
        let node = form.node_id().expect("a node id");
        let runtime_id = form.runtime_id().expect("a runtime id");
        let top = before.logical_rect(form).y;

        host.windows
            .get_mut(MAIN)
            .expect("a live window")
            .scroll_offset = 100;
        host.render().expect("render after scrolling");

        let after = host.accessibility_tree(MAIN).expect("republished tree");
        let moved = after.node(node).expect("the form stays mounted");
        assert_eq!(
            moved.runtime_id(),
            Some(runtime_id),
            "runtime ids are stable"
        );
        assert!(
            (after.logical_rect(moved).y - (top - 100.0)).abs() < 0.5,
            "bounds follow the applied scroll offset"
        );
        assert!(moved.offscreen(), "the scrolled form leaves the viewport");
    }

    #[test]
    fn scroll_pattern_reports_vertical_geometry_and_horizontal_no_scroll() {
        let mut host = open_host();
        host.resize(MAIN, 320, 100).expect("shrink the window");
        host.render().expect("render the small window");
        let scene = host.scene(MAIN).expect("scene");
        let max = scene.max_scroll_offset();
        let viewport = scene.viewport().height();
        let content = scene.content_extent().height;
        assert!(max >= 100, "the demo content is taller than the viewport");

        let tree = host.accessibility_tree(MAIN).expect("published tree");
        let geometry = tree.scroll_geometry();
        assert_eq!(geometry.max_offset(), max);
        assert!(geometry.vertically_scrollable());
        assert_close(
            geometry.vertical_view_size(),
            f64::from(viewport) / f64::from(content) * 100.0,
        );
        assert_eq!(geometry.vertical_scroll_percent(), Some(0.0));

        let scroll = scroll_pattern(&mut host);
        assert!(
            unsafe { scroll.VerticallyScrollable() }
                .expect("vertical")
                .as_bool()
        );
        assert!(
            !unsafe { scroll.HorizontallyScrollable() }
                .expect("horizontal")
                .as_bool()
        );
        assert_eq!(
            unsafe { scroll.HorizontalScrollPercent() }.expect("horizontal percent"),
            UIA_ScrollPatternNoScroll
        );
        assert_eq!(
            unsafe { scroll.HorizontalViewSize() }.expect("horizontal view size"),
            UIA_ScrollPatternNoScroll
        );
        assert_close(
            unsafe { scroll.VerticalViewSize() }.expect("vertical view size"),
            f64::from(viewport) / f64::from(content) * 100.0,
        );
        assert_eq!(
            unsafe { scroll.VerticalScrollPercent() }.expect("vertical percent"),
            0.0
        );
    }

    #[test]
    fn scroll_pattern_set_percent_rounds_and_clamps() {
        let mut host = open_host();
        host.resize(MAIN, 320, 100).expect("shrink the window");
        host.render().expect("render the small window");
        let max = host.scene(MAIN).expect("scene").max_scroll_offset();
        assert!(max >= 2, "the percent mapping needs a real range");
        let scroll = scroll_pattern(&mut host);

        unsafe { scroll.SetScrollPercent(UIA_ScrollPatternNoScroll, 50.0) }.expect("queue half");
        host.pump().expect("drain the half scroll");
        assert_eq!(
            host.scene(MAIN).expect("scene").scroll_offset(),
            percent_offset(50.0, max)
        );
        let percent = unsafe { scroll.VerticalScrollPercent() }.expect("vertical percent");
        assert_close(
            percent,
            f64::from(percent_offset(50.0, max)) / f64::from(max) * 100.0,
        );

        unsafe { scroll.SetScrollPercent(UIA_ScrollPatternNoScroll, 100.0) }.expect("queue end");
        host.pump().expect("drain the end scroll");
        assert_eq!(host.scene(MAIN).expect("scene").scroll_offset(), max);
        let end = unsafe { scroll.VerticalScrollPercent() }.expect("vertical percent");
        assert_close(end, 100.0);
        assert!(
            host.accessibility_tree(MAIN)
                .expect("tree")
                .find_name("Add task")
                .expect("the add form")
                .offscreen(),
            "UIA bounds/IsOffscreen follow the translated scene at the end offset"
        );

        let before = host.scene(MAIN).expect("scene").scroll_offset();
        let error = unsafe { scroll.SetScrollPercent(UIA_ScrollPatternNoScroll, 100.1) }
            .expect_err("out-of-range percent");
        assert_invalid_operation(error);
        let error = unsafe { scroll.SetScrollPercent(0.0, 50.0) }.expect_err("horizontal percent");
        assert_invalid_operation(error);
        let error = unsafe { scroll.SetScrollPercent(UIA_ScrollPatternNoScroll, f64::NAN) }
            .expect_err("NaN percent");
        assert_invalid_operation(error);
        assert_eq!(
            host.scene(MAIN).expect("scene").scroll_offset(),
            before,
            "rejected percents leave the offset untouched"
        );

        unsafe { scroll.SetScrollPercent(UIA_ScrollPatternNoScroll, UIA_ScrollPatternNoScroll) }
            .expect("NoScroll is a no-op");
        host.pump().expect("drain the no-op");
        assert_eq!(host.scene(MAIN).expect("scene").scroll_offset(), max);
    }

    #[test]
    fn scroll_pattern_revalidates_a_queued_percent_against_the_current_scene() {
        let mut host = open_host();
        host.resize(MAIN, 320, 100).expect("shrink the window");
        let first_max = host.scene(MAIN).expect("scene").max_scroll_offset();
        assert!(first_max > 0);
        let scroll = scroll_pattern(&mut host);
        unsafe { scroll.SetScrollPercent(UIA_ScrollPatternNoScroll, 100.0) }
            .expect("queue the end scroll");

        host.resize(MAIN, 320, 160).expect("grow the window");
        let second_max = host.scene(MAIN).expect("scene").max_scroll_offset();
        assert!(
            second_max > 0 && second_max < first_max,
            "the scene changed before the queued action drained"
        );
        host.pump().expect("drain the queued scroll");
        assert_eq!(
            host.scene(MAIN).expect("scene").scroll_offset(),
            second_max,
            "the UI thread remaps the percent against the current scene"
        );
    }

    #[test]
    fn scroll_pattern_amounts_match_page_and_line_steps_and_clamp() {
        let mut host = open_host();
        host.resize(MAIN, 320, 100).expect("shrink the window");
        host.render().expect("render the small window");
        host.windows.get_mut(MAIN).expect("the window").wheel = wheel(WheelLines::Lines(3));
        let scene = host.scene(MAIN).expect("scene");
        let max = scene.max_scroll_offset();
        let page = page_step(Some(scene));
        assert!(max >= page + 72, "the test needs room for both steps");
        let scroll = scroll_pattern(&mut host);

        unsafe { scroll.Scroll(ScrollAmount_NoAmount, ScrollAmount_LargeIncrement) }
            .expect("page down");
        host.pump().expect("drain page down");
        assert_eq!(host.scene(MAIN).expect("scene").scroll_offset(), page);

        unsafe { scroll.Scroll(ScrollAmount_NoAmount, ScrollAmount_SmallIncrement) }
            .expect("line down");
        host.pump().expect("drain line down");
        assert_eq!(
            host.scene(MAIN).expect("scene").scroll_offset(),
            page + 72,
            "the small increment uses the wheel line step"
        );

        unsafe { scroll.Scroll(ScrollAmount_NoAmount, ScrollAmount_SmallDecrement) }
            .expect("line up");
        host.pump().expect("drain line up");
        assert_eq!(
            host.scene(MAIN).expect("scene").scroll_offset(),
            page,
            "the small decrement uses the wheel line step"
        );

        unsafe { scroll.Scroll(ScrollAmount_NoAmount, ScrollAmount_LargeDecrement) }
            .expect("page up");
        host.pump().expect("drain page up");
        assert_eq!(host.scene(MAIN).expect("scene").scroll_offset(), 0);

        let before = host.scene(MAIN).expect("scene").scroll_offset();
        unsafe { scroll.Scroll(ScrollAmount_NoAmount, ScrollAmount_NoAmount) }.expect("no amount");
        host.pump().expect("drain no amount");
        assert_eq!(host.scene(MAIN).expect("scene").scroll_offset(), before);

        let error = unsafe { scroll.Scroll(ScrollAmount_LargeIncrement, ScrollAmount_NoAmount) }
            .expect_err("horizontal scrolling is not exposed");
        assert_invalid_operation(error);
        let error = unsafe { scroll.Scroll(ScrollAmount_NoAmount, ScrollAmount(99)) }
            .expect_err("unknown vertical amount");
        assert_invalid_operation(error);

        unsafe { scroll.SetScrollPercent(UIA_ScrollPatternNoScroll, 100.0) }.expect("go to end");
        host.pump().expect("drain end scroll");
        unsafe { scroll.Scroll(ScrollAmount_NoAmount, ScrollAmount_LargeIncrement) }
            .expect("page down at the end");
        host.pump().expect("drain clamped page down");
        assert_eq!(
            host.scene(MAIN).expect("scene").scroll_offset(),
            max,
            "large increments never overscroll"
        );
    }

    #[test]
    fn scroll_pattern_changes_only_the_presentation_offset() {
        let (mut host, _probe) = open_uia_probe();
        host.resize(MAIN, 320, 100).expect("shrink the probe");
        host.render().expect("render the probe");
        assert!(
            host.scene(MAIN).expect("scene").max_scroll_offset() > 0,
            "the probe has scrollable content"
        );
        let node = host
            .runtime
            .presentation()
            .expect("snapshot")
            .find_label("Field")
            .expect("the field")
            .id();
        host.runtime.focus_node(node).expect("focus the field");
        host.runtime
            .set_selection_node(node, 1, 2)
            .expect("place the selection");
        host.render().expect("render the focused field");
        with_main(&mut host, |host, window| {
            host.ime_start(window).expect("start the composition");
            host.apply_ime_composition(window, node, request(false, true), None, Some("ni".into()))
                .expect("preedit");
            window.reveal_requested = false;
            Ok(())
        });
        host.render().expect("render the composition");
        host.pump().expect("settle the composition");
        host.render().expect("render the settled composition");

        let before_snapshot = host.presentation(MAIN).expect("snapshot").clone();
        let builds = host.runtime.presentation_builds();
        let counters = host.runtime.counters();
        let revision = host.runtime.presentation_revision();
        let before_tree = host.accessibility_tree(MAIN).expect("tree");
        let before_node = before_tree.node(node).expect("field node");
        let runtime_id = before_node.runtime_id().expect("runtime id");
        let top = before_tree.logical_rect(before_node).y;
        let scroll = scroll_pattern(&mut host);

        unsafe { scroll.SetScrollPercent(UIA_ScrollPatternNoScroll, 25.0) }
            .expect("queue a small scroll");
        host.pump().expect("drain the scroll");

        assert_eq!(
            host.runtime.presentation_builds(),
            builds,
            "UIA scrolling reuses the committed snapshot"
        );
        assert_eq!(
            host.runtime.counters(),
            counters,
            "UIA scrolling leaves runtime counters flat"
        );
        assert_eq!(
            host.runtime.presentation_revision(),
            revision,
            "UIA scrolling does not advance the presentation revision"
        );
        assert_eq!(
            host.presentation(MAIN).expect("snapshot"),
            before_snapshot,
            "focus, selection, draft and composition are unchanged"
        );
        assert!(
            !host.windows[MAIN].reveal_requested,
            "UIA scrolling does not request reveal"
        );
        let offset = host.scene(MAIN).expect("scene").scroll_offset();
        assert!(offset > 0, "the scroll was applied");
        let after_tree = host.accessibility_tree(MAIN).expect("tree");
        let after_node = after_tree.node(node).expect("field node stays mounted");
        assert_eq!(after_node.runtime_id(), Some(runtime_id));
        assert!(
            (after_tree.logical_rect(after_node).y - (top - offset as f32)).abs() < 0.5,
            "UIA bounds follow the applied offset"
        );
    }

    #[test]
    fn scroll_pattern_rejects_zero_maximum_hidden_and_disabled_targets() {
        let mut zero = open_two_input_host();
        zero.render().expect("render the short window");
        assert_eq!(zero.scene(MAIN).expect("scene").max_scroll_offset(), 0);
        let scroll = scroll_pattern(&mut zero);
        let error = unsafe { scroll.SetScrollPercent(UIA_ScrollPatternNoScroll, 50.0) }
            .expect_err("zero maximum");
        assert_invalid_operation(error);
        let error = unsafe { scroll.Scroll(ScrollAmount_NoAmount, ScrollAmount_LargeIncrement) }
            .expect_err("zero maximum");
        assert_invalid_operation(error);

        let mut hidden = open_host();
        hidden.resize(MAIN, 320, 100).expect("shrink the window");
        hidden.render().expect("render the small window");
        let scroll = scroll_pattern(&mut hidden);
        hidden.hide(MAIN).expect("hide the window");
        let error = unsafe { scroll.VerticallyScrollable() }.expect_err("hidden tree");
        assert_eq!(error.code(), HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32));
        let error = unsafe { scroll.SetScrollPercent(UIA_ScrollPatternNoScroll, 50.0) }
            .expect_err("hidden window");
        assert_eq!(error.code(), HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32));

        let mut disabled = open_host();
        disabled.resize(MAIN, 320, 100).expect("shrink the window");
        disabled.render().expect("render the small window");
        let scroll = scroll_pattern(&mut disabled);
        disabled
            .set_native_enabled(MAIN, false)
            .expect("disable the window");
        disabled.render().expect("render the disabled window");
        let error = unsafe { scroll.SetScrollPercent(UIA_ScrollPatternNoScroll, 50.0) }
            .expect_err("disabled window");
        assert_eq!(error.code(), HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32));

        let mut closing = open_host();
        closing.resize(MAIN, 320, 100).expect("shrink the window");
        closing.render().expect("render the small window");
        let builds = closing.runtime.presentation_builds();
        let counters = closing.runtime.counters();
        let revision = closing.runtime.presentation_revision();
        with_main(&mut closing, |host, window| {
            window.close_requested = true;
            host.apply_accessibility_scroll(window, ScrollCommand::Percent(50.0));
            Ok(())
        });
        assert_eq!(closing.scene(MAIN).expect("scene").scroll_offset(), 0);
        assert_eq!(closing.runtime.presentation_builds(), builds);
        assert_eq!(closing.runtime.counters(), counters);
        assert_eq!(closing.runtime.presentation_revision(), revision);
    }

    #[test]
    fn scroll_pattern_is_only_exposed_on_the_window_root() {
        let mut host = open_host();
        host.render().expect("render");
        let root = root_fragment(&mut host);
        let simple: IRawElementProviderSimple = root.cast().expect("simple root");
        let _: IScrollProvider = pattern(&simple, UIA_ScrollPatternId);
        let node = provider_named(&root, "What needs to be done?");
        assert!(
            unsafe { node.GetPatternProvider(UIA_ScrollPatternId) }.is_err(),
            "nodes do not expose ScrollPattern"
        );
        assert!(
            unsafe { simple.GetPatternProvider(UIA_ScrollItemPatternId) }.is_err(),
            "ScrollItemPattern is deferred"
        );
        assert!(
            unsafe { node.GetPatternProvider(UIA_ScrollItemPatternId) }.is_err(),
            "ScrollItemPattern is deferred for nodes too"
        );
    }

    #[test]
    fn spacer_views_are_transparent_for_accessibility() {
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(|catalog| {
                catalog.window(WindowSpec::new(MAIN).primary(), |_| {
                    view! {
                        <view role="spacer">
                            <button id="inner-button" aria:label="Inner">"Go"</button>
                        </view>
                    }
                })?;
                Ok(())
            })
            .expect("declare the window");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open the host");
        host.render().expect("render");

        let tree = host.accessibility_tree(MAIN).expect("published tree");
        assert!(
            tree.iter().all(|node| node.tag() != "view"),
            "spacer layout views do not become accessibility nodes"
        );
        let inner = tree.find_name("Inner").expect("the inner button");
        assert_eq!(inner.control_type(), ControlType::Button);
        assert_eq!(inner.automation_id(), "inner-button");
        assert!(
            tree.children(tree.root())
                .any(|node| node.node_id() == inner.node_id()),
            "the inner button is re-parented to the root"
        );
    }

    #[test]
    fn details_is_an_independent_accessibility_root() {
        let mut host = open_host();
        host.render().expect("render");
        host.show(zenui_demo::DETAILS_WINDOW)
            .expect("show the details window");
        host.render().expect("render details");

        let details = host
            .accessibility_tree(zenui_demo::DETAILS_WINDOW)
            .expect("a details tree");
        assert_eq!(details.root().name(), "ZenUI Details");
        assert!(details.find_name("Details rows").is_some());
        assert_eq!(
            host.accessibility_tree(MAIN)
                .expect("a main tree")
                .root()
                .name(),
            "ZenUI Todo",
            "each window publishes its own root"
        );
    }

    #[test]
    fn the_root_provider_reports_identity_and_rejects_stale_publications() {
        let mut host = open_host();
        host.render().expect("render");
        let hwnd = host.hwnd(MAIN).expect("a live window");
        let provider = host.accessibility_provider(hwnd).expect("a root provider");

        let options = unsafe { provider.ProviderOptions() }.expect("provider options");
        assert_eq!(options, ProviderOptions_ServerSideProvider);
        let name = unsafe { provider.GetPropertyValue(UIA_NamePropertyId) }.expect("name");
        // 安全：provider 返回的 VT_BSTR 由这个 VARIANT 持有。
        let name = unsafe { &name.Anonymous.Anonymous.Anonymous.bstrVal }.to_string();
        assert_eq!(name, "ZenUI Todo");

        let fragment: IRawElementProviderFragment = provider.cast().expect("fragment");
        let child = unsafe { fragment.Navigate(NavigateDirection_FirstChild) }
            .expect("the root has children");
        let runtime = unsafe { child.GetRuntimeId() }.expect("a runtime id");
        let mut first = 0i32;
        // 安全：runtime 是 UIA 返回的两元素 SAFEARRAY。
        unsafe {
            SafeArrayGetElement(runtime, &0, (&mut first as *mut i32).cast()).expect("element");
            let _ = SafeArrayDestroy(runtime);
        }
        assert_eq!(first, UiaAppendRuntimeId as i32);

        host.hide(MAIN).expect("hide");
        let error = unsafe { provider.GetPropertyValue(UIA_NamePropertyId) }
            .expect_err("a hidden window has no published tree");
        assert_eq!(error.code(), HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32));
    }

    #[test]
    fn invoke_and_toggle_patterns_enqueue_one_command_each_in_order() {
        let (mut host, probe) = open_uia_probe();
        let root = root_fragment(&mut host);
        let act = provider_named(&root, "Act");
        let flag = provider_named(&root, "Flag");
        let caption = provider_named(&root, "Caption");
        let invoke: IInvokeProvider = pattern(&act, UIA_InvokePatternId);
        let toggle: IToggleProvider = pattern(&flag, UIA_TogglePatternId);
        assert!(
            unsafe { caption.GetPatternProvider(UIA_InvokePatternId) }.is_err(),
            "pattern presence comes from mounted actions, not the tag"
        );
        assert!(
            unsafe { act.GetPatternProvider(UIA_TogglePatternId) }.is_err(),
            "a button without a change handler has no toggle pattern"
        );
        assert_eq!(
            unsafe { toggle.ToggleState() }.expect("initial state"),
            ToggleState_Off
        );

        unsafe { invoke.Invoke() }.expect("queue invoke");
        unsafe { toggle.Toggle() }.expect("queue toggle");
        assert_eq!(
            probe.clicks.load(Ordering::SeqCst),
            0,
            "provider calls only enqueue"
        );
        assert!(probe.changes.lock().expect("changes").is_empty());

        host.pump().expect("drain the queued actions");
        assert_eq!(
            probe.clicks.load(Ordering::SeqCst),
            1,
            "Invoke runs the click handler once"
        );
        assert_eq!(
            *probe.changes.lock().expect("changes"),
            [true],
            "Toggle runs one change"
        );
        assert_eq!(
            *probe.events.lock().expect("events"),
            ["click", "change"],
            "actions drain in queue order"
        );
        assert_eq!(
            host.accessibility_tree(MAIN)
                .expect("published tree")
                .find_name("Flag")
                .and_then(|node| node.toggle_state()),
            Some(true)
        );
        assert_eq!(
            unsafe { toggle.ToggleState() }.expect("state after toggling"),
            ToggleState_On
        );

        unsafe { toggle.Toggle() }.expect("queue the reverse toggle");
        host.pump().expect("drain");
        assert_eq!(*probe.changes.lock().expect("changes"), [true, false]);
        assert_eq!(
            unsafe { toggle.ToggleState() }.expect("state after the reverse toggle"),
            ToggleState_Off
        );
    }

    #[test]
    fn actions_report_unavailable_for_disabled_stale_and_hidden_targets() {
        // 静态禁用的控件：provider 在调用线程上直接报告不可用。
        let mut disabled_host = open_disabled_probe();
        let root = root_fragment(&mut disabled_host);
        let dead = provider_named(&root, "Dead");
        let invoke: IInvokeProvider = pattern(&dead, UIA_InvokePatternId);
        let error = unsafe { invoke.Invoke() }.expect_err("disabled targets are unavailable");
        assert_eq!(error.code(), HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32));
        let locked = provider_named(&root, "Locked");
        let value: IValueProvider = pattern(&locked, UIA_ValuePatternId);
        assert_eq!(unsafe { value.IsReadOnly() }.expect("read-only").0, 1);
        let error =
            unsafe { value.SetValue(w!("blocked")) }.expect_err("disabled inputs are read-only");
        assert_eq!(error.code(), HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32));
        disabled_host.pump().expect("drain the rejected actions");

        // 先取到 provider，再让目标卸载：旧 provider 必须报告不可用。
        let (mut host, probe) = open_uia_probe();
        let root = root_fragment(&mut host);
        let ephemeral = provider_named(&root, "Ephemeral");
        let ephemeral_invoke: IInvokeProvider = pattern(&ephemeral, UIA_InvokePatternId);
        let hide = provider_named(&root, "Hide");
        let hide_invoke: IInvokeProvider = pattern(&hide, UIA_InvokePatternId);
        unsafe { hide_invoke.Invoke() }.expect("queue hiding the ephemeral button");
        host.pump().expect("drain the hide");
        assert!(
            host.accessibility_tree(MAIN)
                .expect("published tree")
                .find_name("Ephemeral")
                .is_none(),
            "the gated button is unmounted"
        );
        let error =
            unsafe { ephemeral_invoke.Invoke() }.expect_err("stale targets are unavailable");
        assert_eq!(error.code(), HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32));

        let act = provider_named(&root, "Act");
        let act_invoke: IInvokeProvider = pattern(&act, UIA_InvokePatternId);
        host.pump().expect("drain the rejected actions");
        assert_eq!(
            probe.clicks.load(Ordering::SeqCst),
            0,
            "rejected actions run no handler"
        );

        host.hide(MAIN).expect("hide the window");
        let error = unsafe { act_invoke.Invoke() }.expect_err("hidden windows have no tree");
        assert_eq!(error.code(), HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32));
        host.pump().expect("drain after hiding");
        assert_eq!(probe.clicks.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn queued_actions_are_revalidated_against_the_latest_snapshot() {
        let (mut host, probe) = open_uia_probe();
        let root = root_fragment(&mut host);
        let act = provider_named(&root, "Act");
        let disable = provider_named(&root, "Disable");
        let act_invoke: IInvokeProvider = pattern(&act, UIA_InvokePatternId);
        let disable_invoke: IInvokeProvider = pattern(&disable, UIA_InvokePatternId);

        // provider 在入队时看到的目标仍然启用；中间的动作会禁用它。
        unsafe { act_invoke.Invoke() }.expect("queue the first click");
        unsafe { disable_invoke.Invoke() }.expect("queue disabling the button");
        unsafe { act_invoke.Invoke() }.expect("queue a click that becomes invalid");
        assert_eq!(probe.clicks.load(Ordering::SeqCst), 0);

        host.pump().expect("drain in order");
        assert_eq!(
            probe.clicks.load(Ordering::SeqCst),
            1,
            "the UI thread drops the click queued before the disable"
        );
        assert_eq!(
            *probe.events.lock().expect("events"),
            ["click", "disable"],
            "the disabled target runs no handler"
        );
        assert!(
            !host
                .accessibility_tree(MAIN)
                .expect("published tree")
                .find_name("Act")
                .expect("the button")
                .enabled(),
            "the republished tree reports the disabled button"
        );
    }

    #[test]
    fn value_pattern_replaces_the_draft_once_and_records_one_undo() {
        let (mut host, probe) = open_uia_probe();
        let root = root_fragment(&mut host);
        let field = provider_named(&root, "Field");
        let value: IValueProvider = pattern(&field, UIA_ValuePatternId);
        assert_eq!(unsafe { value.Value() }.expect("value").to_string(), "old");
        assert_eq!(unsafe { value.IsReadOnly() }.expect("writable").0, 0);

        unsafe { value.SetValue(w!("new value")) }.expect("queue the replacement");
        assert!(probe.inputs.lock().expect("inputs").is_empty());
        host.pump().expect("drain the replacement");
        assert_eq!(
            *probe.inputs.lock().expect("inputs"),
            [String::from("new value")],
            "SetValue dispatches exactly one input"
        );

        let snapshot = host.presentation(MAIN).expect("snapshot");
        let input = snapshot.find_label("Field").expect("the draft input");
        assert_eq!(input.text_value(), Some("new value"));
        assert_eq!(input.edit().expect("text input").caret(), 9);
        let node = input.id();

        host.runtime.undo_node(node).expect("undo the replacement");
        assert_eq!(
            host.presentation(MAIN)
                .expect("snapshot")
                .node(node)
                .expect("input")
                .text_value(),
            Some("old"),
            "one undo restores the value before the replacement"
        );
        host.runtime
            .undo_node(node)
            .expect("a second undo is a no-op");
        assert_eq!(
            host.presentation(MAIN)
                .expect("snapshot")
                .node(node)
                .expect("input")
                .text_value(),
            Some("old"),
            "the replacement recorded exactly one undo entry"
        );
        assert_eq!(
            *probe.inputs.lock().expect("inputs"),
            [String::from("new value"), String::from("old")],
            "the replacement and the undo each dispatch one input"
        );

        // 组合期间：provider 仍按旧树入队，UI 线程按最新快照拒绝。
        host.runtime
            .compose_start_node(node)
            .expect("start composition");
        host.runtime
            .compose_update_node(node, "ka")
            .expect("update the preedit");
        unsafe { value.SetValue(w!("blocked")) }.expect("the stale tree looks writable");
        host.pump().expect("drain the composing action");
        assert_eq!(
            probe.inputs.lock().expect("inputs").len(),
            2,
            "no input is dispatched while composing"
        );

        host.render().expect("republish the composition");
        assert_eq!(unsafe { value.IsReadOnly() }.expect("read-only").0, 1);
        let error =
            unsafe { value.SetValue(w!("blocked")) }.expect_err("a composing input is read-only");
        assert_eq!(error.code(), HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32));
    }

    #[test]
    fn set_focus_only_queues_a_focusable_target_and_never_steals_foreground() {
        let (mut host, _probe) = open_uia_probe();
        let root = root_fragment(&mut host);
        let field = provider_named(&root, "Field");
        let fragment: IRawElementProviderFragment = field.cast().expect("fragment");
        assert!(host.presentation(MAIN).expect("snapshot").focus().is_none());

        unsafe { fragment.SetFocus() }.expect("queue focus for the focusable input");
        host.pump().expect("drain");
        assert!(
            host.presentation(MAIN).expect("snapshot").focus().is_none(),
            "a background window does not move runtime focus"
        );

        let caption = provider_named(&root, "Caption");
        let caption: IRawElementProviderFragment = caption.cast().expect("fragment");
        let error = unsafe { caption.SetFocus() }.expect_err("text is not focusable");
        assert_eq!(error.code(), HRESULT(UIA_E_NOTSUPPORTED as i32));
    }

    #[test]
    fn text_pattern_reads_the_document_and_selection_in_utf16_offsets() {
        let (mut host, _probe) = open_uia_probe();
        let root = root_fragment(&mut host);
        let field = provider_named(&root, "Field");
        let text: ITextProvider = pattern(&field, UIA_TextPatternId);
        assert_eq!(
            unsafe { text.SupportedTextSelection() }.expect("supported selection"),
            SupportedTextSelection_Single
        );
        let document = unsafe { text.DocumentRange() }.expect("document range");
        assert_eq!(range_text(&document), "old");
        let tree = host.accessibility_tree(MAIN).expect("tree");
        let field_node = tree.find_name("Field").expect("the field");
        assert_range_rectangles(&host, &document, &[tree.logical_rect(field_node)]);
        assert_eq!(
            unsafe { document.GetText(2) }.expect("prefix").to_string(),
            "ol"
        );
        assert_eq!(
            unsafe { document.GetText(0) }
                .expect("empty prefix")
                .to_string(),
            ""
        );
        let visible = unsafe { text.GetVisibleRanges() }.expect("visible ranges");
        assert_eq!(range_text(&text_range_at(visible, 0)), "old");
        let ranges = unsafe { text.GetSelection() }.expect("selection");
        let selection = text_range_at(ranges, 0);
        assert_eq!(range_text(&selection), "");
        assert!(
            rect_values(&selection).is_empty(),
            "an unfocused input has no caret geometry"
        );

        let value: IValueProvider = pattern(&field, UIA_ValuePatternId);
        unsafe { value.SetValue(w!("a中🙂e\u{301}z")) }.expect("replace the value");
        host.pump().expect("drain the replacement");
        let document = unsafe { text.DocumentRange() }.expect("new document range");
        assert_eq!(range_text(&document), "a中🙂e\u{301}z");
        let node = host
            .presentation(MAIN)
            .expect("snapshot")
            .find_label("Field")
            .expect("the field")
            .id();

        host.runtime.focus_node(node).expect("focus the field");
        host.render().expect("render the focus");
        for (range, expected) in [
            ((0usize, 1usize), "a"),
            ((1, 2), "中"),
            ((2, 3), "🙂"),
            ((3, 5), "e\u{301}"),
        ] {
            host.runtime
                .set_selection_node(node, range.0, range.1)
                .expect("select a range");
            host.render().expect("render the selection");
            let snapshot = host.presentation(MAIN).expect("snapshot");
            let edit = snapshot
                .find_label("Field")
                .expect("the field")
                .edit()
                .expect("edit state");
            assert_eq!(edit.caret(), range.1);
            assert_eq!(edit.selection(), Some(range));
            let ranges = unsafe { text.GetSelection() }.expect("selection");
            let selected = text_range_at(ranges, 0);
            assert_eq!(
                range_text(&selected),
                expected,
                "GetSelection matches TextEditState for {range:?}"
            );
            let selections: Vec<Rect> = host
                .scene(MAIN)
                .expect("scene")
                .decorations()
                .iter()
                .filter(|decoration| {
                    decoration.node() == node && decoration.kind() == DecorationKind::Selection
                })
                .map(|decoration| decoration.rect())
                .collect();
            assert!(
                !selections.is_empty(),
                "the selection has visible geometry for {range:?}"
            );
            assert_range_rectangles(&host, &selected, &selections);
        }

        host.runtime
            .set_caret_node(node, 4)
            .expect("place the caret");
        host.render().expect("render the caret");
        let snapshot = host.presentation(MAIN).expect("snapshot");
        let edit = snapshot
            .find_label("Field")
            .expect("the field")
            .edit()
            .expect("edit state");
        assert_eq!(edit.caret(), 4);
        assert_eq!(
            edit.selection(),
            Some((4, 4)),
            "set_caret stores a collapsed selection"
        );
        let ranges = unsafe { text.GetSelection() }.expect("selection");
        let caret = text_range_at(ranges, 0);
        assert_eq!(range_text(&caret), "");
        let caret_rect = host
            .scene(MAIN)
            .expect("scene")
            .caret(node)
            .expect("caret geometry");
        assert_range_rectangles(&host, &caret, &[caret_rect]);
    }

    #[test]
    fn text_pattern_reports_no_visible_text_when_the_input_is_scrolled_out() {
        let mut host = open_long_text_host();
        let mut pages = 0;
        while host.scene(MAIN).expect("scene").scroll_offset()
            < host.scene(MAIN).expect("scene").max_scroll_offset()
            && pages < 40
        {
            send_msg(
                &mut host,
                MSG {
                    message: WM_KEYDOWN,
                    wParam: WPARAM(VK_NEXT.0 as usize),
                    ..Default::default()
                },
            );
            pages += 1;
        }
        assert!(pages > 0, "the long host scrolls");
        let tree = host.accessibility_tree(MAIN).expect("tree");
        assert!(
            tree.find_name("What needs to be done?")
                .expect("the draft input")
                .offscreen(),
            "the draft is scrolled out of view"
        );

        let root = root_fragment(&mut host);
        let field = provider_named(&root, "What needs to be done?");
        let text: ITextProvider = pattern(&field, UIA_TextPatternId);
        let visible = unsafe { text.GetVisibleRanges() }.expect("visible ranges");
        let visible = text_range_at(visible, 0);
        assert_eq!(
            range_text(&visible),
            "",
            "no visible text returns a degenerate range"
        );
        assert!(
            rect_values(&visible).is_empty(),
            "a fully offscreen range has no rectangles"
        );
        let document = unsafe { text.DocumentRange() }.expect("document range");
        assert!(
            rect_values(&document).is_empty(),
            "a fully offscreen document returns an empty array, not NULL"
        );
    }

    #[test]
    fn text_pattern_rejects_every_unsupported_range_operation() {
        let (mut host, _probe) = open_uia_probe();
        let root = root_fragment(&mut host);
        let field = provider_named(&root, "Field");
        let text: ITextProvider = pattern(&field, UIA_TextPatternId);
        let document = unsafe { text.DocumentRange() }.expect("document range");
        let other = unsafe { document.Clone() }.expect("cloned range");
        assert_eq!(range_text(&other), "old");

        let error = unsafe { text.RangeFromPoint(UiaPoint { x: 0.0, y: 0.0 }) }
            .expect_err("RangeFromPoint is not part of this surface");
        assert_not_supported(error);
        let error = unsafe { text.RangeFromChild(None) }
            .expect_err("RangeFromChild is not part of this surface");
        assert_not_supported(error);
        let error = unsafe { document.Move(TextUnit_Character, 1) }
            .expect_err("Move is not part of this surface");
        assert_not_supported(error);
        let error = unsafe { document.ExpandToEnclosingUnit(TextUnit_Character) }
            .expect_err("ExpandToEnclosingUnit is not part of this surface");
        assert_not_supported(error);
        let error =
            unsafe { document.Compare(&other) }.expect_err("Compare is not part of this surface");
        assert_not_supported(error);
        let error = unsafe {
            document.CompareEndpoints(
                TextPatternRangeEndpoint_Start,
                &other,
                TextPatternRangeEndpoint_End,
            )
        }
        .expect_err("CompareEndpoints is not part of this surface");
        assert_not_supported(error);
        let error = unsafe {
            document.MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, 1)
        }
        .expect_err("MoveEndpointByUnit is not part of this surface");
        assert_not_supported(error);
        let error = unsafe {
            document.MoveEndpointByRange(
                TextPatternRangeEndpoint_Start,
                &other,
                TextPatternRangeEndpoint_End,
            )
        }
        .expect_err("MoveEndpointByRange is not part of this surface");
        assert_not_supported(error);
        let error =
            unsafe { document.FindAttribute(UIA_FontNameAttributeId, &VARIANT::default(), false) }
                .expect_err("FindAttribute is not part of this surface");
        assert_not_supported(error);
        let error = unsafe { document.FindText(&BSTR::from("old"), false, false) }
            .expect_err("FindText is not part of this surface");
        assert_not_supported(error);
        let error = unsafe { document.GetAttributeValue(UIA_FontNameAttributeId) }
            .expect_err("GetAttributeValue is not part of this surface");
        assert_not_supported(error);
        let error = unsafe { document.ScrollIntoView(false) }
            .expect_err("ScrollIntoView is not part of this surface");
        assert_not_supported(error);
        let error = unsafe { document.AddToSelection() }
            .expect_err("AddToSelection is not part of this surface");
        assert_not_supported(error);
        let error = unsafe { document.RemoveFromSelection() }
            .expect_err("RemoveFromSelection is not part of this surface");
        assert_not_supported(error);
        let error =
            unsafe { document.GetChildren() }.expect_err("GetChildren is not part of this surface");
        assert_not_supported(error);

        let enclosing = unsafe { document.GetEnclosingElement() }.expect("enclosing element");
        assert_eq!(provider_name(&enclosing), "Field");
    }

    #[test]
    fn text_pattern_select_moves_the_selection_without_dispatching_input() {
        let (mut host, probe) = open_uia_probe();
        let root = root_fragment(&mut host);
        let field = provider_named(&root, "Field");
        let text: ITextProvider = pattern(&field, UIA_TextPatternId);
        let node = host
            .presentation(MAIN)
            .expect("snapshot")
            .find_label("Field")
            .expect("the field")
            .id();
        host.runtime.focus_node(node).expect("focus the field");
        host.runtime
            .set_selection_node(node, 1, 3)
            .expect("select a range");
        host.render().expect("render the selection");

        let ranges = unsafe { text.GetSelection() }.expect("selection");
        let selected = text_range_at(ranges, 0);
        assert_eq!(range_text(&selected), "ld");
        host.runtime
            .set_caret_node(node, 0)
            .expect("move the caret");
        host.render().expect("render the caret");
        unsafe { selected.Select() }.expect("queue the selection");
        assert!(
            probe.inputs.lock().expect("inputs").is_empty(),
            "Select only queues a selection change"
        );
        host.pump().expect("drain the selection");
        assert!(
            probe.inputs.lock().expect("inputs").is_empty(),
            "Select dispatches no on:input"
        );
        let snapshot = host.presentation(MAIN).expect("snapshot");
        let edit = snapshot
            .find_label("Field")
            .expect("the field")
            .edit()
            .expect("edit state");
        assert_eq!(edit.caret(), 3, "the caret moves to the range focus");
        assert_eq!(edit.anchor(), 1);
        assert_eq!(edit.selection(), Some((1, 3)));

        let document = unsafe { text.DocumentRange() }.expect("document range");
        unsafe { document.Select() }.expect("queue selecting the document");
        host.pump().expect("drain the selection");
        let snapshot = host.presentation(MAIN).expect("snapshot");
        let edit = snapshot
            .find_label("Field")
            .expect("the field")
            .edit()
            .expect("edit state");
        assert_eq!(edit.selection(), Some((0, 3)));
        assert_eq!(edit.caret(), 3);
        assert!(
            probe.inputs.lock().expect("inputs").is_empty(),
            "selecting the document keeps on:input silent"
        );
    }

    #[test]
    fn text_pattern_keeps_composing_disabled_and_static_inputs_read_only() {
        let (mut host, probe) = open_uia_probe();
        let node = host
            .presentation(MAIN)
            .expect("snapshot")
            .find_label("Field")
            .expect("the field")
            .id();
        host.runtime.focus_node(node).expect("focus the field");
        host.runtime
            .set_selection_node(node, 1, 3)
            .expect("select before composing");
        host.runtime
            .compose_start_node(node)
            .expect("start the composition");
        host.runtime
            .compose_update_node(node, "X")
            .expect("update the preedit");
        host.render().expect("render the composition");
        let snapshot = host.presentation(MAIN).expect("snapshot");
        let edit = snapshot
            .find_label("Field")
            .expect("the field")
            .edit()
            .expect("edit state");
        assert_eq!(edit.caret(), 2, "the display caret follows the preedit");
        assert_eq!(edit.composition(), Some((1, 3)));
        assert_eq!(edit.selection(), None);
        let tree = host.accessibility_tree(MAIN).expect("tree");
        let geometry = tree
            .find_name("Field")
            .expect("the field")
            .text_geometry()
            .expect("text geometry");
        assert_eq!(
            geometry.caret(),
            3,
            "the document caret is the end of the replaced range, not the display offset"
        );
        assert_eq!(geometry.selection(), None);
        let root = root_fragment(&mut host);
        let field = provider_named(&root, "Field");
        let text: ITextProvider = pattern(&field, UIA_TextPatternId);
        let document = unsafe { text.DocumentRange() }.expect("document range");
        assert_eq!(
            range_text(&document),
            "old",
            "the document excludes the preedit"
        );
        let ranges = unsafe { text.GetSelection() }.expect("selection is readable");
        let selection = text_range_at(ranges, 0);
        assert_eq!(range_text(&selection), "");
        let error = unsafe { document.Select() }.expect_err("a composing input is read-only");
        assert_eq!(error.code(), HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32));
        host.pump().expect("drain the rejected selection");
        assert!(
            probe.inputs.lock().expect("inputs").is_empty(),
            "a rejected Select dispatches no on:input"
        );

        let mut disabled = open_disabled_probe();
        let root = root_fragment(&mut disabled);
        let locked = provider_named(&root, "Locked");
        let text: ITextProvider = pattern(&locked, UIA_TextPatternId);
        let document = unsafe { text.DocumentRange() }.expect("a disabled document is readable");
        assert_eq!(range_text(&document), "frozen");
        let error = unsafe { document.Select() }.expect_err("a disabled input is read-only");
        assert_eq!(error.code(), HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32));
        disabled.pump().expect("drain the rejected selection");

        let mut read_only = open_read_only_probe();
        let root = root_fragment(&mut read_only);
        let field = provider_named(&root, "Static");
        let text: ITextProvider = pattern(&field, UIA_TextPatternId);
        let document = unsafe { text.DocumentRange() }.expect("a static document is readable");
        assert_eq!(range_text(&document), "static text");
        let error =
            unsafe { document.Select() }.expect_err("an input without on:input is read-only");
        assert_eq!(error.code(), HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32));
        read_only.pump().expect("drain the rejected selection");
    }

    #[test]
    fn suspended_parent_targets_report_unavailable_and_keep_their_value() {
        let mut host = open_confirm_host();
        send_msg(
            &mut host,
            MSG {
                message: WM_CLOSE,
                ..Default::default()
            },
        );
        assert!(!host.windows[MAIN].enabled, "the parent is close-suspended");

        let root = root_fragment(&mut host);
        let draft = provider_named(&root, "draft");
        let value: IValueProvider = pattern(&draft, UIA_ValuePatternId);
        let error = unsafe { value.SetValue(w!("blocked")) }
            .expect_err("a suspended parent rejects writes");
        assert_eq!(error.code(), HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32));

        let fragment: IRawElementProviderFragment = draft.cast().expect("fragment");
        let error = unsafe { fragment.SetFocus() }.expect_err("a suspended parent rejects focus");
        assert_eq!(error.code(), HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32));

        host.pump().expect("drain the rejected actions");
        assert_eq!(
            host.presentation(MAIN)
                .expect("snapshot")
                .find_label("draft")
                .expect("the draft input")
                .text_value(),
            Some("")
        );
    }

    #[test]
    fn an_action_enqueued_from_another_thread_runs_only_after_pump() {
        let (mut host, probe) = open_uia_probe();
        let node = host
            .accessibility_tree(MAIN)
            .expect("published tree")
            .find_name("Act")
            .and_then(|node| node.node_id())
            .expect("the action node");
        let accessibility = Arc::clone(&host.windows[MAIN].accessibility);
        std::thread::spawn(move || {
            assert!(accessibility.enqueue(AccessibilityAction::Invoke(node)));
        })
        .join()
        .expect("the provider thread enqueues");

        assert_eq!(
            probe.clicks.load(Ordering::SeqCst),
            0,
            "off-thread calls only change the queue"
        );
        host.pump().expect("the wake message drains the queue");
        assert_eq!(probe.clicks.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_accessibility_action_queue_rejects_overflow() {
        let (host, _probe) = open_uia_probe();
        let node = host
            .accessibility_tree(MAIN)
            .expect("published tree")
            .find_name("Act")
            .and_then(|node| node.node_id())
            .expect("the action node");
        let accessibility = Arc::clone(&host.windows[MAIN].accessibility);
        let limit = crate::accessibility::MAX_PENDING_ACTIONS;
        for _ in 0..limit {
            assert!(accessibility.enqueue(AccessibilityAction::Invoke(node)));
        }
        assert!(
            !accessibility.enqueue(AccessibilityAction::Invoke(node)),
            "a full queue rejects the next action instead of growing"
        );
        assert_eq!(accessibility.take_actions().len(), limit);
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
    fn sc_close_reports_the_system_menu_source() {
        let source: Rc<RefCell<Option<CloseSource>>> = Rc::new(RefCell::new(None));
        let recorded = source.clone();
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(move |catalog| {
                catalog.window(
                    WindowSpec::new(MAIN)
                        .primary()
                        .close_policy(ClosePolicy::Confirm),
                    move |window| {
                        let recorded = recorded.clone();
                        window
                            .on_close(move |event| {
                                *recorded.borrow_mut() = Some(event.source());
                                window
                                    .request_show("confirm")
                                    .expect("show the confirmation");
                                CloseOutcome::Pending
                            })
                            .expect("register the close handler");
                        view! { <view /> }
                    },
                )?;
                catalog.window(
                    WindowSpec::new("confirm").confirmation_for(MAIN),
                    |window| {
                        window
                            .close_request()
                            .expect("read the pending request")
                            .expect("a request exists");
                        view! { <view /> }
                    },
                )?;
                Ok(())
            })
            .expect("declare the windows");
        runtime.show(MAIN).expect("show the window");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open host");
        host.render().expect("render the parent");

        send_msg(
            &mut host,
            MSG {
                message: WM_SYSCOMMAND,
                wParam: WPARAM(SC_CLOSE as usize),
                ..Default::default()
            },
        );
        assert_eq!(*source.borrow(), Some(CloseSource::SystemMenu));
        assert!(!host.windows[MAIN].enabled, "the parent is close-suspended");
    }

    #[test]
    fn a_closed_host_ignores_later_paint_and_input() {
        let mut host = open_host();
        host.render().expect("initial render");
        let hwnd = host.hwnd(MAIN).expect("a live window");
        host.close_from_window(hwnd, CloseSource::Programmatic);
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
    fn a_new_loss_prevents_clearing_the_previous_epoch() {
        let loss = DeviceLossSignal::new();
        loss.set_generation(1);
        loss.raise(1);
        let epoch = loss.epoch();
        assert!(loss.is_lost());

        loss.mark_lost();
        assert!(
            !loss.clear_if(epoch),
            "a newer loss is not cleared by the older recovery"
        );
        assert!(loss.is_lost());
        assert!(loss.clear_if(loss.epoch()));
        assert!(!loss.is_lost());
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

    #[component]
    fn WindowTaskProbe(window: WindowCtx) -> MountResult<MountPlan> {
        let start = move || {
            let _ = window.task_scope().spawn(|_| String::from("done"));
        };
        view! {
            <view role="main">
                <button on:click={move || start()}>"Start task"</button>
            </view>
        }
    }

    fn open_task_host() -> Host {
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(|catalog| {
                catalog.window(
                    WindowSpec::new(MAIN).primary().sized(320, 240),
                    |window| view! { <WindowTaskProbe window={window} /> },
                )?;
                Ok(())
            })
            .expect("declare the task probe window");
        runtime.show(MAIN).expect("show the task probe window");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open the task probe host");
        host.render().expect("render the task probe window");
        host
    }

    fn open_delivering_task_host() -> (Host, &'static AtomicUsize) {
        let completed = leak(AtomicUsize::new(0));
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(move |catalog| {
                catalog.window(
                    WindowSpec::new(MAIN).primary().sized(320, 240),
                    move |window| {
                        let start = move || {
                            let _ = window
                                .task_scope()
                                .spawn(|_| String::from("done"))
                                .and_then(|task| {
                                    task.on_done(move |_: String| {
                                        completed.fetch_add(1, Ordering::SeqCst);
                                        Ok(())
                                    })
                                });
                        };
                        view! {
                            <view role="main">
                                <button on:click={move || start()}>"Start task"</button>
                            </view>
                        }
                    },
                )?;
                Ok(())
            })
            .expect("declare the delivering task window");
        runtime.show(MAIN).expect("show the delivering task window");
        let mut host =
            Host::open(runtime, HostConfig::hidden()).expect("open the delivering task host");
        host.render().expect("render the delivering task window");
        (host, completed)
    }

    #[test]
    fn closing_the_window_cancels_window_owned_tasks() {
        let mut host = open_task_host();
        // 事件 handler 在已安装的运行时上下文里启动窗口所有权任务；
        // 不 pump 唤醒消息，确保关闭发生在交付之前。
        host.runtime
            .click_text("Start task")
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

        host.surface_lost().expect("mark loss");
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
    fn device_loss_preserves_the_applied_scroll_offset() {
        let mut host = open_host();
        for index in 0..100 {
            host.runtime
                .input("What needs to be done?", &format!("Task {index}"))
                .expect("fill the draft");
            host.runtime.submit("Add task").expect("add a row");
        }
        host.render().expect("render the tall window");
        let max = host.scene(MAIN).expect("scene").max_scroll_offset();
        assert!(max > 0, "the seeded rows overflow the viewport");

        host.windows.get_mut(MAIN).expect("main").scroll_offset = 72;
        host.render().expect("render at the scrolled offset");
        assert_eq!(host.scene(MAIN).expect("scene").scroll_offset(), 72);

        host.surface_lost().expect("mark loss");
        host.recover_device().expect("rebuild every surface");

        assert_eq!(
            host.scene(MAIN)
                .expect("scene after recovery")
                .scroll_offset(),
            72,
            "a recreated surface keeps the applied offset"
        );
    }

    #[test]
    fn destroyed_device_recovers_once_and_preserves_ui_state() {
        let mut host = open_host();
        host.runtime
            .focus("What needs to be done?")
            .expect("focus the draft");
        host.runtime
            .input("What needs to be done?", "Recovery draft")
            .expect("type the draft");
        let node = host
            .runtime
            .presentation()
            .expect("snapshot")
            .focus()
            .expect("the focused draft");
        host.runtime
            .set_selection_node(node, 1, 3)
            .expect("selection");
        host.runtime
            .set_appearance(Appearance::Dark)
            .expect("dark appearance");
        host.render().expect("render before the loss");
        let ids: Vec<_> = host
            .scene(MAIN)
            .expect("scene")
            .elements()
            .iter()
            .map(|element| element.id())
            .collect();
        let before = host.read_frame(MAIN).expect("frame before the loss");
        let generation = host.device_generation;

        host.device.destroy();
        let _ = host.device.poll(wgpu::PollType::wait_indefinitely());
        host.render().expect("render after the device is destroyed");

        assert_eq!(
            host.device_generation,
            generation + 1,
            "one device generation is created"
        );
        assert_eq!(
            host.loss_phase,
            LossPhase::Clear,
            "a successful present clears the loss"
        );
        assert!(!host.loss.is_lost(), "the loss signal is cleared");
        assert!(
            host.windows.values().all(|window| window.surface.is_some()),
            "every live window gets a surface from the new device"
        );
        let snapshot = host.runtime.presentation().expect("snapshot after");
        assert_eq!(snapshot.focus(), Some(node), "focus survives");
        let draft = snapshot.node(node).expect("draft");
        assert_eq!(draft.text_value(), Some("Recovery draft"), "draft survives");
        let edit = draft.edit().expect("edit state");
        assert_eq!(edit.selection(), Some((1, 3)), "selection survives");
        assert_eq!(edit.caret(), 3, "caret survives");
        assert_eq!(
            snapshot.appearance(),
            Appearance::Dark,
            "appearance survives"
        );
        assert_eq!(
            host.scene(MAIN)
                .expect("scene after")
                .elements()
                .iter()
                .map(|element| element.id())
                .collect::<Vec<_>>(),
            ids,
            "Node IDs survive"
        );
        let after = host.read_frame(MAIN).expect("frame after the recovery");
        assert_eq!(before.pixels(), after.pixels());

        host.render().expect("render again");
        assert_eq!(
            host.device_generation,
            generation + 1,
            "recovery is not re-entered"
        );

        let before_char = host.read_frame(MAIN).expect("frame before typing");
        send_msg(
            &mut host,
            MSG {
                message: WM_CHAR,
                wParam: WPARAM('X' as usize),
                ..Default::default()
            },
        );
        let after_char = host.read_frame(MAIN).expect("frame after typing");
        assert_ne!(
            before_char.pixels(),
            after_char.pixels(),
            "the rebuilt device renders a typed character"
        );
    }

    #[test]
    fn device_destroy_posts_the_loss_message_and_recovers_on_pump() {
        let mut host = open_host();
        host.render().expect("render before the loss");
        let generation = host.device_generation;

        host.device.destroy();
        let _ = host.device.poll(wgpu::PollType::wait_indefinitely());
        host.pump().expect("pump the pending loss message");

        assert_eq!(
            host.device_generation,
            generation + 1,
            "the callback message rebuilds the device"
        );
        assert!(
            host.windows.values().all(|window| window.surface.is_some()),
            "every live window gets a surface"
        );
        let frame = host.read_frame(MAIN).expect("frame after recovery");
        assert!(!frame.pixels().is_empty());
    }

    #[test]
    fn a_one_shot_rebuild_failure_recovers_on_the_second_attempt() {
        let mut host = open_host();
        host.render().expect("render before the loss");
        let generation = host.device_generation;

        host.injected_rebuild_failures = 1;
        host.device.destroy();
        let _ = host.device.poll(wgpu::PollType::wait_indefinitely());
        host.pump()
            .expect("the failure is retried from the message loop");

        assert!(
            host.latched_loss.is_none(),
            "one failure stays inside the bound"
        );
        assert_eq!(
            host.recovery_attempts, 0,
            "a proving present resets the attempt counter"
        );
        assert_eq!(host.device_generation, generation + 1);
        assert_eq!(host.loss_phase, LossPhase::Clear);
        assert!(!host.loss.is_lost());
        assert!(host.windows.values().all(|window| window.surface.is_some()));
    }

    #[test]
    fn a_permanent_failure_latches_after_three_attempts() {
        let mut host = open_host();
        host.render().expect("render before the loss");
        let generation = host.device_generation;
        let current_adapter = host.adapter_info();

        host.injected_rebuild_failures = 99;
        host.device.destroy();
        let _ = host.device.poll(wgpu::PollType::wait_indefinitely());
        let error = host.pump().expect_err("the bounded recovery gives up");
        let HostError::DeviceLost(lost) = error else {
            panic!("the terminal error is typed: {error}");
        };
        assert_eq!(lost.attempts, 3, "exactly the bounded attempts run");
        assert_eq!(
            lost.adapter, current_adapter,
            "without an attempt adapter the terminal error falls back to the current device"
        );
        assert!(
            !lost.message.is_empty(),
            "the terminal error keeps the last failure"
        );
        assert_eq!(host.injected_rebuild_failures, 96);
        assert_eq!(
            host.device_generation, generation,
            "no failed device is published"
        );

        let error = host
            .pump()
            .expect_err("later pumps return the latched error");
        let HostError::DeviceLost(lost) = error else {
            panic!("the latched error stays typed: {error}");
        };
        assert_eq!(lost.attempts, 3);
        assert_eq!(
            host.injected_rebuild_failures, 96,
            "a latched host never attempts another rebuild"
        );
        assert!(
            matches!(host.read_frame(MAIN), Err(HostError::DeviceLost(_))),
            "readback returns the latched error"
        );
    }

    #[test]
    fn a_failed_staging_attempt_still_posts_the_retry() {
        let mut host = open_host();
        host.render().expect("render before the loss");
        let generation = host.device_generation;

        // 安装新回调之后失败的重建：信号代际已推进到未发布的设备，
        // 宿主已发布的代际仍是旧值。
        host.loss.mark_lost();
        host.loss.set_generation(generation + 1);
        host.loss_phase = LossPhase::Pending;
        host.injected_rebuild_failures = 1;
        host.recover_device()
            .expect_err("the injected attempt fails before publishing");
        assert_eq!(
            host.device_generation, generation,
            "the failed attempt does not publish a generation"
        );
        assert!(
            matches!(host.read_frame(MAIN), Err(HostError::Surface(_))),
            "readback during a pending recovery reports the lost device"
        );

        host.pump().expect("the queued retry recovers");

        assert_eq!(host.device_generation, generation + 1);
        assert_eq!(host.loss_phase, LossPhase::Clear);
        assert!(!host.loss.is_lost());
    }

    #[test]
    fn a_rejected_staging_message_reschedules_the_pending_recovery() {
        let mut host = open_host();
        host.render().expect("render before the loss");
        let generation = host.device_generation;

        // 安装回调后失败的重建：丢弃设备的回调抢占了合并标志，失败尝试自己
        // 的重试被合并掉；派发时它又因代际超前而被拒绝。
        host.loss.mark_lost();
        host.loss.set_generation(generation + 1);
        host.next_device_generation = generation + 1;
        host.loss.raise(generation + 1);
        host.loss_phase = LossPhase::Pending;
        host.recovery_attempts = 1;

        host.pump()
            .expect("the rejected message reschedules the recovery");

        assert_eq!(
            host.device_generation,
            generation + 2,
            "the retry publishes a fresh generation, not the discarded one"
        );
        assert_eq!(host.loss_phase, LossPhase::Clear);
        assert!(!host.loss.is_lost());
        assert_eq!(host.recovery_attempts, 0);
    }

    #[test]
    fn readback_refuses_only_the_lost_states() {
        let mut host = open_host();
        host.render().expect("render before the loss");

        host.loss.mark_lost();
        assert!(
            matches!(host.read_frame(MAIN), Err(HostError::Surface(_))),
            "a signalled loss refuses readback before the message is dispatched"
        );

        host.loss_phase = LossPhase::AwaitingPresent;
        host.loss_epoch = 0;
        assert!(
            matches!(host.read_frame(MAIN), Err(HostError::Surface(_))),
            "a stale proving pass refuses readback"
        );

        host.loss_epoch = host.loss.epoch();
        assert!(
            host.read_frame(MAIN).is_ok(),
            "the rebuilt device still reads back while its first present is pending"
        );
    }

    #[test]
    fn the_terminal_error_prefers_the_final_attempt_adapter() {
        let mut host = open_host();
        let final_attempt = GpuAdapterInfo {
            name: String::from("final attempt"),
            backend: String::from("Dx12"),
            device_type: String::from("Cpu"),
            driver: String::from("warp"),
            driver_info: String::new(),
            fallback: true,
        };
        host.attempt_adapter = Some(final_attempt.clone());
        host.recovery_attempts = MAX_RECOVERY_ATTEMPTS;
        let error = host.latch_device_lost(String::from("the fallback device stayed lost"));
        let HostError::DeviceLost(lost) = error else {
            panic!("the terminal error is typed: {error}");
        };
        assert_eq!(lost.attempts, MAX_RECOVERY_ATTEMPTS);
        assert_eq!(
            lost.adapter, final_attempt,
            "the terminal error reports the adapter of the final attempt"
        );
        assert!(
            lost.adapter.fallback,
            "a WARP final attempt stays observable"
        );
    }

    #[test]
    fn a_failed_attempt_keeps_the_episode_open_for_the_next_window_action() {
        let mut host = open_two_window_host();
        host.render().expect("render both windows");
        let generation = host.device_generation;

        host.injected_rebuild_failures = 1;
        host.surface_lost().expect("mark the synthetic loss");
        host.render()
            .expect("the next window action retries the failed attempt");

        assert!(host.latched_loss.is_none());
        assert_eq!(host.recovery_attempts, 0);
        assert_eq!(host.device_generation, generation + 1);
        assert_eq!(host.loss_phase, LossPhase::Clear);
        assert!(host.windows.values().all(|window| window.surface.is_some()));
    }

    #[test]
    fn adapter_info_reports_the_active_adapter() {
        let host = open_host();
        let info = host.adapter_info();
        let raw = host.adapter.get_info();
        assert!(!info.name.is_empty(), "the adapter has a name");
        assert_eq!(info.name, raw.name);
        assert_eq!(info.backend, format!("{:?}", raw.backend));
        assert_eq!(info.device_type, format!("{:?}", raw.device_type));
        assert_eq!(info.driver, raw.driver);
        assert_eq!(info.driver_info, raw.driver_info);
        assert_eq!(
            info.fallback,
            raw.device_type == wgpu::DeviceType::Cpu,
            "software fallback is observable"
        );
    }

    #[test]
    fn synthetic_lost_rebuilds_once_and_keeps_hidden_windows_hidden() {
        let mut host = open_two_window_host();
        let second = host.windows.get("second").expect("second").ctx;
        assert!(host.runtime.window_visible(second));
        let generation = host.device_generation;
        assert!(
            host.windows.values().all(|window| !window.native.shown),
            "the hidden host starts with no shown native window"
        );

        host.surface_lost().expect("synthetic Lost status");
        assert_eq!(host.loss_phase, LossPhase::Pending);
        host.recover_device().expect("rebuild every surface");

        assert_eq!(host.device_generation, generation + 1);
        assert!(!host.show_windows, "hidden mode stays hidden");
        assert!(
            host.windows.values().all(|window| !window.native.shown),
            "recovery does not show a hidden native window"
        );
        assert!(host.runtime.window_visible(second), "visibility survives");
        assert!(
            host.windows.values().all(|window| window.surface.is_some()),
            "every live window has a new surface"
        );

        host.recover_device().expect("a second call is a no-op");
        assert_eq!(host.device_generation, generation + 1);
    }

    #[test]
    fn a_zero_window_loss_rebuilds_the_device_before_reopening() {
        let mut host = open_host();
        host.close(MAIN).expect("close the last window");
        assert!(host.windows.is_empty());
        let generation = host.device_generation;

        host.surface_lost().expect("mark loss");
        host.recover_device()
            .expect("replace the device without a window");
        assert_eq!(host.device_generation, generation + 1);
        assert_eq!(host.loss_phase, LossPhase::AwaitingPresent);
        assert!(host.loss.is_lost(), "no present has proved the device yet");

        host.show(MAIN)
            .expect("reopen the window on the new device");
        assert!(
            host.windows.values().all(|window| window.surface.is_some()),
            "the reopened window gets a surface"
        );
        assert_eq!(
            host.loss_phase,
            LossPhase::Clear,
            "the first present clears the loss"
        );
        assert!(!host.loss.is_lost());
    }

    #[test]
    fn a_callback_loss_without_a_target_rebuilds_before_reopening() {
        let mut host = open_host();
        host.close(MAIN).expect("close the only window");
        assert!(host.windows.is_empty());
        let generation = host.device_generation;

        host.device.destroy();
        let _ = host.device.poll(wgpu::PollType::wait_indefinitely());
        assert!(host.loss.is_lost(), "the callback sets the loss signal");
        assert_eq!(
            host.loss_phase,
            LossPhase::Clear,
            "no target can dispatch the pending message yet"
        );

        host.show(MAIN).expect("reopen the window");
        assert_eq!(
            host.device_generation,
            generation + 1,
            "the reopen replaces the lost device before creating a surface"
        );
        assert_eq!(host.loss_phase, LossPhase::Clear);
        assert!(!host.loss.is_lost());
        assert!(host.windows.values().all(|window| window.surface.is_some()));
    }

    #[test]
    fn a_second_loss_after_recovery_rebuilds_before_reopening() {
        let mut host = open_host();
        host.close(MAIN).expect("close the only window");
        host.surface_lost().expect("mark loss");
        host.recover_device()
            .expect("replace the device without a window");
        assert_eq!(host.loss_phase, LossPhase::AwaitingPresent);
        let generation = host.device_generation;

        host.device.destroy();
        let _ = host.device.poll(wgpu::PollType::wait_indefinitely());
        assert!(host.loss.is_lost(), "the rebuilt device reports a loss");
        assert_eq!(
            host.loss_phase,
            LossPhase::AwaitingPresent,
            "no target promotes the phase before a show"
        );

        host.show(MAIN).expect("reopen the window");
        assert_eq!(
            host.device_generation,
            generation + 1,
            "the second loss is repaired before the surface is created"
        );
        assert_eq!(host.loss_phase, LossPhase::Clear);
        assert!(!host.loss.is_lost());
        assert!(host.windows.values().all(|window| window.surface.is_some()));
    }

    #[test]
    fn validation_is_loss_only_with_the_current_signal() {
        let mut host = open_host();
        let error = host
            .validation_status()
            .expect_err("validation without a loss signal is fatal");
        assert!(matches!(error, HostError::Surface(_)));

        host.loss.mark_lost();
        host.validation_status()
            .expect("signal-set validation is treated as loss");
        assert_eq!(host.loss_phase, LossPhase::Pending);
        assert!(host.loss.is_lost());
        let generation = host.device_generation;
        host.recover_device()
            .expect("rebuild after the validation loss");
        assert_eq!(host.device_generation, generation + 1);
    }

    #[test]
    fn an_old_device_callback_cannot_trigger_a_second_rebuild() {
        let mut host = open_host();
        let old_generation = host.device_generation;

        host.surface_lost().expect("mark loss");
        host.recover_device().expect("recover");
        let new_generation = host.device_generation;
        assert_eq!(new_generation, old_generation + 1);
        assert_eq!(host.loss_phase, LossPhase::Clear);
        assert!(!host.loss.is_lost());
        let phase = host.loss_phase;

        host.loss.raise(old_generation);
        assert!(
            !host.loss.is_lost(),
            "a stale callback does not set the signal"
        );
        let hwnd = host.hwnd(MAIN).expect("a live window");
        host.handle_message(
            hwnd,
            host_message::DEVICE_LOSS,
            WPARAM(old_generation as usize),
            LPARAM(0),
        );
        assert_eq!(host.device_generation, new_generation);
        assert_eq!(host.loss_phase, phase, "a stale message changes nothing");
        assert!(!host.loss.is_lost());
    }

    #[test]
    fn recovery_keeps_worker_delivery_alive() {
        let (mut host, completed) = open_delivering_task_host();
        host.runtime
            .click_text("Start task")
            .expect("spawn a window-owned task");
        host.runtime.wait_for_tasks();

        host.surface_lost().expect("mark loss");
        host.recover_device().expect("recover");
        assert_eq!(
            completed.load(Ordering::SeqCst),
            0,
            "the result waits for the host to deliver it"
        );

        let hwnd = host.hwnd(MAIN).expect("a live window");
        host.handle_message(hwnd, host_message::WAKE, WPARAM(0), LPARAM(0));

        assert_eq!(
            completed.load(Ordering::SeqCst),
            1,
            "the host delivers the worker result after recovery"
        );
        assert!(!host.wake.is_pending(), "delivery observes the wake");
        assert!(!host.show_windows, "the hidden window stays hidden");
        assert!(
            host.windows.values().all(|window| !window.native.shown),
            "recovery does not show the hidden native window"
        );
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

    fn open_host() -> Host {
        let mut runtime = Runtime::new();
        zenui_demo::open_todo_window(&mut runtime).expect("open the todo window");
        zenui_demo::seed(&mut runtime).expect("seed the todo rows");
        Host::open(runtime, HostConfig::hidden()).expect("open host")
    }

    #[test]
    fn sampling_system_preferences_returns_a_valid_scale() {
        let preferences = sample_system_preferences();
        assert!(
            (100..=225).contains(&preferences.text_scale_percent()),
            "the sampled scale is always in the supported range"
        );
    }

    #[test]
    fn high_contrast_overrides_the_application_appearance() {
        let colors = HighContrastColors {
            window: [1, 2, 3, 255],
            window_text: [4, 5, 6, 255],
            button_face: [7, 8, 9, 255],
            button_highlight: [10, 11, 12, 255],
            button_shadow: [13, 14, 15, 255],
            gray_text: [16, 17, 18, 255],
            highlight: [19, 20, 21, 255],
            highlight_text: [22, 23, 24, 255],
            window_frame: [25, 26, 27, 255],
        };
        assert_eq!(
            palette_for(Appearance::Dark, SystemPreferences::new(true, 100), || {
                colors
            }),
            RenderPalette::high_contrast(colors),
            "high contrast overrides the dark appearance"
        );
        assert_eq!(
            palette_for(Appearance::Dark, SystemPreferences::new(false, 100), || {
                unreachable!("normal appearance does not sample system colors")
            }),
            RenderPalette::dark()
        );
        assert_eq!(
            palette_for(
                Appearance::Light,
                SystemPreferences::new(false, 100),
                || { unreachable!("normal appearance does not sample system colors") }
            ),
            RenderPalette::light()
        );
    }

    #[test]
    fn system_preferences_fall_back_to_100_percent_outside_the_supported_range() {
        assert_eq!(
            SystemPreferences::new(true, 99),
            SystemPreferences::new(true, 100),
            "below the range falls back to 100%"
        );
        assert_eq!(
            SystemPreferences::new(true, 226),
            SystemPreferences::new(true, 100),
            "above the range falls back to 100%"
        );
        let preferences = SystemPreferences::new(true, 225);
        assert!(preferences.high_contrast());
        assert_eq!(preferences.text_scale_percent(), 225);
        assert_eq!(
            SystemPreferences::default(),
            SystemPreferences::new(false, 100)
        );
    }

    #[test]
    fn injected_system_preferences_repaint_without_a_model_transaction() {
        let mut runtime = Runtime::new();
        zenui_demo::open_todo_window(&mut runtime).expect("open the todo window");
        let mut host = Host::open(
            runtime,
            HostConfig::hidden().with_system_preferences(SystemPreferences::new(true, 150)),
        )
        .expect("open host");
        assert_eq!(
            host.system_preferences(),
            SystemPreferences::new(true, 150),
            "the injected startup value replaces sampling"
        );
        host.windows.get_mut(MAIN).expect("window").paint_requested = false;
        let revision = host.runtime.presentation_revision();
        host.set_system_preferences(SystemPreferences::new(false, 225));
        assert_eq!(
            host.system_preferences(),
            SystemPreferences::new(false, 225)
        );
        assert!(
            host.windows.get(MAIN).expect("window").paint_requested,
            "a preference change requests a repaint"
        );
        assert_eq!(
            host.runtime.presentation_revision(),
            revision,
            "preferences do not enter the presentation snapshot"
        );
    }

    #[test]
    fn a_settings_broadcast_resamples_and_repaints_only_on_change() {
        let mut host = open_host();
        let current = host.system_preferences();
        host.windows.get_mut(MAIN).expect("window").paint_requested = false;
        host.settings_sample = Some(current);
        host.with_window(MAIN, |host, window| {
            host.handle_window_message(window, WM_SETTINGCHANGE, WPARAM(0), LPARAM(0))
        })
        .expect("unchanged settings");
        assert!(
            !host.windows.get(MAIN).expect("window").paint_requested,
            "an unchanged sample does not request a repaint"
        );

        host.settings_sample = Some(SystemPreferences::new(true, 200));
        let revision = host.runtime.presentation_revision();
        host.with_window(MAIN, |host, window| {
            host.handle_window_message(window, WM_SETTINGCHANGE, WPARAM(0), LPARAM(0))
        })
        .expect("changed settings");
        assert_eq!(host.system_preferences(), SystemPreferences::new(true, 200));
        assert!(
            host.windows.get(MAIN).expect("window").paint_requested,
            "a changed sample requests a repaint"
        );
        assert_eq!(
            host.runtime.presentation_revision(),
            revision,
            "a settings change does not touch the presentation snapshot"
        );
    }

    #[test]
    fn a_settings_broadcast_applies_a_new_text_scale() {
        let mut host = open_host();
        host.settings_sample = Some(SystemPreferences::new(false, 225));
        host.with_window(MAIN, |host, window| {
            host.handle_window_message(window, WM_SETTINGCHANGE, WPARAM(0), LPARAM(0))
        })
        .expect("settings change");
        assert_eq!(
            host.system_preferences().text_scale_percent(),
            225,
            "the broadcast samples the injected scale"
        );
        assert!(
            host.windows.get(MAIN).expect("window").paint_requested,
            "the broadcast requests a repaint"
        );
        host.render().expect("render at the new scale");
        let node = host
            .runtime
            .presentation()
            .expect("snapshot")
            .find_label("What needs to be done?")
            .expect("draft")
            .id();
        assert!(
            (host
                .scene(MAIN)
                .expect("scene")
                .node(node)
                .expect("draft geometry")
                .resolved_style()
                .font_size()
                - 31.5)
                .abs()
                < 0.01,
            "the render uses the sampled scale"
        );
    }

    #[test]
    fn a_text_scale_change_rebuilds_text_and_preserves_edit_state() {
        let mut host = open_host();
        host.runtime
            .focus("What needs to be done?")
            .expect("focus the draft");
        host.runtime
            .input("What needs to be done?", "Scaled draft")
            .expect("type the draft");
        host.render().expect("render at 100%");
        let snapshot = host.runtime.presentation().expect("snapshot");
        let node = snapshot
            .find_label("What needs to be done?")
            .expect("draft")
            .id();
        host.runtime
            .set_caret_node(node, 3)
            .expect("place the caret");
        host.render().expect("render the caret");
        let (base_size, base_height, base_caret) = {
            let scene = host.scene(MAIN).expect("base scene");
            let base = scene.node(node).expect("base draft");
            (
                base.resolved_style().font_size(),
                base.rect().height,
                scene.caret(node).expect("base caret"),
            )
        };
        let revision = host.runtime.presentation_revision();

        host.set_system_preferences(SystemPreferences::new(false, 225));
        host.render().expect("render at 225%");
        let scene = host.scene(MAIN).expect("scaled scene");
        let scaled = scene.node(node).expect("scaled draft");
        assert!((base_size - 14.0).abs() < 0.01);
        assert!(
            (scaled.resolved_style().font_size() - 31.5).abs() < 0.01,
            "225% scales the draft text"
        );
        assert!(scaled.rect().height > base_height);
        let caret = scene.caret(node).expect("scaled caret");
        assert!(caret.height > base_caret.height);
        assert_eq!(
            host.runtime.presentation().expect("snapshot").focus(),
            Some(node),
            "focus survives the rebuild"
        );
        assert_eq!(
            host.runtime.presentation_revision(),
            revision,
            "text scale does not enter the presentation snapshot"
        );
        assert_eq!(
            host.runtime
                .presentation()
                .expect("snapshot")
                .node(node)
                .and_then(|element| element.text_value()),
            Some("Scaled draft"),
            "the local edit value survives without on:input"
        );
    }

    #[test]
    fn a_text_scale_change_reclamps_the_scroll_offset() {
        let mut runtime = Runtime::new();
        zenui_demo::open_todo_window(&mut runtime).expect("open the todo window");
        for index in 0..40 {
            runtime
                .input("What needs to be done?", &format!("Task {index}"))
                .expect("fill the draft");
            runtime.submit("Add task").expect("add a row");
        }
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open a tall host");
        host.set_system_preferences(SystemPreferences::new(false, 225));
        host.render().expect("render at 225%");
        host.windows.get_mut(MAIN).expect("window").scroll_offset = 100_000;
        host.render().expect("clamp at 225%");
        let tall_max = host.scene(MAIN).expect("scene").max_scroll_offset();
        assert!(tall_max > 400);
        assert_eq!(
            host.windows[MAIN].scroll_offset, tall_max,
            "the offset clamps to the scaled extent"
        );

        host.set_system_preferences(SystemPreferences::new(false, 100));
        host.render().expect("render at 100%");
        let short_max = host.scene(MAIN).expect("scene").max_scroll_offset();
        assert!(
            short_max < tall_max,
            "the smaller text has a smaller content extent"
        );
        assert_eq!(
            host.windows[MAIN].scroll_offset, short_max,
            "the stored offset is re-clamped to the smaller extent"
        );
    }

    /// 测试期记录器：`&'static` 是 Copy，事件闭包重复构造时不会把 view 的
    /// factory 变成 `FnOnce`；每个探针实例各自泄漏一块极小的状态。
    fn leak<T>(value: T) -> &'static T {
        Box::leak(Box::new(value))
    }

    /// UIA 动作探针：每个 handler 记录自己的调用，便于断言恰好一次与顺序。
    struct UiaProbe {
        clicks: &'static AtomicUsize,
        events: &'static Mutex<Vec<&'static str>>,
        changes: &'static Mutex<Vec<bool>>,
        inputs: &'static Mutex<Vec<String>>,
    }

    fn open_uia_probe() -> (Host, UiaProbe) {
        let clicks = leak(AtomicUsize::new(0));
        let events = leak(Mutex::new(Vec::new()));
        let changes = leak(Mutex::new(Vec::new()));
        let inputs = leak(Mutex::new(Vec::new()));

        let mut runtime = Runtime::new();
        runtime
            .declare_windows(move |catalog| {
                catalog.window(WindowSpec::new(MAIN).primary().sized(480, 360), move |_| {
                    // 信号在挂载上下文中创建；测试通过快照与事件日志观察结果。
                    let mounted = signal(true);
                    let locked = signal(false);
                    let checked = signal(false);
                    let value = signal(String::from("old"));
                    view! {
                        <view>
                            <button
                                aria:label="Act"
                                disabled={locked}
                                on:click={move || {
                                    clicks.fetch_add(1, Ordering::SeqCst);
                                    events.lock().expect("events").push("click");
                                }}
                            >"Act"</button>
                            <button
                                aria:label="Disable"
                                on:click={move || {
                                    events.lock().expect("events").push("disable");
                                    locked.set(true);
                                }}
                            >"Disable"</button>
                            <button
                                aria:label="Hide"
                                on:click={move || mounted.set(false)}
                            >"Hide"</button>
                            <Show when={memo(move || mounted.get())}>
                                <button
                                    aria:label="Ephemeral"
                                    on:click={move || {
                                        clicks.fetch_add(1, Ordering::SeqCst);
                                    }}
                                >"Ephemeral"</button>
                            </Show>
                            <text aria:label="Caption">"Caption"</text>
                            <checkbox
                                aria:label="Flag"
                                checked={checked}
                                on:change={move |next: bool| {
                                    events.lock().expect("events").push("change");
                                    changes.lock().expect("changes").push(next);
                                    checked.set(next);
                                }}
                            />
                            <text-input
                                aria:label="Field"
                                value={value}
                                on:input={move |next: String| {
                                    events.lock().expect("events").push("input");
                                    inputs.lock().expect("inputs").push(next.clone());
                                    value.set(next);
                                }}
                            />
                        </view>
                    }
                })?;
                Ok(())
            })
            .expect("declare the probe window");
        runtime.show(MAIN).expect("show the probe window");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open the probe host");
        host.render().expect("render the probe window");
        (
            host,
            UiaProbe {
                clicks,
                events,
                changes,
                inputs,
            },
        )
    }

    /// 静态禁用的按钮与输入：任何误派发的 handler 都会让测试失败。
    fn open_disabled_probe() -> Host {
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(|catalog| {
                catalog.window(WindowSpec::new(MAIN).primary().sized(320, 240), |_| {
                    view! {
                        <view>
                            <button
                                aria:label="Dead"
                                disabled={true}
                                on:click={move || -> () {
                                    panic!("a disabled Invoke ran its handler")
                                }}
                            >"Dead"</button>
                            <text-input
                                aria:label="Locked"
                                value="frozen"
                                disabled={true}
                                on:input={move |_value: String| -> () {
                                    panic!("a disabled SetValue ran its handler")
                                }}
                            />
                        </view>
                    }
                })?;
                Ok(())
            })
            .expect("declare the disabled probe window");
        runtime.show(MAIN).expect("show the disabled probe window");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open the disabled host");
        host.render().expect("render the disabled probe window");
        host
    }

    /// 足够长的 Todo 列表：滚动到底部后草稿输入完全离开视口。
    fn open_long_text_host() -> Host {
        let mut runtime = Runtime::new();
        zenui_demo::open_todo_window(&mut runtime).expect("open the todo window");
        for index in 0..80 {
            runtime
                .input("What needs to be done?", &format!("Task {index}"))
                .expect("fill the draft");
            runtime.submit("Add task").expect("add a row");
        }
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open a long host");
        host.render().expect("render the long host");
        host
    }

    /// 没有 on:input 的静态文本输入：可读但不可选择。
    fn open_read_only_probe() -> Host {
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(|catalog| {
                catalog.window(WindowSpec::new(MAIN).primary().sized(320, 240), |_| {
                    view! {
                        <view>
                            <text-input aria:label="Static" value="static text" />
                        </view>
                    }
                })?;
                Ok(())
            })
            .expect("declare the read-only probe window");
        runtime.show(MAIN).expect("show the read-only probe window");
        let mut host =
            Host::open(runtime, HostConfig::hidden()).expect("open the read-only probe host");
        host.render().expect("render the read-only probe window");
        host
    }

    fn root_fragment(host: &mut Host) -> IRawElementProviderFragment {
        let hwnd = host.hwnd(MAIN).expect("a live window");
        host.accessibility_provider(hwnd)
            .expect("a root provider")
            .cast()
            .expect("fragment root")
    }

    fn scroll_pattern(host: &mut Host) -> IScrollProvider {
        let root = root_fragment(host);
        let simple: IRawElementProviderSimple = root.cast().expect("simple root provider");
        pattern(&simple, UIA_ScrollPatternId)
    }

    fn provider_name(simple: &IRawElementProviderSimple) -> String {
        let variant = unsafe { simple.GetPropertyValue(UIA_NamePropertyId) }.expect("element name");
        // 安全：provider 返回的 VT_BSTR 由这个 VARIANT 持有。
        unsafe { &variant.Anonymous.Anonymous.Anonymous.bstrVal }.to_string()
    }

    /// 从根开始按 UIA 导航查找名称匹配的元素 provider。
    fn provider_named(root: &IRawElementProviderFragment, name: &str) -> IRawElementProviderSimple {
        fn walk(
            fragment: &IRawElementProviderFragment,
            name: &str,
        ) -> Option<IRawElementProviderSimple> {
            let mut child = unsafe { fragment.Navigate(NavigateDirection_FirstChild) }.ok();
            while let Some(child_fragment) = child {
                let simple: IRawElementProviderSimple =
                    child_fragment.cast().expect("simple provider");
                if provider_name(&simple) == name {
                    return Some(simple);
                }
                if let Some(found) = walk(&child_fragment, name) {
                    return Some(found);
                }
                child = unsafe { child_fragment.Navigate(NavigateDirection_NextSibling) }.ok();
            }
            None
        }
        walk(root, name).expect("a provider with the requested name")
    }

    /// 取出 VT_UNKNOWN SAFEARRAY 的一个元素并销毁数组。
    ///
    /// `SafeArrayGetElement` 对接口元素调用 AddRef，取出的范围持有自己的引用。
    fn text_range_at(array: *mut SAFEARRAY, index: i32) -> ITextRangeProvider {
        let mut raw: *mut c_void = std::ptr::null_mut();
        // 安全：raw 是本地输出缓冲；数组由本次调用方的 provider 创建。
        unsafe {
            SafeArrayGetElement(array, &index, (&mut raw as *mut *mut c_void).cast())
                .expect("a range element");
            let range = ITextRangeProvider::from_raw(raw);
            let _ = SafeArrayDestroy(array);
            range
        }
    }

    fn range_text(range: &ITextRangeProvider) -> String {
        unsafe { range.GetText(-1) }
            .expect("range text")
            .to_string()
    }

    /// 复制 VT_R8 SAFEARRAY 的矩形值并销毁数组。
    fn rect_values(range: &ITextRangeProvider) -> Vec<f64> {
        let array = unsafe { range.GetBoundingRectangles() }.expect("rectangle array");
        assert!(
            !array.is_null(),
            "GetBoundingRectangles returns an empty array, not NULL"
        );
        let lower = unsafe { SafeArrayGetLBound(array, 1) }.expect("lower bound");
        let upper = unsafe { SafeArrayGetUBound(array, 1) }.expect("upper bound");
        let mut values = Vec::with_capacity((upper - lower + 1).max(0) as usize);
        for index in lower..=upper {
            let mut value = 0.0f64;
            // 安全：value 是本地输出缓冲；索引已由数组边界确定。
            unsafe { SafeArrayGetElement(array, &index, (&mut value as *mut f64).cast()) }
                .expect("a rectangle value");
            values.push(value);
        }
        let _ = unsafe { SafeArrayDestroy(array) };
        values
    }

    /// 范围矩形等于场景矩形按客户端原点和窗口 scale 翻译后的物理值。
    fn assert_range_rectangles(host: &Host, range: &ITextRangeProvider, expected: &[Rect]) {
        let values = rect_values(range);
        assert_eq!(values.len(), expected.len() * 4, "one quad per rectangle");
        let hwnd = host.hwnd(MAIN).expect("a live window");
        let mut origin = POINT { x: 0, y: 0 };
        // 安全：origin 是本地输出缓冲；句柄由测试宿主持有。
        assert!(
            unsafe { ClientToScreen(hwnd, &mut origin) }.as_bool(),
            "client origin"
        );
        let scale = f64::from(host.accessibility_tree(MAIN).expect("tree").scale());
        for (actual, rect) in values.chunks(4).zip(expected) {
            assert_close(actual[0], f64::from(origin.x) + f64::from(rect.x) * scale);
            assert_close(actual[1], f64::from(origin.y) + f64::from(rect.y) * scale);
            assert_close(actual[2], f64::from(rect.width) * scale);
            assert_close(actual[3], f64::from(rect.height) * scale);
        }
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-6,
            "expected {expected}, got {actual}"
        );
    }

    fn assert_not_supported(error: windows::core::Error) {
        assert_eq!(error.code(), HRESULT(UIA_E_NOTSUPPORTED as i32));
    }

    fn assert_invalid_operation(error: windows::core::Error) {
        assert_eq!(error.code(), HRESULT(UIA_E_INVALIDOPERATION as i32));
    }

    fn pattern<T: Interface>(
        simple: &IRawElementProviderSimple,
        id: windows::Win32::UI::Accessibility::UIA_PATTERN_ID,
    ) -> T {
        unsafe { simple.GetPatternProvider(id) }
            .expect("a pattern provider")
            .cast()
            .expect("the requested pattern interface")
    }

    /// 一个 Confirm 主窗口与它声明的确认窗口；确认窗口用按钮回复。
    fn open_confirm_host() -> Host {
        const CONFIRM: &str = "confirm";
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(|catalog| {
                catalog.window(
                    WindowSpec::new(MAIN)
                        .primary()
                        .close_policy(ClosePolicy::Confirm),
                    |window| {
                        window
                            .on_close(move |_| {
                                window.request_show(CONFIRM).expect("show the confirmation");
                                CloseOutcome::Pending
                            })
                            .expect("register the close handler");
                        let draft = signal(String::new());
                        view! {
                            <view>
                                <text-input
                                    aria:label="draft"
                                    value={draft}
                                    on:input=move |value: String| draft.set(value)
                                />
                            </view>
                        }
                    },
                )?;
                catalog.window(WindowSpec::new(CONFIRM).confirmation_for(MAIN), |window| {
                    let request = window
                        .close_request()
                        .expect("read the close request")
                        .expect("a pending epoch");
                    view! {
                        <view>
                            <button
                                aria:label="Allow"
                                on:click=move || request.allow().expect("allow")
                            >"Allow"</button>
                            <button
                                aria:label="Deny"
                                on:click=move || request.deny().expect("deny")
                            >"Deny"</button>
                        </view>
                    }
                })?;
                Ok(())
            })
            .expect("declare the confirmation catalog");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open the host");
        host.render().expect("render the primary window");
        host
    }

    fn click_label_in(host: &mut Host, key: &'static str, label: &str) {
        let ctx = host.windows[key].ctx;
        let node = host
            .runtime
            .presentation_of(ctx)
            .expect("window snapshot")
            .find_label(label)
            .expect("labelled element")
            .id();
        let scene = host.scene(key).expect("a rendered scene");
        let rect = scene.node(node).expect("element").rect();
        let scale = scene.viewport().scale();
        let x = ((rect.x + rect.width / 2.0) * scale).round() as isize;
        let y = ((rect.y + rect.height / 2.0) * scale).round() as isize;
        let lparam = LPARAM((x & 0xffff) | ((y & 0xffff) << 16));
        for message in [WM_LBUTTONDOWN, WM_LBUTTONUP] {
            send_msg_in(
                host,
                key,
                MSG {
                    message,
                    wParam: WPARAM(0),
                    lParam: lparam,
                    ..Default::default()
                },
            );
        }
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
        if std::mem::take(&mut host.focus_after_sync) {
            host.focus_native_focused_window()
                .expect("focus the active window");
        }
        host.repaint_changed_windows().expect("repaint");
        if let Some(error) = host.deferred_error.take() {
            panic!("the window procedure failed: {error}");
        }
    }

    /// 一次 fake 上下文菜单打开收到的请求快照。
    #[derive(Clone, Debug, PartialEq)]
    struct ContextMenuRecord {
        window: WindowCtx,
        hwnd: HWND,
        point: (i32, i32),
        menu: MenuSpec,
    }

    /// 在“弹窗期间”重入窗口过程的动作。
    type DuringShow = Box<dyn FnMut(HWND, &MenuSpec)>;

    /// 记录请求并返回固定命令的 presenter；可选动作在“弹窗期间”重入窗口过程。
    struct FakeContextMenuPresenter {
        records: Rc<RefCell<Vec<ContextMenuRecord>>>,
        result: Option<&'static str>,
        panic_first: Rc<Cell<bool>>,
        during_show: Option<DuringShow>,
    }

    impl ContextMenuPresenter for FakeContextMenuPresenter {
        fn show(&mut self, request: ContextMenuRequest) -> Result<Option<&'static str>, HostError> {
            if self.panic_first.replace(false) {
                panic!("the fake context menu presenter failed");
            }
            self.records.borrow_mut().push(ContextMenuRecord {
                window: request.window(),
                hwnd: request.hwnd(),
                point: request.point(),
                menu: request.menu().clone(),
            });
            if let Some(action) = self.during_show.as_mut() {
                action(request.hwnd(), request.menu());
            }
            Ok(self.result)
        }
    }

    fn context_menu_declaration() -> MenuSpec {
        MenuSpec::new()
            .command("edit.copy", "Copy")
            .separator()
            .submenu("More", MenuSpec::new().command("edit.paste", "Paste"))
    }

    /// fake presenter 安装后的隐藏宿主与观察点。
    struct ContextMenuHarness {
        host: Host,
        records: Rc<RefCell<Vec<ContextMenuRecord>>>,
        runs: Rc<Cell<u32>>,
    }

    /// 带上下文菜单声明的隐藏宿主；`edit.copy` 的处理器记录派发次数。
    fn context_menu_host(
        result: Option<&'static str>,
        during_show: Option<DuringShow>,
        panic_first: bool,
    ) -> ContextMenuHarness {
        let runs = Rc::new(Cell::new(0));
        let handler_runs = runs.clone();
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(move |catalog| {
                catalog.window(WindowSpec::new(MAIN).primary(), move |window| {
                    let handler_runs = handler_runs.clone();
                    window
                        .on_command("edit.copy", move |_| {
                            handler_runs.set(handler_runs.get() + 1);
                        })
                        .expect("register edit.copy");
                    window
                        .on_command("edit.paste", |_| {})
                        .expect("register edit.paste");
                    view! {
                        <view>
                            <text aria:label="status">"Ready"</text>
                        </view>
                    }
                })?;
                catalog.context_menu(MAIN, context_menu_declaration())?;
                Ok(())
            })
            .expect("declare the context menu host");
        let mut host =
            Host::open(runtime, HostConfig::hidden()).expect("open the context menu host");
        let records = Rc::new(RefCell::new(Vec::new()));
        host.set_context_menu_presenter(FakeContextMenuPresenter {
            records: records.clone(),
            result,
            panic_first: Rc::new(Cell::new(panic_first)),
            during_show,
        });
        ContextMenuHarness {
            host,
            records,
            runs,
        }
    }

    /// 同步派发 `WM_CONTEXTMENU`；`None` 是键盘调用的 (-1,-1)。
    fn send_context_menu(host: &mut Host, point: Option<(i16, i16)>) -> Option<HostError> {
        let hwnd = host.hwnd(MAIN).expect("a live window");
        let (x, y) = point.unwrap_or((-1, -1));
        let lparam = LPARAM((((y as u16 as u32) << 16) | (x as u16 as u32)) as isize);
        {
            let _pointer = HostPointer::install(host as *mut Host);
            // 安全：SendMessageW 在调用线程同步派发；调用期间不使用 host。
            unsafe {
                SendMessageW(hwnd, WM_CONTEXTMENU, Some(WPARAM(0)), Some(lparam));
            }
        }
        host.sync_windows().expect("sync windows");
        if std::mem::take(&mut host.focus_after_sync) {
            host.focus_native_focused_window()
                .expect("focus the active window");
        }
        host.repaint_changed_windows().expect("repaint");
        host.deferred_error.take()
    }

    /// 键盘打开时客户区中心的屏幕坐标。
    fn client_center(hwnd: HWND) -> (i32, i32) {
        let mut rect = RECT::default();
        // 安全：rect 是本地输出缓冲；hwnd 由宿主持有。
        unsafe { GetClientRect(hwnd, &mut rect) }.expect("client rect");
        let mut point = POINT {
            x: (rect.right - rect.left) / 2,
            y: (rect.bottom - rect.top) / 2,
        };
        // 安全：客户区坐标转屏幕坐标；hwnd 由宿主持有。
        assert!(unsafe { ClientToScreen(hwnd, &mut point) }.as_bool());
        (point.x, point.y)
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
            host.char_typed(window, WPARAM('X' as usize), LPARAM(0))
        });
        let snapshot = host.runtime.presentation().expect("snapshot");
        let element = snapshot.node(node).expect("editor");
        assert_eq!(element.text_value(), Some("X"));
        let edit = element.edit().expect("edit");
        assert_eq!(edit.caret(), 1);
        assert_eq!(edit.selection(), None, "typing collapses the selection");
    }

    /// 撤销/重做探针：一个记录每次 `on:input` 的输入，以及一个不抢焦点的禁用按钮。
    fn open_history_probe() -> (Host, &'static Mutex<Vec<String>>) {
        let inputs = leak(Mutex::new(Vec::new()));
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(move |catalog| {
                catalog.window(WindowSpec::new(MAIN).primary().sized(320, 240), move |_| {
                    let value = signal(String::from("old"));
                    let locked = signal(false);
                    view! {
                        <view>
                            <button
                                aria:label="Lock"
                                on:click={move || locked.set(true)}
                            >"Lock"</button>
                            <text-input
                                aria:label="Field"
                                value={value}
                                disabled={locked}
                                on:input={move |next: String| {
                                    inputs.lock().expect("inputs").push(next.clone());
                                    value.set(next);
                                }}
                            />
                        </view>
                    }
                })?;
                Ok(())
            })
            .expect("declare the history probe window");
        runtime.show(MAIN).expect("show the history probe window");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open the history host");
        host.render().expect("render the history probe window");
        (host, inputs)
    }

    fn history_node(host: &Host) -> NodeId {
        host.runtime
            .presentation()
            .expect("snapshot")
            .find_label("Field")
            .expect("field")
            .id()
    }

    /// 内存剪贴板探针：记录读取次数与写入内容，供宿主路由测试注入。
    #[derive(Clone, Default)]
    struct ClipboardProbe(Rc<RefCell<ClipboardProbeState>>);

    #[derive(Default)]
    struct ClipboardProbeState {
        text: Option<String>,
        reads: usize,
        writes: Vec<String>,
        fail_read: bool,
        fail_write: bool,
    }

    impl ClipboardProbe {
        fn install(host: &mut Host) -> Self {
            let probe = Self::default();
            host.set_clipboard_provider(MemoryClipboard(probe.clone()));
            probe
        }

        fn set_text(&self, text: Option<&str>) {
            self.0.borrow_mut().text = text.map(str::to_owned);
        }

        fn text(&self) -> Option<String> {
            self.0.borrow().text.clone()
        }

        fn reads(&self) -> usize {
            self.0.borrow().reads
        }

        fn writes(&self) -> Vec<String> {
            self.0.borrow().writes.clone()
        }

        fn fail_reads(&self) {
            self.0.borrow_mut().fail_read = true;
        }

        fn fail_writes(&self) {
            self.0.borrow_mut().fail_write = true;
        }
    }

    struct MemoryClipboard(ClipboardProbe);

    impl ClipboardProvider for MemoryClipboard {
        fn read_text(&mut self) -> Result<Option<String>, ClipboardError> {
            let mut state = self.0.0.borrow_mut();
            state.reads += 1;
            if state.fail_read {
                return Err(ClipboardError::new("clipboard read failed"));
            }
            Ok(state.text.clone())
        }

        fn write_text(&mut self, text: &str) -> Result<(), ClipboardError> {
            let mut state = self.0.0.borrow_mut();
            state.writes.push(text.to_owned());
            if state.fail_write {
                return Err(ClipboardError::new("clipboard write failed"));
            }
            state.text = Some(text.to_owned());
            Ok(())
        }
    }

    #[test]
    fn ctrl_z_undoes_and_ctrl_y_redoes_value_caret_and_selection_once() {
        let (mut host, inputs) = open_history_probe();
        host.runtime.focus("Field").expect("focus");
        let node = history_node(&host);
        host.runtime
            .set_selection_node(node, 1, 3)
            .expect("select the middle");
        with_main(&mut host, |host, window| {
            host.char_typed(window, WPARAM('X' as usize), LPARAM(0))
        });
        {
            let snapshot = host.runtime.presentation().expect("snapshot");
            let element = snapshot.node(node).expect("field");
            assert_eq!(element.text_value(), Some("oX"));
            let edit = element.edit().expect("edit");
            assert_eq!((edit.anchor(), edit.caret()), (2, 2));
            assert_eq!(edit.selection(), None, "typing collapses the selection");
        }
        assert_eq!(*inputs.lock().expect("inputs"), ["oX"]);

        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_Z, false, true)
        });
        {
            let snapshot = host.runtime.presentation().expect("snapshot");
            let element = snapshot.node(node).expect("field");
            assert_eq!(element.text_value(), Some("old"));
            let edit = element.edit().expect("edit");
            assert_eq!((edit.anchor(), edit.caret()), (1, 3));
            assert_eq!(
                edit.selection(),
                Some((1, 3)),
                "undo restores the pre-edit selection"
            );
        }
        assert_eq!(
            *inputs.lock().expect("inputs"),
            ["oX", "old"],
            "one on:input per undo"
        );

        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_Y, false, true)
        });
        {
            let snapshot = host.runtime.presentation().expect("snapshot");
            let element = snapshot.node(node).expect("field");
            assert_eq!(element.text_value(), Some("oX"));
            let edit = element.edit().expect("edit");
            assert_eq!((edit.anchor(), edit.caret()), (2, 2));
            assert_eq!(edit.selection(), None);
        }
        assert_eq!(
            *inputs.lock().expect("inputs"),
            ["oX", "old", "oX"],
            "one on:input per redo"
        );
    }

    #[test]
    fn typing_after_undo_clears_the_redo_stack() {
        let (mut host, inputs) = open_history_probe();
        host.runtime.focus("Field").expect("focus");
        let node = history_node(&host);
        host.runtime.set_caret_node(node, 3).expect("caret at end");
        with_main(&mut host, |host, window| {
            host.char_typed(window, WPARAM('X' as usize), LPARAM(0))
        });
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_Z, false, true)
        });
        with_main(&mut host, |host, window| {
            host.char_typed(window, WPARAM('Y' as usize), LPARAM(0))
        });

        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_Y, false, true)
        });
        let snapshot = host.runtime.presentation().expect("snapshot");
        assert_eq!(
            snapshot.node(node).expect("field").text_value(),
            Some("oldY")
        );
        assert_eq!(
            *inputs.lock().expect("inputs"),
            ["oldX", "old", "oldY"],
            "the stale redo dispatches nothing"
        );
    }

    #[test]
    fn undo_and_redo_without_a_focused_input_are_consumed_no_ops() {
        let (mut host, inputs) = open_history_probe();
        let focus_before = host.runtime.presentation().expect("snapshot").focus();
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_Z, false, true)
        });
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_Y, false, true)
        });
        let snapshot = host.runtime.presentation().expect("snapshot");
        assert_eq!(snapshot.focus(), focus_before, "focus does not move");
        assert_eq!(
            snapshot.find_label("Field").expect("field").text_value(),
            Some("old")
        );
        assert!(inputs.lock().expect("inputs").is_empty());
    }

    #[test]
    fn undo_and_redo_during_composition_are_consumed_no_ops() {
        let (mut host, inputs) = open_history_probe();
        host.runtime.focus("Field").expect("focus");
        let node = history_node(&host);
        host.runtime.compose_start_node(node).expect("start");
        host.runtime
            .compose_update_node(node, "ni")
            .expect("update");

        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_Z, false, true)
        });
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_Y, false, true)
        });
        let snapshot = host.runtime.presentation().expect("snapshot");
        let element = snapshot.node(node).expect("field");
        assert_eq!(element.text_value(), Some("old"));
        assert!(
            element.edit().expect("edit").composition().is_some(),
            "the composition survives"
        );
        assert_eq!(element.edit().expect("edit").preedit(), Some("ni"));
        assert_eq!(snapshot.focus(), Some(node));
        assert!(inputs.lock().expect("inputs").is_empty());
    }

    #[test]
    fn undo_and_redo_after_the_focused_input_is_disabled_are_consumed_no_ops() {
        let (mut host, inputs) = open_history_probe();
        host.runtime.focus("Field").expect("focus");
        let node = history_node(&host);
        let lock = host
            .runtime
            .presentation()
            .expect("snapshot")
            .find_label("Lock")
            .expect("lock")
            .id();
        host.runtime.click_node(lock).expect("disable the field");
        let focus_after_disable = host.runtime.presentation().expect("snapshot").focus();
        assert!(
            host.runtime
                .presentation()
                .expect("snapshot")
                .node(node)
                .expect("field")
                .disabled(),
            "the field is disabled"
        );
        assert_ne!(
            focus_after_disable,
            Some(node),
            "disabling the field moves focus away from it"
        );

        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_Z, false, true)
        });
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_Y, false, true)
        });
        let snapshot = host.runtime.presentation().expect("snapshot");
        assert_eq!(
            snapshot.focus(),
            focus_after_disable,
            "the keys do not move focus"
        );
        assert_eq!(
            snapshot.node(node).expect("field").text_value(),
            Some("old")
        );
        assert!(inputs.lock().expect("inputs").is_empty());
    }

    #[test]
    fn ctrl_c_writes_the_exact_selection_and_dispatches_no_input() {
        let (mut host, inputs) = open_history_probe();
        let clipboard = ClipboardProbe::install(&mut host);
        host.runtime.focus("Field").expect("focus");
        let node = history_node(&host);
        host.runtime
            .set_selection_node(node, 0, 2)
            .expect("select the prefix");
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_C, false, true)
        });
        assert_eq!(clipboard.writes(), ["ol"]);
        assert_eq!(clipboard.text().as_deref(), Some("ol"));
        assert!(inputs.lock().expect("inputs").is_empty());

        host.runtime.set_caret_node(node, 2).expect("collapse");
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_C, false, true)
        });
        assert_eq!(
            clipboard.writes(),
            ["ol"],
            "a collapsed selection copies nothing"
        );

        let lock = host
            .runtime
            .presentation()
            .expect("snapshot")
            .find_label("Lock")
            .expect("lock")
            .id();
        host.runtime.click_node(lock).expect("disable the field");
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_C, false, true)
        });
        assert_eq!(
            clipboard.writes(),
            ["ol"],
            "a disabled field copies nothing"
        );
        assert!(inputs.lock().expect("inputs").is_empty());
    }

    #[test]
    fn ctrl_c_and_ctrl_v_preserve_surrogate_pairs() {
        let (mut host, inputs) = open_history_probe();
        let clipboard = ClipboardProbe::install(&mut host);
        host.runtime.focus("Field").expect("focus");
        let node = history_node(&host);
        host.runtime
            .set_selection_node(node, 0, 3)
            .expect("select all");
        host.runtime
            .insert_text_node(node, "a🙂b")
            .expect("replace the value");
        host.runtime
            .set_selection_node(node, 1, 2)
            .expect("select the emoji");
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_C, false, true)
        });
        assert_eq!(
            clipboard.writes(),
            ["🙂"],
            "copy slices scalars, not UTF-16 units"
        );

        clipboard.set_text(Some("🙂你好"));
        host.runtime
            .set_selection_node(node, 0, 3)
            .expect("select all");
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_V, false, true)
        });
        let snapshot = host.runtime.presentation().expect("snapshot");
        let element = snapshot.node(node).expect("field");
        assert_eq!(element.text_value(), Some("🙂你好"));
        let edit = element.edit().expect("edit");
        assert_eq!((edit.anchor(), edit.caret()), (3, 3));
        assert_eq!(edit.selection(), None);
        assert_eq!(*inputs.lock().expect("inputs"), ["a🙂b", "🙂你好"]);
    }

    #[test]
    fn ctrl_v_inserts_at_the_caret_once_and_undoes_as_one_edit() {
        let (mut host, inputs) = open_history_probe();
        let clipboard = ClipboardProbe::install(&mut host);
        clipboard.set_text(Some("ink"));
        host.runtime.focus("Field").expect("focus");
        let node = history_node(&host);
        host.runtime
            .set_caret_node(node, 1)
            .expect("place the caret");
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_V, false, true)
        });
        {
            let snapshot = host.runtime.presentation().expect("snapshot");
            let element = snapshot.node(node).expect("field");
            assert_eq!(element.text_value(), Some("oinkld"));
            let edit = element.edit().expect("edit");
            assert_eq!((edit.anchor(), edit.caret()), (4, 4));
            assert_eq!(edit.selection(), None);
        }
        assert_eq!(*inputs.lock().expect("inputs"), ["oinkld"]);
        assert_eq!(clipboard.reads(), 1);

        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_Z, false, true)
        });
        {
            let snapshot = host.runtime.presentation().expect("snapshot");
            let element = snapshot.node(node).expect("field");
            assert_eq!(element.text_value(), Some("old"));
            let edit = element.edit().expect("edit");
            assert_eq!((edit.anchor(), edit.caret()), (1, 1));
        }
        assert_eq!(*inputs.lock().expect("inputs"), ["oinkld", "old"]);

        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_Y, false, true)
        });
        assert_eq!(
            host.runtime
                .presentation()
                .expect("snapshot")
                .node(node)
                .expect("field")
                .text_value(),
            Some("oinkld")
        );
    }

    #[test]
    fn ctrl_v_replaces_a_selection_filters_controls_and_never_deletes_on_failure() {
        let (mut host, inputs) = open_history_probe();
        let clipboard = ClipboardProbe::install(&mut host);
        clipboard.set_text(Some("a\r\nb\0c\td"));
        host.runtime.focus("Field").expect("focus");
        let node = history_node(&host);
        host.runtime
            .set_selection_node(node, 0, 2)
            .expect("select the prefix");
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_V, false, true)
        });
        {
            let snapshot = host.runtime.presentation().expect("snapshot");
            let element = snapshot.node(node).expect("field");
            assert_eq!(
                element.text_value(),
                Some("abcdd"),
                "controls are stripped and the selection is replaced"
            );
            let edit = element.edit().expect("edit");
            assert_eq!((edit.anchor(), edit.caret()), (4, 4));
        }
        assert_eq!(*inputs.lock().expect("inputs"), ["abcdd"]);
        assert_eq!(clipboard.reads(), 1);

        clipboard.set_text(Some("\r\n\0\t"));
        host.runtime
            .set_selection_node(node, 1, 3)
            .expect("select again");
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_V, false, true)
        });
        let snapshot = host.runtime.presentation().expect("snapshot");
        let element = snapshot.node(node).expect("field");
        assert_eq!(element.text_value(), Some("abcdd"));
        assert_eq!(
            element.edit().expect("edit").selection(),
            Some((1, 3)),
            "an empty clipboard keeps the selection"
        );
        assert_eq!(*inputs.lock().expect("inputs"), ["abcdd"]);
        assert_eq!(clipboard.reads(), 2);

        clipboard.set_text(None);
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_V, false, true)
        });
        assert_eq!(
            host.runtime
                .presentation()
                .expect("snapshot")
                .node(node)
                .expect("field")
                .edit()
                .expect("edit")
                .selection(),
            Some((1, 3)),
            "a non-text clipboard keeps the selection"
        );
        assert_eq!(clipboard.reads(), 3);

        clipboard.fail_reads();
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_V, false, true)
        });
        assert_eq!(
            host.runtime
                .presentation()
                .expect("snapshot")
                .node(node)
                .expect("field")
                .edit()
                .expect("edit")
                .selection(),
            Some((1, 3)),
            "a failed read keeps the selection"
        );
        assert_eq!(*inputs.lock().expect("inputs"), ["abcdd"]);
        assert_eq!(clipboard.reads(), 4);
    }

    #[test]
    fn ctrl_x_writes_then_deletes_the_selection_once() {
        let (mut host, inputs) = open_history_probe();
        let clipboard = ClipboardProbe::install(&mut host);
        host.runtime.focus("Field").expect("focus");
        let node = history_node(&host);
        host.runtime
            .set_selection_node(node, 1, 3)
            .expect("select the suffix");
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_X, false, true)
        });
        assert_eq!(clipboard.writes(), ["ld"]);
        assert_eq!(clipboard.text().as_deref(), Some("ld"));
        {
            let snapshot = host.runtime.presentation().expect("snapshot");
            let element = snapshot.node(node).expect("field");
            assert_eq!(element.text_value(), Some("o"));
            let edit = element.edit().expect("edit");
            assert_eq!((edit.anchor(), edit.caret()), (1, 1));
            assert_eq!(edit.selection(), None);
            assert_eq!(snapshot.focus(), Some(node), "cut keeps focus");
        }
        assert_eq!(*inputs.lock().expect("inputs"), ["o"]);

        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_Z, false, true)
        });
        let snapshot = host.runtime.presentation().expect("snapshot");
        let element = snapshot.node(node).expect("field");
        assert_eq!(element.text_value(), Some("old"));
        let edit = element.edit().expect("edit");
        assert_eq!((edit.anchor(), edit.caret()), (1, 3));
        assert_eq!(edit.selection(), Some((1, 3)));
    }

    #[test]
    fn ctrl_x_keeps_text_when_the_write_fails_or_no_provider_exists() {
        let (mut host, inputs) = open_history_probe();
        let clipboard = ClipboardProbe::install(&mut host);
        clipboard.fail_writes();
        host.runtime.focus("Field").expect("focus");
        let node = history_node(&host);
        host.runtime
            .set_selection_node(node, 1, 3)
            .expect("select the suffix");
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_X, false, true)
        });
        assert_eq!(clipboard.writes(), ["ld"], "the write was attempted");
        assert_eq!(clipboard.text(), None, "the failed write changed nothing");
        assert!(inputs.lock().expect("inputs").is_empty());
        {
            let snapshot = host.runtime.presentation().expect("snapshot");
            let element = snapshot.node(node).expect("field");
            assert_eq!(element.text_value(), Some("old"));
            let edit = element.edit().expect("edit");
            assert_eq!((edit.anchor(), edit.caret()), (1, 3));
            assert_eq!(edit.selection(), Some((1, 3)));
            assert_eq!(snapshot.focus(), Some(node));
        }

        let (mut host, inputs) = open_history_probe();
        host.runtime.focus("Field").expect("focus");
        let node = history_node(&host);
        host.runtime
            .set_selection_node(node, 0, 2)
            .expect("select the prefix");
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_X, false, true)
        });
        let snapshot = host.runtime.presentation().expect("snapshot");
        let element = snapshot.node(node).expect("field");
        assert_eq!(element.text_value(), Some("old"));
        assert_eq!(
            element.edit().expect("edit").selection(),
            Some((0, 2)),
            "a missing provider leaves the selection intact"
        );
        assert!(inputs.lock().expect("inputs").is_empty());
    }

    #[test]
    fn ctrl_x_is_a_no_op_without_a_selection_or_while_composing_or_disabled() {
        let (mut host, inputs) = open_history_probe();
        let clipboard = ClipboardProbe::install(&mut host);
        host.runtime.focus("Field").expect("focus");
        let node = history_node(&host);
        host.runtime.set_caret_node(node, 1).expect("collapse");
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_X, false, true)
        });
        assert!(clipboard.writes().is_empty());

        host.runtime
            .set_selection_node(node, 0, 2)
            .expect("select the prefix");
        let lock = host
            .runtime
            .presentation()
            .expect("snapshot")
            .find_label("Lock")
            .expect("lock")
            .id();
        host.runtime.click_node(lock).expect("disable the field");
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_X, false, true)
        });
        assert!(clipboard.writes().is_empty(), "disabled input cuts nothing");
        assert!(inputs.lock().expect("inputs").is_empty());

        let (mut host, inputs) = open_history_probe();
        let clipboard = ClipboardProbe::install(&mut host);
        host.runtime.focus("Field").expect("focus");
        let node = history_node(&host);
        host.runtime
            .set_selection_node(node, 0, 2)
            .expect("select the prefix");
        host.runtime.compose_start_node(node).expect("start");
        host.runtime
            .compose_update_node(node, "ni")
            .expect("update");
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_X, false, true)
        });
        assert!(clipboard.writes().is_empty(), "composition cuts nothing");
        let snapshot = host.runtime.presentation().expect("snapshot");
        let element = snapshot.node(node).expect("field");
        assert_eq!(element.text_value(), Some("old"));
        assert_eq!(element.edit().expect("edit").preedit(), Some("ni"));
        assert!(inputs.lock().expect("inputs").is_empty());
    }

    #[test]
    fn cut_and_paste_route_between_windows_without_moving_focus() {
        let mut host = open_host();
        host.render().expect("render the primary window");
        let clipboard = ClipboardProbe::install(&mut host);
        host.runtime
            .focus("What needs to be done?")
            .expect("focus the draft");
        let draft = host
            .runtime
            .presentation()
            .expect("snapshot")
            .find_label("What needs to be done?")
            .expect("draft")
            .id();
        host.runtime
            .insert_text_node(draft, "shared")
            .expect("type the draft");
        host.runtime
            .set_selection_node(draft, 0, 6)
            .expect("select the draft");
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_X, false, true)
        });
        assert_eq!(clipboard.writes(), ["shared"]);
        {
            let snapshot = host.presentation(MAIN).expect("main snapshot");
            let element = snapshot.node(draft).expect("draft");
            assert_eq!(element.text_value(), Some(""));
            let edit = element.edit().expect("edit");
            assert_eq!((edit.anchor(), edit.caret()), (0, 0));
            assert_eq!(snapshot.focus(), Some(draft), "cut keeps focus");
        }

        host.show(zenui_demo::DETAILS_WINDOW)
            .expect("show the details window");
        host.runtime
            .focus("Details note")
            .expect("focus the details note");
        let note = host
            .presentation(zenui_demo::DETAILS_WINDOW)
            .expect("details snapshot")
            .find_label("Details note")
            .expect("note")
            .id();
        host.with_window(zenui_demo::DETAILS_WINDOW, |host, window| {
            host.key_down_with(window, VK_V, false, true)
        })
        .expect("paste into the details note");
        {
            let snapshot = host
                .presentation(zenui_demo::DETAILS_WINDOW)
                .expect("details snapshot");
            let element = snapshot.node(note).expect("note");
            assert_eq!(element.text_value(), Some("shared"));
            assert_eq!(element.edit().expect("edit").caret(), 6);
            assert_eq!(snapshot.focus(), Some(note), "paste keeps focus");
        }

        host.runtime
            .set_selection_node(note, 0, 6)
            .expect("select the note");
        host.with_window(zenui_demo::DETAILS_WINDOW, |host, window| {
            host.key_down_with(window, VK_X, false, true)
        })
        .expect("cut from the details note");
        assert_eq!(clipboard.writes(), ["shared", "shared"]);
        {
            let snapshot = host
                .presentation(zenui_demo::DETAILS_WINDOW)
                .expect("details snapshot");
            let element = snapshot.node(note).expect("note");
            assert_eq!(element.text_value(), Some(""));
            assert_eq!(snapshot.focus(), Some(note), "cut keeps focus");
        }
        host.runtime
            .focus("What needs to be done?")
            .expect("focus the draft");
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_V, false, true)
        });
        {
            let snapshot = host.presentation(MAIN).expect("main snapshot");
            let element = snapshot.node(draft).expect("draft");
            assert_eq!(element.text_value(), Some("shared"));
            assert_eq!(element.edit().expect("edit").caret(), 6);
            assert_eq!(snapshot.focus(), Some(draft), "paste keeps focus");
        }
    }

    #[test]
    fn clipboard_shortcuts_are_no_ops_while_composing() {
        let (mut host, inputs) = open_history_probe();
        let clipboard = ClipboardProbe::install(&mut host);
        host.runtime.focus("Field").expect("focus");
        let node = history_node(&host);
        host.runtime
            .set_selection_node(node, 0, 2)
            .expect("select the prefix");
        host.runtime.compose_start_node(node).expect("start");
        host.runtime
            .compose_update_node(node, "ni")
            .expect("update");

        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_C, false, true)
        });
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_V, false, true)
        });
        assert!(clipboard.writes().is_empty());
        assert_eq!(clipboard.reads(), 0);
        let snapshot = host.runtime.presentation().expect("snapshot");
        let element = snapshot.node(node).expect("field");
        assert_eq!(element.text_value(), Some("old"));
        assert_eq!(element.edit().expect("edit").preedit(), Some("ni"));
        assert!(inputs.lock().expect("inputs").is_empty());
    }

    #[test]
    fn clipboard_shortcuts_without_a_focused_input_are_no_ops() {
        let (mut host, inputs) = open_history_probe();
        let clipboard = ClipboardProbe::install(&mut host);
        host.runtime.focus("Field").expect("focus");
        let node = history_node(&host);
        host.runtime
            .set_selection_node(node, 0, 2)
            .expect("select the prefix");
        let lock = host
            .runtime
            .presentation()
            .expect("snapshot")
            .find_label("Lock")
            .expect("lock")
            .id();
        host.runtime.click_node(lock).expect("disable the field");

        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_C, false, true)
        });
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_V, false, true)
        });
        assert!(clipboard.writes().is_empty());
        assert_eq!(clipboard.reads(), 0);
        assert_eq!(
            host.runtime
                .presentation()
                .expect("snapshot")
                .node(node)
                .expect("field")
                .text_value(),
            Some("old")
        );
        assert!(inputs.lock().expect("inputs").is_empty());
    }

    #[test]
    fn a_hidden_host_without_a_provider_drives_no_clipboard_io() {
        let (mut host, inputs) = open_history_probe();
        host.runtime.focus("Field").expect("focus");
        let node = history_node(&host);
        host.runtime
            .set_selection_node(node, 0, 2)
            .expect("select the prefix");

        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_C, false, true)
        });
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_V, false, true)
        });
        let snapshot = host.runtime.presentation().expect("snapshot");
        assert_eq!(
            snapshot.node(node).expect("field").text_value(),
            Some("old")
        );
        assert_eq!(
            snapshot
                .node(node)
                .expect("field")
                .edit()
                .expect("edit")
                .selection(),
            Some((0, 2))
        );
        assert!(inputs.lock().expect("inputs").is_empty());
    }

    #[test]
    fn a_hidden_host_never_registers_a_drop_target_or_initializes_ole() {
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(|catalog| {
                catalog.window(WindowSpec::new(MAIN).primary(), |window| {
                    window
                        .on_drop(|drop| {
                            let _ = drop.into_paths();
                        })
                        .expect("register the drop handler");
                    view! { <view /> }
                })
            })
            .expect("declare the drop window");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open the host");
        host.sync_windows().expect("sync the hidden host");
        let ctx = host.windows[MAIN].ctx;
        assert!(
            host.runtime.window_has_drop_handler(ctx),
            "the runtime still exposes the registered handler"
        );
        assert!(
            host.windows[MAIN].drop_target.is_none(),
            "a hidden host registers no drop target"
        );
        assert!(host.ole.is_none(), "a hidden host initializes no OLE");
    }

    #[test]
    fn altgr_does_not_trigger_copy_or_paste() {
        let (mut host, _inputs) = open_history_probe();
        let clipboard = ClipboardProbe::install(&mut host);
        host.runtime.focus("Field").expect("focus");
        let node = history_node(&host);
        host.runtime
            .set_selection_node(node, 0, 2)
            .expect("select the prefix");
        let hwnd = host.hwnd(MAIN).expect("a live window");

        let _altgr = AltGrPressed::press();
        // AltGr 在 Windows 上是 Ctrl+Alt；两个快捷键都必须让位给字符输入。
        // 安全：消息目标是宿主持有的 HWND。
        unsafe { PostMessageW(Some(hwnd), WM_KEYDOWN, WPARAM(VK_C.0 as usize), LPARAM(0)) }
            .expect("post AltGr+C");
        host.pump().expect("pump AltGr+C");
        assert!(
            clipboard.writes().is_empty(),
            "AltGr+C is not a copy shortcut"
        );
        // 安全：消息目标是宿主持有的 HWND。
        unsafe { PostMessageW(Some(hwnd), WM_KEYDOWN, WPARAM(VK_V.0 as usize), LPARAM(0)) }
            .expect("post AltGr+V");
        host.pump().expect("pump AltGr+V");
        assert_eq!(clipboard.reads(), 0, "AltGr+V is not a paste shortcut");
    }

    #[test]
    fn copy_and_paste_route_between_windows_through_one_provider() {
        let mut host = open_host();
        host.render().expect("render the primary window");
        let clipboard = ClipboardProbe::install(&mut host);
        host.runtime
            .focus("What needs to be done?")
            .expect("focus the draft");
        let draft = host
            .runtime
            .presentation()
            .expect("snapshot")
            .find_label("What needs to be done?")
            .expect("draft")
            .id();
        host.runtime
            .insert_text_node(draft, "shared")
            .expect("type the draft");
        host.runtime
            .set_selection_node(draft, 0, 6)
            .expect("select the draft");
        with_main(&mut host, |host, window| {
            host.key_down_with(window, VK_C, false, true)
        });
        assert_eq!(clipboard.writes(), ["shared"]);

        host.show(zenui_demo::DETAILS_WINDOW)
            .expect("show the details window");
        host.runtime
            .focus("Details note")
            .expect("focus the details note");
        let note = host
            .presentation(zenui_demo::DETAILS_WINDOW)
            .expect("details snapshot")
            .find_label("Details note")
            .expect("note")
            .id();
        assert_eq!(
            host.presentation(zenui_demo::DETAILS_WINDOW)
                .expect("details snapshot")
                .node(note)
                .expect("note")
                .text_value(),
            Some("")
        );
        host.with_window(zenui_demo::DETAILS_WINDOW, |host, window| {
            host.key_down_with(window, VK_V, false, true)
        })
        .expect("paste into the details note");
        let details = host
            .presentation(zenui_demo::DETAILS_WINDOW)
            .expect("details snapshot");
        let element = details.node(note).expect("note");
        assert_eq!(element.text_value(), Some("shared"));
        assert_eq!(element.edit().expect("edit").caret(), 6);
        assert_eq!(
            host.presentation(MAIN)
                .expect("main snapshot")
                .find_label("What needs to be done?")
                .expect("draft")
                .text_value(),
            Some("shared"),
            "pasting into details does not touch the main draft"
        );

        host.with_window(zenui_demo::DETAILS_WINDOW, |host, window| {
            host.key_down_with(window, VK_Z, false, true)
        })
        .expect("undo in the details note");
        assert_eq!(
            host.presentation(zenui_demo::DETAILS_WINDOW)
                .expect("details snapshot")
                .node(note)
                .expect("note")
                .text_value(),
            Some("")
        );
        assert_eq!(
            host.presentation(MAIN)
                .expect("main snapshot")
                .find_label("What needs to be done?")
                .expect("draft")
                .text_value(),
            Some("shared"),
            "undo in details leaves the main draft alone"
        );
    }

    /// 100 行待办窗口；`consume` 为真时窗口根 keydown 吃掉 PageDown。
    fn scrolling_keydown_host(consume: bool) -> Host {
        use zenui_demo::{TodoApp, TodoState};

        let mut runtime = Runtime::new();
        let state = runtime.setup(TodoState::new).expect("todo state");
        runtime
            .declare_windows(move |catalog| {
                catalog.window(
                    WindowSpec::new(MAIN).primary().sized(900, 700),
                    move |window| {
                        window
                            .on_keydown(move |event: &mut KeyEvent| {
                                if consume && event.key() == Key::PageDown {
                                    event.consume();
                                }
                                HandlerOutcome::Ok
                            })
                            .map_err(|_| MountError::Component("register window keydown"))?;
                        view! { <TodoApp window={window} state={state} /> }
                    },
                )
            })
            .expect("declare the window");
        runtime.show(MAIN).expect("show the window");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open host");
        for index in 0..100 {
            host.runtime
                .input("What needs to be done?", &format!("Task {index}"))
                .expect("fill the draft");
            host.runtime.submit("Add task").expect("add a row");
        }
        host.render().expect("render the tall window");
        host
    }

    #[test]
    fn a_consumed_keydown_blocks_the_host_scroll_default() {
        let mut host = scrolling_keydown_host(true);
        let (before, after) = with_main(&mut host, |host, window| {
            let before = window.scroll_offset;
            host.key_down_with(window, VK_NEXT, false, false)
                .expect("page down");
            Ok((before, window.scroll_offset))
        });
        assert_eq!(after, before, "a consumed PageDown does not scroll");

        let mut host = scrolling_keydown_host(false);
        let (before, after) = with_main(&mut host, |host, window| {
            let before = window.scroll_offset;
            host.key_down_with(window, VK_NEXT, false, false)
                .expect("page down");
            Ok((before, window.scroll_offset))
        });
        assert!(
            after > before,
            "an unhandled PageDown still reaches the host scroll default"
        );
    }

    #[test]
    fn pointer_press_focuses_a_checkbox_through_the_press_intent() {
        let mut host = open_host();
        host.render().expect("render the todo window");
        let button = host
            .runtime
            .presentation()
            .expect("snapshot")
            .find_label("Complete Write tests")
            .expect("the checkbox")
            .id();
        let (point, scale, hit) = {
            let scene = host.scene(MAIN).expect("scene");
            let rect = scene.node(button).expect("scene button").rect();
            let point = Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
            eprintln!(
                "button rect {rect:?} point {point:?} hit rect {:?}",
                scene
                    .hit_test(point)
                    .and_then(|id| scene.node(id))
                    .map(|e| e.rect())
            );
            (point, scene.viewport().scale(), scene.hit_test(point))
        };
        assert_eq!(hit, Some(button), "the test point hits the button");
        with_main(&mut host, |host, window| {
            host.pointer_pressed(window, pointer_lparam(point, scale))
        });
        let snapshot = host.runtime.presentation().expect("snapshot");
        let focused = snapshot
            .focus()
            .and_then(|node| snapshot.node(node))
            .map(|element| (element.tag(), element.label()));
        assert_eq!(
            focused,
            Some(("checkbox", Some("Complete Write tests"))),
            "the host press path focuses the pressed checkbox"
        );
    }

    #[component]
    fn CharProbe(calls: Rc<Cell<u32>>) -> MountResult<MountPlan> {
        let value = signal(String::new());
        view! {
            <text-input
                aria:label="char-probe"
                value={value}
                on:input={move |next: String| {
                    calls.set(calls.get() + 1);
                    value.set(next);
                }}
                on:keydown={move |event: &mut KeyEvent| {
                    if event.key() == Key::Other {
                        event.consume();
                    }
                }}
            />
        }
    }

    /// 一个聚焦输入，其 keydown handler 消费所有可打印键。
    fn suppression_host() -> (Host, Rc<Cell<u32>>) {
        let calls = Rc::new(Cell::new(0));
        let probe_calls = calls.clone();
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(move |catalog| {
                catalog.window(WindowSpec::new(MAIN).primary().sized(400, 300), move |_| {
                    let calls = probe_calls.clone();
                    view! { <CharProbe calls={calls} /> }
                })
            })
            .expect("declare the window");
        runtime.show(MAIN).expect("show the window");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open host");
        host.render().expect("render the window");
        host.runtime.focus("char-probe").expect("focus the input");
        (host, calls)
    }

    #[test]
    fn a_consumed_keydown_suppresses_the_matching_char() {
        let (mut host, calls) = suppression_host();
        let scan = (0x1e_i32 << 16) as isize;
        with_main(&mut host, |host, window| {
            host.key_down(window, WPARAM(VK_A.0 as usize), LPARAM(scan))
                .expect("keydown");
            host.char_typed(window, WPARAM('a' as usize), LPARAM(scan))
                .expect("suppressed char");
            Ok(())
        });
        let snapshot = host.runtime.presentation().expect("snapshot");
        assert_eq!(
            snapshot
                .find_label("char-probe")
                .expect("input")
                .text_value(),
            Some(""),
            "the consumed keydown drops its WM_CHAR"
        );
        assert_eq!(calls.get(), 0, "the suppressed char runs no on:input");

        with_main(&mut host, |host, window| {
            host.char_typed(
                window,
                WPARAM('b' as usize),
                LPARAM((0x30_i32 << 16) as isize),
            )
            .expect("a later char");
            Ok(())
        });
        let snapshot = host.runtime.presentation().expect("snapshot");
        assert_eq!(
            snapshot
                .find_label("char-probe")
                .expect("input")
                .text_value(),
            Some("b"),
            "the token is single-use"
        );
        assert_eq!(calls.get(), 1);
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
            host.char_typed(window, WPARAM(0x0009), LPARAM(0))
        });
        with_main(&mut host, |host, window| {
            host.char_typed(window, WPARAM(0xDC00), LPARAM(0))
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
            host.char_typed(window, WPARAM(0xD83D), LPARAM(0))
        });
        with_main(&mut host, |host, window| {
            host.char_typed(window, WPARAM(0xDE42), LPARAM(0))
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

    fn pointer_lparam(point: Point, scale: f32) -> LPARAM {
        let x = (point.x * scale).round() as i16;
        let y = (point.y * scale).round() as i16;
        LPARAM((x as isize & 0xffff) | ((y as isize & 0xffff) << 16))
    }

    #[test]
    fn pressing_the_scrollbar_pages_without_cancelling_a_composition() {
        let mut host = open_host();
        for index in 0..100 {
            host.runtime
                .input("What needs to be done?", &format!("Task {index}"))
                .expect("fill the draft");
            host.runtime.submit("Add task").expect("add a row");
        }
        host.render().expect("render the tall window");
        host.runtime
            .focus("Edit Task 0")
            .expect("focus the row input");
        let node = host
            .runtime
            .presentation()
            .expect("snapshot")
            .find_label("Edit Task 0")
            .expect("row input")
            .id();
        let (page, offset, pressed) = with_main(&mut host, |host, window| {
            host.ime_start(window).expect("start the composition");
            host.apply_ime_composition(window, node, request(false, true), None, Some("ni".into()))
                .expect("preedit");
            let scene = window.scene.as_ref().expect("scene");
            let bar = scene.scrollbar().expect("scrollbar");
            let page = ((scene.viewport().height() - 24.0).floor() as i32).max(1);
            let point = Point::new(bar.track().x + 2.0, bar.thumb().bottom() + 30.0);
            let lparam = pointer_lparam(point, scene.viewport().scale());
            host.pointer_pressed(window, lparam)
                .expect("press the track");
            Ok((page, window.scroll_offset, window.pressed))
        });
        assert_eq!(offset, page, "the track click pages by one page");
        assert_eq!(pressed, None, "the scrollbar press is not a content press");
        let snapshot = host.runtime.presentation().expect("snapshot");
        let edit = snapshot.node(node).expect("editor").edit().expect("edit");
        assert_eq!(edit.preedit(), Some("ni"), "the composition survives");
        assert!(
            edit.composition().is_some(),
            "pressing the scrollbar does not cancel the composition"
        );
    }

    #[test]
    fn a_composing_editor_keeps_its_scroll_under_editing_keys() {
        let mut host = open_host();
        for index in 0..100 {
            host.runtime
                .input("What needs to be done?", &format!("Task {index}"))
                .expect("fill the draft");
            host.runtime.submit("Add task").expect("add a row");
        }
        host.render().expect("render the tall window");
        host.runtime
            .focus("Edit Task 0")
            .expect("focus the row input");
        let node = host
            .runtime
            .presentation()
            .expect("snapshot")
            .find_label("Edit Task 0")
            .expect("row input")
            .id();
        // 滚到聚焦行恰好在窗口上方一行的 grace 里：编辑键仍不得编辑、显露
        // 或滚动，组合与 caret 保持挂载（ADR-0011 的焦点 grace）。
        let band = host.scene(MAIN).expect("scene").row_bands()[0];
        let away = (band.top() + band.row_height() + 10.0) as i32;
        with_main(&mut host, |host, window| {
            host.ime_start(window).expect("start the composition");
            host.apply_ime_composition(window, node, request(false, true), None, Some("ni".into()))
                .expect("preedit");
            // 组合启动本身请求过显露；只观察按键造成的请求。
            window.reveal_requested = false;
            window.scroll_offset = away;
            for key in [VK_HOME, VK_END, VK_LEFT, VK_RIGHT, VK_BACK, VK_DELETE] {
                host.key_down_with(window, key, false, false)
                    .expect("composing editing key");
            }
            assert!(
                !window.reveal_requested,
                "composing editing keys do not request a reveal"
            );
            Ok(())
        });
        host.render().expect("render after the composing keys");
        assert_eq!(
            host.windows[MAIN].scroll_offset, away,
            "composing editing keys do not scroll the window"
        );
        let snapshot = host.runtime.presentation().expect("snapshot");
        let edit = snapshot.node(node).expect("editor").edit().expect("edit");
        assert_eq!(edit.preedit(), Some("ni"), "the composition survives");
        assert!(
            edit.composition().is_some(),
            "composing editing keys do not end the composition"
        );
    }

    #[test]
    fn a_reveal_that_moves_the_offset_resets_the_wheel_residue() {
        let mut host = open_host();
        host.render().expect("render the initial window");
        host.windows.get_mut(MAIN).expect("the main window").wheel = wheel(WheelLines::Lines(3));
        let sub_notch = WPARAM((((-24i16) as u16 as u32) << 16) as usize);
        with_main(&mut host, |host, window| {
            host.wheel_scrolled(window, sub_notch)
                .expect("apply a sub-notch wheel step");
            assert_eq!(window.scroll_offset, 14, "the sub-notch step applies");
            assert_ne!(
                window.wheel.accumulator, 0,
                "the sub-notch residue is retained"
            );
            Ok(())
        });

        // 把草稿输入滚出视口后再聚焦：显露会把它拉回顶部并改变偏移。
        with_main(&mut host, |_host, window| {
            window.scroll_offset = 400;
            Ok(())
        });
        host.render().expect("render the scrolled window");
        host.runtime
            .focus("What needs to be done?")
            .expect("focus the offscreen draft");
        host.render().expect("reveal the draft");

        let window = &host.windows[MAIN];
        assert!(
            window.scroll_offset < 400,
            "the reveal really moved the offset"
        );
        assert_eq!(
            window.wheel.accumulator, 0,
            "a reveal that moves the offset discards the wheel residue"
        );
        assert_eq!(window.wheel.anchor, 0);
    }

    #[test]
    fn scrolling_a_focused_composing_row_out_of_the_grace_window_blurs_it() {
        let mut host = open_host();
        for index in 0..100 {
            host.runtime
                .input("What needs to be done?", &format!("Task {index}"))
                .expect("fill the draft");
            host.runtime.submit("Add task").expect("add a row");
        }
        host.render().expect("render the tall window");
        host.runtime
            .focus("Edit Task 0")
            .expect("focus the row input");
        let node = host
            .runtime
            .presentation()
            .expect("snapshot")
            .find_label("Edit Task 0")
            .expect("row input")
            .id();
        with_main(&mut host, |host, window| {
            host.ime_start(window).expect("start the composition");
            host.apply_ime_composition(window, node, request(false, true), None, Some("ni".into()))
                .expect("preedit");
            // 开始时请求的显露已经发生；这里只观察滚动后的行为。
            window.reveal_requested = false;
            Ok(())
        });

        // 远距离滚动：目标窗口从列表顶部离开，聚焦行超过一行 grace。
        host.windows
            .get_mut(MAIN)
            .expect("the main window")
            .scroll_offset = 2000;
        host.render().expect("render the far window");
        assert!(
            host.runtime.focused().is_none(),
            "the focused row is blurred before it unmounts"
        );
        let snapshot = host.runtime.presentation().expect("snapshot");
        assert_eq!(snapshot.focus(), None);
        assert!(
            snapshot.node(node).is_none(),
            "the composing row was unmounted by the new window"
        );
    }

    #[test]
    fn wheel_and_pointer_ticks_reuse_the_runtime_snapshot() {
        let mut host = open_host();
        for index in 0..1000 {
            host.runtime
                .input("What needs to be done?", &format!("Task {index}"))
                .expect("fill the draft");
            host.runtime.submit("Add task").expect("add a row");
        }
        host.render().expect("render the tall window");
        // 测试不依赖机器的滚轮设置：固定为一个 notch 三行。
        host.windows.get_mut(MAIN).expect("the main window").wheel = wheel(WheelLines::Lines(3));
        assert_eq!(
            host.windows[MAIN].snapshot_revision,
            host.runtime.presentation_revision(),
            "the render stores the current presentation revision"
        );
        let scale = host.scene(MAIN).expect("scene").viewport().scale();
        let point = Point::new(200.0, 200.0);
        let lparam = pointer_lparam(point, scale);
        let blank = pointer_lparam(Point::new(5.0, 5.0), scale);
        // 第一次空白按压把焦点窗口切到 main，这是运行时变化，允许一次重建。
        for message in [WM_LBUTTONDOWN, WM_LBUTTONUP] {
            send_msg(
                &mut host,
                MSG {
                    message,
                    lParam: blank,
                    ..Default::default()
                },
            );
        }
        assert_eq!(
            host.windows[MAIN].snapshot_revision,
            host.runtime.presentation_revision(),
            "the warm-up press leaves a current cached snapshot"
        );
        let before: Vec<NodeId> = host
            .runtime
            .presentation()
            .expect("snapshot")
            .elements()
            .map(zenui::SnapshotElement::id)
            .collect();
        let builds = host.runtime.presentation_builds();
        let counters = host.runtime.counters();
        // 不移动滚动偏移的瞬态输入完全复用快照；每次 tick 都不构建。
        for step in 0..30 {
            send_msg(
                &mut host,
                MSG {
                    message: WM_MOUSEMOVE,
                    lParam: lparam,
                    ..Default::default()
                },
            );
            for message in [WM_LBUTTONDOWN, WM_LBUTTONUP] {
                send_msg(
                    &mut host,
                    MSG {
                        message,
                        lParam: blank,
                        ..Default::default()
                    },
                );
            }
            assert_eq!(
                host.runtime.presentation_builds(),
                builds,
                "snapshot rebuilt after pointer tick {step}"
            );
            with_main(&mut host, |host, window| host.blink_caret(window));
            assert_eq!(
                host.runtime.presentation_builds(),
                builds,
                "snapshot rebuilt after blink {step}"
            );
        }
        // 滚轮跨过行边界时窗口推进；每次事件至多重建一次快照（一帧两遍场景）。
        let before_wheel = host.runtime.presentation_builds();
        send_msg(
            &mut host,
            MSG {
                message: WM_MOUSEWHEEL,
                wParam: WPARAM(((-120i16 as u16) as usize) << 16),
                ..Default::default()
            },
        );
        assert!(
            host.scene(MAIN).expect("scene").scroll_offset() > 0,
            "the wheel tick really scrolled"
        );
        let builds = host.runtime.presentation_builds();
        assert!(
            builds <= before_wheel + 1,
            "crossing a row boundary rebuilds the snapshot at most once"
        );
        // 新窗口内继续指针/闪烁瞬态：再次完全复用快照。
        for step in 0..10 {
            send_msg(
                &mut host,
                MSG {
                    message: WM_MOUSEMOVE,
                    lParam: lparam,
                    ..Default::default()
                },
            );
            for message in [WM_LBUTTONDOWN, WM_LBUTTONUP] {
                send_msg(
                    &mut host,
                    MSG {
                        message,
                        lParam: blank,
                        ..Default::default()
                    },
                );
            }
            with_main(&mut host, |host, window| host.blink_caret(window));
            assert_eq!(
                host.runtime.presentation_builds(),
                builds,
                "the new row window reuses its snapshot after tick {step}"
            );
        }
        assert_eq!(
            host.runtime.counters(),
            counters,
            "wheel and pointer ticks leave runtime counters flat"
        );
        let after: Vec<NodeId> = host
            .runtime
            .presentation()
            .expect("snapshot")
            .elements()
            .map(zenui::SnapshotElement::id)
            .collect();
        assert_ne!(
            before, after,
            "crossing a row boundary remounts the visible rows"
        );
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
            host.char_typed(window, WPARAM(u32::from('中') as usize), LPARAM(0))
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
            host.key_down(window, WPARAM(VK_PROCESSKEY.0 as usize), LPARAM(0))
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
    fn a_window_action_keeps_synchronous_messages_out_of_the_host() {
        let mut host = open_host();
        host.render().expect("render");
        let hwnd = host.hwnd(MAIN).expect("a live window");
        {
            let _pointer = HostPointer::install(&mut host as *mut Host);
            host.with_window(MAIN, |host, window| {
                // 真实 IME 会在 ImmGetCompositionStringW / ImmSetCompositionWindow
                // 内同步送出 WM_IME_NOTIFY 等消息；动作正在借用宿主，窗口过程
                // 不得再次借出，否则会把取出的窗口当成已关闭。
                // 安全：SendMessageW 在调用线程同步调用窗口过程；外层不读取宿主。
                unsafe {
                    SendMessageW(hwnd, WM_IME_COMPOSITION, None, None);
                }
                host.without_routing(Some(window.hwnd()), |_| unsafe {
                    SendMessageW(hwnd, WM_IME_COMPOSITION, None, None);
                });
                // 内层动作结束后外层仍然借用宿主；路由必须保持关闭。
                // 安全：同上。
                unsafe {
                    SendMessageW(hwnd, WM_IME_COMPOSITION, None, None);
                }
                Ok(())
            })
            .expect("the action finishes");
        }
        assert!(
            host.deferred_error.is_none(),
            "a synchronous IME message borrowed the host twice: {:?}",
            host.deferred_error
        );
    }

    /// 外部 reset 探针：编辑器的值来自应用 Signal，任务交付时写入新值。
    fn external_reset_host(reset: &'static str) -> (Host, NodeId) {
        let mut runtime = Runtime::new();
        let field = runtime
            .setup(|| signal(String::from("old")))
            .expect("application setup");
        runtime
            .declare_windows(move |catalog| {
                catalog.window(WindowSpec::new(MAIN).primary().sized(320, 240), move |_| {
                    view! {
                        <text-input
                            aria:label="Field"
                            value={field}
                            on:input={move |value: String| field.set(value)}
                        />
                    }
                })?;
                Ok(())
            })
            .expect("catalog");
        let task = runtime
            .spawn_task(runtime.task_scope(), move |_| String::from(reset))
            .expect("spawn the reset task");
        runtime
            .task_done(task, move |value| {
                field.set(value);
                Ok(())
            })
            .expect("reset continuation");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open host");
        host.runtime.focus("Field").expect("focus");
        let node = host
            .runtime
            .presentation()
            .expect("snapshot")
            .find_label("Field")
            .expect("field")
            .id();
        (host, node)
    }

    #[test]
    fn an_external_reset_during_composition_cancels_the_os_composition() {
        let (mut host, node) = external_reset_host("external");
        with_main(&mut host, |host, window| {
            host.ime_start(window)?;
            host.apply_ime_composition(window, node, request(false, true), None, Some("ni".into()))
        });
        with_main(&mut host, |_, window| {
            window.os_composition = Some(true);
            Ok(())
        });

        host.runtime.wait_for_tasks();
        host.deliver_tasks().expect("deliver the external reset");

        let snapshot = host.runtime.presentation().expect("snapshot");
        let element = snapshot.node(node).expect("field");
        assert_eq!(element.text_value(), Some("external"));
        let edit = element.edit().expect("edit");
        assert_eq!(edit.preedit(), None, "the reset cancelled the preedit");
        assert_eq!(edit.caret(), 8, "the caret moves to the end");
        assert_eq!(edit.selection(), None);
        let notifications = with_main(&mut host, |_, window| Ok(window.ime_cancel_notifications));
        assert_eq!(
            notifications, 1,
            "the host tells the OS IME to drop its composition string"
        );
    }

    #[test]
    fn a_normal_composition_survives_an_unrelated_task_delivery() {
        let mut runtime = Runtime::new();
        let (field, status) = runtime
            .setup(|| (signal(String::from("old")), signal(String::from("idle"))))
            .expect("application setup");
        runtime
            .declare_windows(move |catalog| {
                catalog.window(WindowSpec::new(MAIN).primary().sized(320, 240), move |_| {
                    let shown = memo(move || status.get());
                    view! {
                        <view>
                            <text aria:label="status">{shown}</text>
                            <text-input
                                aria:label="Field"
                                value={field}
                                on:input={move |value: String| field.set(value)}
                            />
                        </view>
                    }
                })?;
                Ok(())
            })
            .expect("catalog");
        let task = runtime
            .spawn_task(runtime.task_scope(), |_| String::from("done"))
            .expect("spawn the status task");
        runtime
            .task_done(task, move |value| {
                status.set(value);
                Ok(())
            })
            .expect("status continuation");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open host");
        host.runtime.focus("Field").expect("focus");
        let node = host
            .runtime
            .presentation()
            .expect("snapshot")
            .find_label("Field")
            .expect("field")
            .id();
        with_main(&mut host, |host, window| {
            host.ime_start(window)?;
            host.apply_ime_composition(window, node, request(false, true), None, Some("ni".into()))
        });
        with_main(&mut host, |_, window| {
            window.os_composition = Some(true);
            Ok(())
        });

        host.runtime.wait_for_tasks();
        host.deliver_tasks().expect("deliver the status");

        let snapshot = host.runtime.presentation().expect("snapshot");
        assert_eq!(
            snapshot.find_label("status").expect("label").text(),
            "done",
            "the unrelated delivery repainted the window"
        );
        let edit = snapshot.node(node).expect("field").edit().expect("edit");
        assert_eq!(edit.preedit(), Some("ni"), "the composition survives");
        assert!(edit.composition().is_some());
        let notifications = with_main(&mut host, |_, window| Ok(window.ime_cancel_notifications));
        assert_eq!(notifications, 0, "the OS IME keeps its composition");
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
    fn a_task_continuation_can_open_a_window_through_the_host() {
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(|catalog| {
                catalog.window(WindowSpec::new("main").primary(), |window| {
                    view! {
                        <button
                            aria:label="later"
                            on:click={move || {
                                let task = window
                                    .task_scope()
                                    .spawn(|_| String::from("ready"))
                                    .expect("spawn the task");
                                task.on_done(move |_| {
                                    window.request_show("details").expect("queue the show");
                                    Ok(())
                                })
                                .expect("register the continuation");
                            }}
                        >"Later"</button>
                    }
                })?;
                catalog.window(
                    WindowSpec::new("details"),
                    |_| view! { <text>"details"</text> },
                )?;
                Ok(())
            })
            .expect("catalog");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open the host");
        host.render().expect("render the primary window");

        host.runtime.click_label("later").expect("start the task");
        host.runtime.wait_for_tasks();
        // 只有宿主级唤醒消息会进入 deliver_tasks；任务交付后的窗口请求在那里结算。
        send_msg(
            &mut host,
            MSG {
                message: host_message::WAKE,
                ..Default::default()
            },
        );
        assert!(host.hwnd(MAIN).is_some());
        assert!(
            host.hwnd("details").is_some(),
            "a delivery-time request creates the native window"
        );
        assert!(host.scene("details").is_some(), "the new window paints");
    }

    #[test]
    fn wm_close_on_a_confirm_window_opens_a_disabled_prompt() {
        let mut host = open_confirm_host();
        send_msg(
            &mut host,
            MSG {
                message: WM_CLOSE,
                ..Default::default()
            },
        );
        assert!(host.hwnd(MAIN).is_some());
        let confirmation = host.hwnd("confirm").expect("a confirmation HWND");
        assert!(!host.windows[MAIN].enabled, "the parent is disabled");
        assert!(
            // 安全：只查询由宿主持有的 HWND 的启用状态。
            !unsafe { IsWindowEnabled(host.hwnd(MAIN).expect("main HWND")) }.as_bool(),
            "the native parent HWND is disabled"
        );
        assert!(host.windows["confirm"].enabled);
        assert!(host.scene("confirm").is_some());
        let parent = host
            .accessibility_tree(MAIN)
            .expect("the disabled parent keeps its tree");
        let draft = parent.find_name("draft").expect("the draft input");
        assert!(!draft.enabled(), "the close prompt disables the parent");
        assert!(!draft.focusable());
        assert!(host.accessibility_tree("confirm").is_some());

        send_msg(
            &mut host,
            MSG {
                message: WM_CLOSE,
                ..Default::default()
            },
        );
        assert_eq!(host.hwnd("confirm"), Some(confirmation));
        assert_eq!(host.runtime.windows().len(), 2, "no second epoch or prompt");

        click_label_in(&mut host, "confirm", "Deny");
        assert!(host.hwnd("confirm").is_none());
        assert!(host.windows[MAIN].enabled);
        assert!(
            // 安全：只查询由宿主持有的 HWND 的启用状态。
            unsafe { IsWindowEnabled(host.hwnd(MAIN).expect("main HWND")) }.as_bool(),
            "the native parent HWND is re-enabled"
        );
        assert!(host.runtime.window_accepts_input(host.windows[MAIN].ctx));
    }

    #[test]
    fn suspending_a_parent_cancels_its_ime_composition() {
        let mut host = open_confirm_host();
        let node = host
            .runtime
            .presentation()
            .expect("snapshot")
            .find_label("draft")
            .expect("draft input")
            .id();
        host.runtime.focus_node(node).expect("focus the draft");
        host.runtime.compose_start_node(node).expect("start");
        host.runtime
            .compose_update_node(node, "ni")
            .expect("update");

        send_msg(
            &mut host,
            MSG {
                message: WM_CLOSE,
                ..Default::default()
            },
        );
        let main = host.windows[MAIN].ctx;
        let snapshot = host.runtime.presentation_of(main).expect("snapshot");
        assert!(
            snapshot
                .node(node)
                .expect("draft")
                .edit()
                .expect("edit")
                .composition()
                .is_none(),
            "suspending the parent cancels the pending composition"
        );
        assert!(!host.windows[MAIN].enabled);
    }

    #[test]
    fn closing_the_confirmation_window_denies_the_parent() {
        let mut host = open_confirm_host();
        send_msg(
            &mut host,
            MSG {
                message: WM_CLOSE,
                ..Default::default()
            },
        );
        assert!(!host.windows[MAIN].enabled);
        send_msg_in(
            &mut host,
            "confirm",
            MSG {
                message: WM_CLOSE,
                ..Default::default()
            },
        );
        assert!(host.hwnd("confirm").is_none());
        assert!(host.hwnd(MAIN).is_some());
        assert!(host.windows[MAIN].enabled);
    }

    #[test]
    fn escape_on_the_confirmation_denies_and_reenables_the_parent() {
        let mut host = open_confirm_host();
        send_msg(
            &mut host,
            MSG {
                message: WM_CLOSE,
                ..Default::default()
            },
        );
        assert!(!host.windows[MAIN].enabled);

        send_msg_in(
            &mut host,
            "confirm",
            MSG {
                message: WM_KEYDOWN,
                wParam: WPARAM(VK_ESCAPE.0 as usize),
                ..Default::default()
            },
        );
        assert!(host.hwnd("confirm").is_none());
        assert!(host.windows[MAIN].enabled);
        assert!(host.runtime.window_accepts_input(host.windows[MAIN].ctx));
    }

    #[test]
    fn allow_from_the_confirmation_closes_the_group() {
        let mut host = open_confirm_host();
        send_msg(
            &mut host,
            MSG {
                message: WM_CLOSE,
                ..Default::default()
            },
        );
        click_label_in(&mut host, "confirm", "Allow");
        assert!(host.runtime.windows().is_empty());
        assert!(host.hwnd(MAIN).is_none());
        assert!(host.hwnd("confirm").is_none());
    }

    #[test]
    fn expected_dispatch_rejections_are_no_ops() {
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
            ignore_expected_dispatch(app.blur_node(id)).is_ok(),
            "the host ignores the rejection"
        );
    }

    fn menu_host() -> (Host, Rc<Cell<u32>>) {
        let runs = Rc::new(Cell::new(0));
        let handler_runs = runs.clone();
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(move |catalog| {
                let handler_runs = handler_runs.clone();
                catalog.window(
                    WindowSpec::new(MAIN).primary().titled("ZenUI Menu"),
                    move |window| {
                        let handler_runs = handler_runs.clone();
                        window
                            .on_command("view.status", move |_| {
                                handler_runs.set(handler_runs.get() + 1);
                            })
                            .expect("register view.status");
                        window
                            .on_command("file.close", |_| {})
                            .expect("register file.close");
                        window
                            .on_command("file.quit", |_| {})
                            .expect("register file.quit");
                        view! {
                            <view>
                                <text aria:label="status">"Ready"</text>
                                <text-input aria:label="edit" />
                            </view>
                        }
                    },
                )?;
                catalog.menu(
                    MAIN,
                    MenuSpec::new()
                        .submenu(
                            "File",
                            MenuSpec::new()
                                .command("file.close", "Close")
                                .separator()
                                .item(MenuCommand::new("file.quit", "Quit").enabled(false)),
                        )
                        .submenu(
                            "View",
                            MenuSpec::new().item(
                                MenuCommand::new("view.status", "Show status")
                                    .checked(true)
                                    .bind_shortcut("Ctrl+S")?,
                            ),
                        ),
                )?;
                Ok(())
            })
            .expect("declare the menu host");
        (
            Host::open(runtime, HostConfig::hidden()).expect("open the menu host"),
            runs,
        )
    }

    fn menu_confirm_host() -> (Host, Rc<Cell<u32>>) {
        let runs = Rc::new(Cell::new(0));
        let handler_runs = runs.clone();
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(move |catalog| {
                let handler_runs = handler_runs.clone();
                catalog.window(
                    WindowSpec::new(MAIN)
                        .primary()
                        .close_policy(ClosePolicy::Confirm),
                    move |window| {
                        let handler_runs = handler_runs.clone();
                        window
                            .on_command("act", move |_| {
                                handler_runs.set(handler_runs.get() + 1);
                            })
                            .expect("register act");
                        window
                            .on_close(move |_| {
                                window
                                    .request_show("confirm")
                                    .expect("show the confirmation");
                                CloseOutcome::Pending
                            })
                            .expect("register the close handler");
                        view! { <view /> }
                    },
                )?;
                catalog.window(
                    WindowSpec::new("confirm").confirmation_for(MAIN),
                    |window| {
                        window
                            .close_request()
                            .expect("read the pending request")
                            .expect("a pending request exists");
                        view! { <view /> }
                    },
                )?;
                catalog.menu(MAIN, MenuSpec::new().command("act", "Act"))?;
                Ok(())
            })
            .expect("declare the confirmation menu host");
        (
            Host::open(runtime, HostConfig::hidden()).expect("open the confirmation menu host"),
            runs,
        )
    }

    fn menu_state_host() -> (Host, Rc<Cell<u32>>) {
        let runs = Rc::new(Cell::new(0));
        let mut runtime = Runtime::new();
        let checked = runtime
            .setup(|| signal(true))
            .expect("menu state application setup");
        let handler_runs = runs.clone();
        runtime
            .declare_windows(move |catalog| {
                let handler_runs = handler_runs.clone();
                catalog.window(
                    WindowSpec::new(MAIN).primary().titled("ZenUI Menu State"),
                    move |window| {
                        let handler_runs = handler_runs.clone();
                        window
                            .on_command("file.clear", move |_| {
                                handler_runs.set(handler_runs.get() + 1);
                            })
                            .expect("register file.clear");
                        window
                            .on_command("view.status", |_| {})
                            .expect("register view.status");
                        window
                            .on_command("state.toggle", move |_| checked.set(!checked.get()))
                            .expect("register state.toggle");
                        window
                            .on_command("app.about", |_| {})
                            .expect("register app.about");
                        window
                            .on_menu_state(move |command| match command.id() {
                                "file.clear" | "app.about" => {
                                    MenuCommandState::new(checked.get(), false)
                                }
                                "view.status" => MenuCommandState::new(true, checked.get()),
                                _ => command.default_state(),
                            })
                            .expect("register menu state");
                        view! {
                            <view>
                                <text aria:label="status">"Ready"</text>
                            </view>
                        }
                    },
                )?;
                catalog.menu(
                    MAIN,
                    MenuSpec::new()
                        .submenu("File", MenuSpec::new().command("file.clear", "Clear"))
                        .submenu(
                            "View",
                            MenuSpec::new()
                                .command("view.status", "Show status")
                                .command("state.toggle", "Toggle"),
                        )
                        .command("app.about", "About"),
                )?;
                Ok(())
            })
            .expect("declare the menu state host");
        (
            Host::open(runtime, HostConfig::hidden()).expect("open the menu state host"),
            runs,
        )
    }

    fn accelerator_host() -> (Host, Rc<RefCell<Vec<CommandSource>>>) {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = Runtime::new();
        let (enabled, draft) = runtime
            .setup(|| (signal(true), signal(String::new())))
            .expect("accelerator application setup");
        let handler_events = events.clone();
        runtime
            .declare_windows(move |catalog| {
                let handler_events = handler_events.clone();
                catalog.window(WindowSpec::new(MAIN).primary(), move |window| {
                    let handler_events = handler_events.clone();
                    window
                        .on_command("file.save", move |event| {
                            handler_events.borrow_mut().push(event.source());
                        })
                        .expect("register file.save");
                    window
                        .on_command("state.toggle", move |_| enabled.set(!enabled.get()))
                        .expect("register state.toggle");
                    window
                        .on_menu_state(move |command| match command.id() {
                            "file.save" => MenuCommandState::new(enabled.get(), false),
                            _ => command.default_state(),
                        })
                        .expect("register menu state");
                    view! {
                        <view>
                            <text-input
                                aria:label="edit"
                                value={draft}
                                on:input=move |value: String| draft.set(value)
                            />
                            <button aria:label="other">"Other"</button>
                        </view>
                    }
                })?;
                catalog.menu(
                    MAIN,
                    MenuSpec::new()
                        .item(MenuCommand::new("file.save", "Save").bind_shortcut("Ctrl+S")?)
                        .item(MenuCommand::new("state.toggle", "Toggle").bind_shortcut("Ctrl+T")?),
                )?;
                Ok(())
            })
            .expect("declare the accelerator host");
        (
            Host::open(runtime, HostConfig::hidden()).expect("open the accelerator host"),
            events,
        )
    }

    fn accelerator_entries(accelerator: &NativeAccelerator) -> i32 {
        // 安全：句柄由宿主窗口持有；只查询表项数。
        unsafe { CopyAcceleratorTableW(accelerator.handle(), None) }
    }

    fn send_command(host: &mut Host, wparam: u32) {
        send_msg(
            host,
            MSG {
                message: WM_COMMAND,
                wParam: WPARAM(wparam as usize),
                ..Default::default()
            },
        );
    }

    /// 测试线程按住 Ctrl，析构时恢复原来的键盘状态。
    struct ControlPressed {
        previous: [u8; 256],
    }

    impl ControlPressed {
        fn press() -> Self {
            let mut previous = [0u8; 256];
            // 安全：缓冲区由本地数组提供。
            unsafe { GetKeyboardState(&mut previous) }.expect("save the keyboard state");
            let mut pressed = [0u8; 256];
            pressed[VK_CONTROL.0 as usize] = 0x80;
            // 安全：缓冲区由本地数组提供；测试线程与消息循环同一线程。
            unsafe { SetKeyboardState(&pressed) }.expect("press Ctrl");
            Self { previous }
        }
    }

    impl Drop for ControlPressed {
        fn drop(&mut self) {
            // 安全：恢复测试线程自己的键盘状态。
            let _ = unsafe { SetKeyboardState(&self.previous) };
        }
    }

    /// 测试线程按住 Ctrl+Alt（AltGr 在 Windows 上的表现），析构时恢复。
    struct AltGrPressed {
        previous: [u8; 256],
    }

    impl AltGrPressed {
        fn press() -> Self {
            let mut previous = [0u8; 256];
            // 安全：缓冲区由本地数组提供。
            unsafe { GetKeyboardState(&mut previous) }.expect("save the keyboard state");
            let mut pressed = [0u8; 256];
            pressed[VK_CONTROL.0 as usize] = 0x80;
            pressed[VK_MENU.0 as usize] = 0x80;
            // 安全：缓冲区由本地数组提供；测试线程与消息循环同一线程。
            unsafe { SetKeyboardState(&pressed) }.expect("press AltGr");
            Self { previous }
        }
    }

    impl Drop for AltGrPressed {
        fn drop(&mut self) {
            // 安全：恢复测试线程自己的键盘状态。
            let _ = unsafe { SetKeyboardState(&self.previous) };
        }
    }

    fn menu_label(menu: HMENU, item: u32, flags: MENU_ITEM_FLAGS) -> String {
        let mut buffer = [0u16; 64];
        let length = unsafe { GetMenuStringW(menu, item, Some(&mut buffer), flags) };
        assert!(length > 0, "the menu item has a label");
        String::from_utf16_lossy(&buffer[..length as usize])
    }

    #[test]
    fn hidden_window_reports_the_declared_menu_bar() {
        let (host, _runs) = menu_host();
        let hwnd = host.hwnd(MAIN).expect("a live window");
        let menu = unsafe { GetMenu(hwnd) };
        assert!(!menu.is_invalid(), "the HWND owns a native menu");
        assert_eq!(unsafe { GetMenuItemCount(Some(menu)) }, 2);

        let file = unsafe { GetSubMenu(menu, 0) };
        let view = unsafe { GetSubMenu(menu, 1) };
        assert!(!file.is_invalid() && !view.is_invalid());
        assert_eq!(menu_label(menu, 0, MF_BYPOSITION), "File");
        assert_eq!(menu_label(menu, 1, MF_BYPOSITION), "View");
        assert_eq!(unsafe { GetMenuItemCount(Some(file)) }, 3);
        assert_eq!(menu_label(file, 0, MF_BYPOSITION), "Close");
        assert_eq!(menu_label(file, 2, MF_BYPOSITION), "Quit");

        assert_eq!(unsafe { GetMenuItemID(file, 0) }, 1);
        assert_eq!(unsafe { GetMenuItemID(file, 1) }, 0, "a separator has id 0");
        assert_eq!(unsafe { GetMenuItemID(file, 2) }, 2);
        assert_eq!(unsafe { GetMenuItemID(view, 0) }, 3);
        assert_eq!(menu_label(file, 1, MF_BYCOMMAND), "Close");
        assert_eq!(menu_label(view, 3, MF_BYCOMMAND), "Show status\tCtrl+S");

        let quit = unsafe { GetMenuState(file, 2, MF_BYCOMMAND) };
        assert_ne!(quit & MF_GRAYED.0, 0, "the declared disabled state");
        let status = unsafe { GetMenuState(view, 3, MF_BYCOMMAND) };
        assert_ne!(status & MF_CHECKED.0, 0, "the declared checked state");
    }

    #[test]
    fn menu_commands_dispatch_once_and_reject_non_menu_messages() {
        let (mut host, runs) = menu_host();
        send_msg(
            &mut host,
            MSG {
                message: WM_COMMAND,
                wParam: WPARAM(3),
                ..Default::default()
            },
        );
        assert_eq!(runs.get(), 1, "WM_COMMAND runs the declared handler once");

        send_msg(
            &mut host,
            MSG {
                message: WM_COMMAND,
                wParam: WPARAM(3),
                lParam: LPARAM(1),
                ..Default::default()
            },
        );
        assert_eq!(runs.get(), 1, "a control notification does not dispatch");

        send_msg(
            &mut host,
            MSG {
                message: WM_COMMAND,
                wParam: WPARAM(3 | (1 << 16)),
                ..Default::default()
            },
        );
        assert_eq!(
            runs.get(),
            2,
            "an accelerator-form command dispatches through the same path"
        );

        send_msg(
            &mut host,
            MSG {
                message: WM_COMMAND,
                wParam: WPARAM(3 | (2 << 16)),
                ..Default::default()
            },
        );
        assert_eq!(runs.get(), 2, "an unknown command form does not dispatch");

        send_msg(
            &mut host,
            MSG {
                message: WM_COMMAND,
                wParam: WPARAM(9999),
                ..Default::default()
            },
        );
        assert_eq!(runs.get(), 2, "an unknown id does not dispatch");

        send_msg(
            &mut host,
            MSG {
                message: WM_COMMAND,
                wParam: WPARAM(0),
                ..Default::default()
            },
        );
        assert_eq!(runs.get(), 2, "separator ids do not dispatch");
    }

    #[test]
    fn accelerator_commands_use_the_accelerator_source_and_refuse_disabled() {
        let (mut host, events) = accelerator_host();
        let hwnd = host.hwnd(MAIN).expect("a live window");
        let menu = unsafe { GetMenu(hwnd) };
        let save = unsafe { GetMenuItemID(menu, 0) };
        let toggle = unsafe { GetMenuItemID(menu, 1) };
        assert_eq!((save, toggle), (1, 2));
        let accelerator = host.windows[MAIN]
            .menu
            .as_ref()
            .and_then(WindowMenu::accelerator)
            .expect("two shortcuts create a table");
        assert_eq!(accelerator_entries(accelerator), 2);

        for wparam in [save, save | (1 << 16)] {
            send_command(&mut host, wparam);
        }
        assert_eq!(
            *events.borrow(),
            [CommandSource::MenuBar, CommandSource::Accelerator]
        );

        send_command(&mut host, toggle);
        for wparam in [save, save | (1 << 16)] {
            send_command(&mut host, wparam);
        }
        assert_eq!(
            events.borrow().len(),
            2,
            "a disabled command refuses both entry points"
        );

        send_command(&mut host, toggle | (1 << 16));
        send_command(&mut host, save | (1 << 16));
        assert_eq!(
            events.borrow().last(),
            Some(&CommandSource::Accelerator),
            "re-enabling restores the accelerator"
        );
        assert_eq!(events.borrow().len(), 3);
    }

    #[test]
    fn keyboard_messages_translate_through_the_accelerator_table() {
        let (mut host, events) = accelerator_host();
        let hwnd = host.hwnd(MAIN).expect("a live window");
        host.runtime.focus("edit").expect("focus the input");
        let edit = host
            .runtime
            .presentation()
            .expect("snapshot")
            .find_label("edit")
            .expect("input")
            .id();
        for character in ['a', 'b', 'c'] {
            send_msg(
                &mut host,
                MSG {
                    message: WM_CHAR,
                    wParam: WPARAM(character as usize),
                    ..Default::default()
                },
            );
        }

        let _control = ControlPressed::press();

        // 未声明的 Ctrl+A 继续走编辑器路径，即使窗口有 HACCEL。
        // 安全：消息目标是宿主持有的 HWND。
        unsafe { PostMessageW(Some(hwnd), WM_KEYDOWN, WPARAM(VK_A.0 as usize), LPARAM(0)) }
            .expect("post Ctrl+A");
        host.pump().expect("pump Ctrl+A");
        let selection = host
            .runtime
            .presentation()
            .expect("snapshot")
            .node(edit)
            .and_then(|element| element.edit())
            .and_then(|edit| edit.selection());
        assert_eq!(selection, Some((0, 3)), "Ctrl+A selects all");

        // 声明的 Ctrl+S 命中 HACCEL，只派发一次命令，不改文本或选择。
        // 安全：同上。
        unsafe { PostMessageW(Some(hwnd), WM_KEYDOWN, WPARAM(VK_S.0 as usize), LPARAM(0)) }
            .expect("post Ctrl+S");
        host.pump().expect("pump Ctrl+S");
        assert_eq!(
            *events.borrow(),
            [CommandSource::Accelerator],
            "one key message produces one accelerator command"
        );
        let selection = host
            .runtime
            .presentation()
            .expect("snapshot")
            .node(edit)
            .and_then(|element| element.edit())
            .and_then(|edit| edit.selection());
        assert_eq!(selection, Some((0, 3)), "the shortcut does not edit");
    }

    #[test]
    fn reopening_rebuilds_the_accelerator_table() {
        let (mut host, _events) = accelerator_host();
        let old_hwnd = host.hwnd(MAIN).expect("a live window");
        let table = host.windows[MAIN]
            .menu
            .as_ref()
            .and_then(WindowMenu::accelerator)
            .expect("a table");
        assert_eq!(accelerator_entries(table), 2);
        host.close(MAIN).expect("close the window");
        assert!(
            !host.windows.contains_key(MAIN),
            "the table dies with the window"
        );

        host.show(MAIN).expect("reopen the window");
        let hwnd = host.hwnd(MAIN).expect("a live window");
        assert_ne!(hwnd, old_hwnd);
        let table = host.windows[MAIN]
            .menu
            .as_ref()
            .and_then(WindowMenu::accelerator)
            .expect("the new activation rebuilds the table");
        assert_eq!(accelerator_entries(table), 2);
    }

    #[test]
    fn an_active_composition_shadows_the_accelerator() {
        let (mut host, events) = accelerator_host();
        let hwnd = host.hwnd(MAIN).expect("a live window");
        host.runtime.focus("edit").expect("focus the input");
        let edit = host
            .runtime
            .presentation()
            .expect("snapshot")
            .find_label("edit")
            .expect("input")
            .id();
        host.runtime
            .compose_start_node(edit)
            .expect("start the composition");
        host.runtime
            .compose_update_node(edit, "ni")
            .expect("update the composition");

        let _control = ControlPressed::press();
        // 安全：消息目标是宿主持有的 HWND。
        unsafe { PostMessageW(Some(hwnd), WM_KEYDOWN, WPARAM(VK_S.0 as usize), LPARAM(0)) }
            .expect("post Ctrl+S");
        host.pump().expect("pump Ctrl+S");
        assert!(
            events.borrow().is_empty(),
            "the IME composition consumes the key"
        );
        let composition = host
            .runtime
            .presentation()
            .expect("snapshot")
            .node(edit)
            .and_then(|element| element.edit())
            .and_then(|edit| edit.composition());
        assert!(composition.is_some(), "the composition survives");
    }

    #[test]
    fn a_composition_that_outlives_focus_still_shadows_the_accelerator() {
        let (mut host, events) = accelerator_host();
        let hwnd = host.hwnd(MAIN).expect("a live window");
        host.runtime.focus("edit").expect("focus the input");
        let snapshot = host.runtime.presentation().expect("snapshot");
        let edit = snapshot.find_label("edit").expect("input").id();
        let other = snapshot.find_label("other").expect("button").id();
        drop(snapshot);
        host.runtime
            .compose_start_node(edit)
            .expect("start the composition");
        host.runtime
            .compose_update_node(edit, "ni")
            .expect("update the composition");
        host.runtime
            .focus_node(other)
            .expect("move focus away from the composing input");

        let _control = ControlPressed::press();
        // 安全：消息目标是宿主持有的 HWND。
        unsafe { PostMessageW(Some(hwnd), WM_KEYDOWN, WPARAM(VK_S.0 as usize), LPARAM(0)) }
            .expect("post Ctrl+S");
        host.pump().expect("pump Ctrl+S");
        assert!(
            events.borrow().is_empty(),
            "a composition outside focus still shadows the accelerator"
        );
        let composition = host
            .runtime
            .presentation()
            .expect("snapshot")
            .node(edit)
            .and_then(|element| element.edit())
            .and_then(|edit| edit.composition());
        assert!(composition.is_some(), "the composition survives");
    }

    #[test]
    fn menu_commands_are_ignored_while_hidden_or_closing() {
        let (mut host, runs) = menu_host();
        host.hide(MAIN).expect("hide the menu window");
        send_msg(
            &mut host,
            MSG {
                message: WM_COMMAND,
                wParam: WPARAM(3),
                ..Default::default()
            },
        );
        assert_eq!(runs.get(), 0, "a hidden window runs nothing");
        host.show(MAIN).expect("show the menu window");

        let (mut closing, closing_runs) = menu_confirm_host();
        send_msg(
            &mut closing,
            MSG {
                message: WM_CLOSE,
                ..Default::default()
            },
        );
        assert!(closing.hwnd("confirm").is_some());
        send_msg(
            &mut closing,
            MSG {
                message: WM_COMMAND,
                wParam: WPARAM(1),
                ..Default::default()
            },
        );
        assert_eq!(
            closing_runs.get(),
            0,
            "a parent awaiting close confirmation runs nothing"
        );
    }

    #[test]
    fn entering_the_menu_loop_cancels_composition_and_pointer_transients() {
        let (mut host, _runs) = menu_host();
        host.runtime.focus("edit").expect("focus the input");
        let node = host
            .runtime
            .presentation()
            .expect("snapshot")
            .find_label("edit")
            .expect("input")
            .id();
        send_msg(
            &mut host,
            MSG {
                message: WM_IME_STARTCOMPOSITION,
                ..Default::default()
            },
        );
        assert!(
            host.runtime
                .presentation()
                .expect("snapshot")
                .node(node)
                .expect("input")
                .edit()
                .expect("edit")
                .composition()
                .is_some(),
            "the composition started"
        );
        let hwnd = host.hwnd(MAIN).expect("a live window");
        with_main(&mut host, |_host, window| {
            window.pointer = Some(Point::new(10.0, 10.0));
            window.hovered = Some(node);
            window.pressed = Some(node);
            window.scrollbar_drag = Some(2.0);
            window.tracking_leave = true;
            // 安全：捕获属于这个测试窗口；松开或窗口销毁时清除。
            unsafe {
                SetCapture(window.hwnd());
            }
            Ok(())
        });
        assert_eq!(
            // 安全：只查询当前线程的捕获窗口。
            unsafe { GetCapture() },
            hwnd,
            "the simulated press holds the pointer capture"
        );

        send_msg(
            &mut host,
            MSG {
                message: WM_ENTERMENULOOP,
                ..Default::default()
            },
        );

        assert!(
            host.runtime
                .presentation()
                .expect("snapshot")
                .node(node)
                .expect("input")
                .edit()
                .expect("edit")
                .composition()
                .is_none(),
            "the menu loop cancels the composition"
        );
        let window = &host.windows[MAIN];
        assert!(window.pointer.is_none());
        assert!(window.hovered.is_none());
        assert!(window.pressed.is_none());
        assert!(window.scrollbar_drag.is_none());
        assert!(!window.tracking_leave);
        assert_ne!(
            // 安全：只查询当前线程的捕获窗口。
            unsafe { GetCapture() },
            hwnd,
            "the menu loop releases the pointer capture"
        );
    }

    #[test]
    fn menu_bar_keeps_the_logical_client_size_at_96_and_144_dpi() {
        let (mut host, _runs) = menu_host();
        host.resize(MAIN, 640, 480).expect("resize the menu window");
        let hwnd = host.hwnd(MAIN).expect("a live window");
        let (client_width, client_height) = client_size(hwnd).expect("client size");
        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
        let scale = dpi as f32 / 96.0;
        assert_eq!(
            (client_width, client_height),
            ((640.0 * scale) as u32, (480.0 * scale) as u32),
            "resize keeps the logical client size with a menu bar"
        );

        for new_dpi in [96, 144] {
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
            let (width, height) =
                scaled_client_size(client_width as i32, client_height as i32, dpi, new_dpi);
            let mut with_menu = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };
            let mut without_menu = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };
            // 安全：rect 是本地值；样式与窗口创建时一致。
            unsafe {
                AdjustWindowRectExForDpi(
                    &mut with_menu,
                    WS_OVERLAPPEDWINDOW,
                    true,
                    WINDOW_EX_STYLE::default(),
                    new_dpi,
                )
                .expect("menu frame metrics");
                AdjustWindowRectExForDpi(
                    &mut without_menu,
                    WS_OVERLAPPEDWINDOW,
                    false,
                    WINDOW_EX_STYLE::default(),
                    new_dpi,
                )
                .expect("plain frame metrics");
            }
            assert_eq!(
                (size.cx, size.cy),
                (
                    with_menu.right - with_menu.left,
                    with_menu.bottom - with_menu.top
                ),
                "the candidate outer size at {new_dpi} DPI includes the menu bar"
            );
            assert!(
                with_menu.bottom - with_menu.top > without_menu.bottom - without_menu.top,
                "the menu adds non-client height at {new_dpi} DPI"
            );
        }
    }

    #[test]
    fn menu_handle_dies_with_the_window() {
        let (mut host, _runs) = menu_host();
        let hwnd = host.hwnd(MAIN).expect("a live window");
        let menu = unsafe { GetMenu(hwnd) };
        assert!(!menu.is_invalid());
        host.close(MAIN).expect("close the window");
        assert!(
            // 安全：只查询句柄是否仍是一个菜单；不访问已销毁窗口。
            !unsafe { IsMenu(menu) }.as_bool(),
            "the assigned menu dies with its window"
        );
    }

    fn menu_state_at(menu: HMENU, id: u32) -> u32 {
        // 安全：id 来自同一窗口拥有的菜单。
        unsafe { GetMenuState(menu, id, MF_BYCOMMAND) }
    }

    fn assert_menu_enabled(menu: HMENU, id: u32, expected: bool) {
        let state = menu_state_at(menu, id);
        assert_eq!(
            state & MF_GRAYED.0 == 0,
            expected,
            "menu item {id} enabled state: {state:#x}"
        );
    }

    fn assert_menu_checked(menu: HMENU, id: u32, expected: bool) {
        let state = menu_state_at(menu, id);
        assert_eq!(
            state & MF_CHECKED.0 != 0,
            expected,
            "menu item {id} checked state: {state:#x}"
        );
    }

    #[test]
    fn menu_init_messages_reapply_committed_state() {
        let (mut host, runs) = menu_state_host();
        let hwnd = host.hwnd(MAIN).expect("a live window");
        let menu = unsafe { GetMenu(hwnd) };
        let file = unsafe { GetSubMenu(menu, 0) };
        let view = unsafe { GetSubMenu(menu, 1) };
        // 安全：位置在菜单项范围内。
        let clear = unsafe { GetMenuItemID(file, 0) };
        let status = unsafe { GetMenuItemID(view, 0) };
        let toggle = unsafe { GetMenuItemID(view, 1) };
        let about = unsafe { GetMenuItemID(menu, 2) };
        assert_eq!((clear, status, toggle, about), (1, 2, 3, 4));

        send_msg(
            &mut host,
            MSG {
                message: WM_INITMENU,
                wParam: WPARAM(menu.0 as usize),
                ..Default::default()
            },
        );
        assert_menu_enabled(menu, about, true);
        send_msg(
            &mut host,
            MSG {
                message: WM_INITMENUPOPUP,
                wParam: WPARAM(file.0 as usize),
                ..Default::default()
            },
        );
        assert_menu_enabled(file, clear, true);
        send_msg(
            &mut host,
            MSG {
                message: WM_INITMENUPOPUP,
                wParam: WPARAM(view.0 as usize),
                ..Default::default()
            },
        );
        assert_menu_checked(view, status, true);
        assert_menu_enabled(view, toggle, true);

        send_msg(
            &mut host,
            MSG {
                message: WM_COMMAND,
                wParam: WPARAM(toggle as usize),
                ..Default::default()
            },
        );
        send_msg(
            &mut host,
            MSG {
                message: WM_INITMENU,
                wParam: WPARAM(menu.0 as usize),
                ..Default::default()
            },
        );
        assert_menu_enabled(menu, about, false);
        send_msg(
            &mut host,
            MSG {
                message: WM_INITMENUPOPUP,
                wParam: WPARAM(file.0 as usize),
                ..Default::default()
            },
        );
        assert_menu_enabled(file, clear, false);
        send_msg(
            &mut host,
            MSG {
                message: WM_INITMENUPOPUP,
                wParam: WPARAM(view.0 as usize),
                ..Default::default()
            },
        );
        assert_menu_checked(view, status, false);
        assert_menu_enabled(view, toggle, true);

        send_msg(
            &mut host,
            MSG {
                message: WM_COMMAND,
                wParam: WPARAM(clear as usize),
                ..Default::default()
            },
        );
        assert_eq!(runs.get(), 0, "a disabled item cannot dispatch");

        send_msg(
            &mut host,
            MSG {
                message: WM_COMMAND,
                wParam: WPARAM(toggle as usize),
                ..Default::default()
            },
        );
        send_msg(
            &mut host,
            MSG {
                message: WM_INITMENU,
                wParam: WPARAM(menu.0 as usize),
                ..Default::default()
            },
        );
        assert_menu_enabled(menu, about, true);
        send_msg(
            &mut host,
            MSG {
                message: WM_INITMENUPOPUP,
                wParam: WPARAM(file.0 as usize),
                ..Default::default()
            },
        );
        assert_menu_enabled(file, clear, true);
        send_msg(
            &mut host,
            MSG {
                message: WM_COMMAND,
                wParam: WPARAM(clear as usize),
                ..Default::default()
            },
        );
        assert_eq!(runs.get(), 1, "an enabled item dispatches");
    }

    #[test]
    fn hidden_windows_leave_menu_items_unchanged() {
        let (mut host, _runs) = menu_state_host();
        let hwnd = host.hwnd(MAIN).expect("a live window");
        let menu = unsafe { GetMenu(hwnd) };
        let view = unsafe { GetSubMenu(menu, 1) };
        let status = unsafe { GetMenuItemID(view, 0) };
        let toggle = unsafe { GetMenuItemID(view, 1) };
        let about = unsafe { GetMenuItemID(menu, 2) };

        send_msg(
            &mut host,
            MSG {
                message: WM_INITMENU,
                wParam: WPARAM(menu.0 as usize),
                ..Default::default()
            },
        );
        send_msg(
            &mut host,
            MSG {
                message: WM_INITMENUPOPUP,
                wParam: WPARAM(view.0 as usize),
                ..Default::default()
            },
        );
        assert_menu_enabled(menu, about, true);
        assert_menu_checked(view, status, true);

        send_msg(
            &mut host,
            MSG {
                message: WM_COMMAND,
                wParam: WPARAM(toggle as usize),
                ..Default::default()
            },
        );
        host.hide(MAIN).expect("hide the menu window");
        send_msg(
            &mut host,
            MSG {
                message: WM_INITMENU,
                wParam: WPARAM(menu.0 as usize),
                ..Default::default()
            },
        );
        send_msg(
            &mut host,
            MSG {
                message: WM_INITMENUPOPUP,
                wParam: WPARAM(view.0 as usize),
                ..Default::default()
            },
        );
        assert_menu_enabled(menu, about, true);
        assert_menu_checked(view, status, true);

        host.show(MAIN).expect("show the menu window");
        send_msg(
            &mut host,
            MSG {
                message: WM_INITMENU,
                wParam: WPARAM(menu.0 as usize),
                ..Default::default()
            },
        );
        send_msg(
            &mut host,
            MSG {
                message: WM_INITMENUPOPUP,
                wParam: WPARAM(view.0 as usize),
                ..Default::default()
            },
        );
        assert_menu_enabled(menu, about, false);
        assert_menu_checked(view, status, false);
    }

    #[test]
    fn menu_state_errors_leave_the_menu_unchanged() {
        let mut runtime = Runtime::new();
        let checked = runtime
            .setup(|| signal(true))
            .expect("menu state error setup");
        runtime
            .declare_windows(move |catalog| {
                catalog.window(WindowSpec::new(MAIN).primary(), move |window| {
                    window
                        .on_command("file.clear", |_| {})
                        .expect("register file.clear");
                    window
                        .on_menu_state(move |command| {
                            checked.set(false);
                            command.default_state()
                        })
                        .expect("register menu state");
                    view! { <view /> }
                })?;
                catalog.menu(
                    MAIN,
                    MenuSpec::new().submenu(
                        "File",
                        MenuSpec::new().item(
                            MenuCommand::new("file.clear", "Clear")
                                .enabled(false)
                                .checked(true),
                        ),
                    ),
                )?;
                Ok(())
            })
            .expect("declare the menu state error host");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open the host");
        let hwnd = host.hwnd(MAIN).expect("a live window");
        let menu = unsafe { GetMenu(hwnd) };
        let file = unsafe { GetSubMenu(menu, 0) };
        let clear = unsafe { GetMenuItemID(file, 0) };
        let before = menu_state_at(file, clear);
        assert_ne!(before & MF_GRAYED.0, 0, "the declared disabled state");
        assert_ne!(before & MF_CHECKED.0, 0, "the declared checked state");

        host.handle_message(hwnd, WM_INITMENUPOPUP, WPARAM(file.0 as usize), LPARAM(0));
        match host.deferred_error.take() {
            Some(HostError::MenuState(MenuStateError::Context {
                operation: "Signal::set",
                ..
            })) => {}
            other => panic!("the state error must surface typed: {other:?}"),
        }
        assert_eq!(
            menu_state_at(file, clear),
            before,
            "a failed callback leaves the native menu unchanged"
        );
    }

    #[test]
    fn reopening_rebuilds_the_menu_and_reregisters_state() {
        let (mut host, _runs) = menu_state_host();
        let old_hwnd = host.hwnd(MAIN).expect("a live window");
        let old_menu = unsafe { GetMenu(old_hwnd) };
        assert!(!old_menu.is_invalid());
        host.close(MAIN).expect("close the window");
        assert!(
            // 安全：只查询旧菜单句柄。
            !unsafe { IsMenu(old_menu) }.as_bool(),
            "the old menu dies with its window"
        );

        host.show(MAIN).expect("reopen the window");
        let hwnd = host.hwnd(MAIN).expect("a live window");
        assert_ne!(hwnd, old_hwnd, "reopening creates a new native window");
        let menu = unsafe { GetMenu(hwnd) };
        let file = unsafe { GetSubMenu(menu, 0) };
        let view = unsafe { GetSubMenu(menu, 1) };
        let clear = unsafe { GetMenuItemID(file, 0) };
        let status = unsafe { GetMenuItemID(view, 0) };
        let toggle = unsafe { GetMenuItemID(view, 1) };

        send_msg(
            &mut host,
            MSG {
                message: WM_INITMENUPOPUP,
                wParam: WPARAM(view.0 as usize),
                ..Default::default()
            },
        );
        assert_menu_checked(view, status, true);
        assert_menu_enabled(view, toggle, true);

        send_msg(
            &mut host,
            MSG {
                message: WM_COMMAND,
                wParam: WPARAM(toggle as usize),
                ..Default::default()
            },
        );
        send_msg(
            &mut host,
            MSG {
                message: WM_INITMENUPOPUP,
                wParam: WPARAM(file.0 as usize),
                ..Default::default()
            },
        );
        assert_menu_enabled(file, clear, false);
        send_msg(
            &mut host,
            MSG {
                message: WM_INITMENUPOPUP,
                wParam: WPARAM(view.0 as usize),
                ..Default::default()
            },
        );
        assert_menu_checked(view, status, false);
    }

    #[test]
    fn an_empty_menu_declaration_creates_no_native_menu_bar() {
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(|catalog| {
                catalog.window(WindowSpec::new(MAIN).primary(), |_| view! { <view /> })?;
                catalog.menu(MAIN, MenuSpec::new())?;
                Ok(())
            })
            .expect("declare the empty menu");
        let host = Host::open(runtime, HostConfig::hidden()).expect("open the host");
        let hwnd = host.hwnd(MAIN).expect("a live window");
        assert!(
            // 安全：只查询宿主持有的 HWND 的菜单。
            unsafe { GetMenu(hwnd) }.is_invalid(),
            "an empty declaration leaves the HWND without a native menu bar"
        );
    }

    #[test]
    fn context_menu_pointer_and_keyboard_pass_the_same_declaration() {
        let ContextMenuHarness {
            mut host,
            records,
            runs,
        } = context_menu_host(Some("edit.copy"), None, false);
        let hwnd = host.hwnd(MAIN).expect("a live window");
        let window = host.windows[MAIN].ctx;
        let keyboard_point = client_center(hwnd);

        assert!(send_context_menu(&mut host, Some((120, 80))).is_none());
        assert!(send_context_menu(&mut host, None).is_none());

        let records = records.borrow();
        assert_eq!(records.len(), 2, "each open reaches the presenter once");
        assert_eq!(records[0].window, window);
        assert_eq!(records[0].hwnd, hwnd);
        assert_eq!(records[0].point, (120, 80));
        assert_eq!(records[0].menu, context_menu_declaration());
        assert_eq!(
            records[1].point, keyboard_point,
            "the keyboard route opens at the client center"
        );
        assert_eq!(
            records[1].menu,
            context_menu_declaration(),
            "both routes share one declaration"
        );
        assert_eq!(
            runs.get(),
            2,
            "the returned command dispatches once per open, without a second WM_COMMAND"
        );
    }

    #[test]
    fn cancelling_the_context_menu_dispatches_nothing_and_restores_the_presenter() {
        let ContextMenuHarness {
            mut host,
            records,
            runs,
        } = context_menu_host(None, None, false);
        assert!(send_context_menu(&mut host, Some((10, 10))).is_none());
        assert_eq!(records.borrow().len(), 1);
        assert_eq!(runs.get(), 0, "a cancel dispatches no command");
        assert!(
            host.context_menu_presenter.is_some(),
            "the presenter returns after a cancel"
        );

        assert!(send_context_menu(&mut host, Some((12, 14))).is_none());
        assert_eq!(
            records.borrow().len(),
            2,
            "the next open reaches the restored presenter"
        );
    }

    #[test]
    fn a_worker_wake_during_the_open_menu_is_delivered() {
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(|catalog| {
                catalog.window(WindowSpec::new(MAIN).primary(), |window| {
                    window
                        .on_command("edit.copy", |_| {})
                        .expect("register edit.copy");
                    window
                        .on_command("edit.paste", |_| {})
                        .expect("register edit.paste");
                    view! {
                        <view>
                            <button
                                aria:label="start"
                                on:click={move || {
                                    let task = window
                                        .task_scope()
                                        .spawn(|_| String::from("ready"))
                                        .expect("spawn the task");
                                    task.on_done(move |_| {
                                        window.request_show("details").expect("queue the show");
                                        Ok(())
                                    })
                                    .expect("register the continuation");
                                }}
                            >"Start"</button>
                        </view>
                    }
                })?;
                catalog.window(
                    WindowSpec::new("details"),
                    |_| view! { <text>"details"</text> },
                )?;
                catalog.context_menu(MAIN, context_menu_declaration())?;
                Ok(())
            })
            .expect("declare the wake host");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open the wake host");
        let records = Rc::new(RefCell::new(Vec::new()));
        host.set_context_menu_presenter(FakeContextMenuPresenter {
            records: records.clone(),
            result: None,
            panic_first: Rc::new(Cell::new(false)),
            during_show: Some(Box::new(|hwnd, _| {
                // 菜单自己的模态循环不返回外层消息循环；这里同步投递唤醒，
                // 重入窗口过程交付任务结果。
                // 安全：hwnd 由宿主持有；SendMessageW 在本线程同步派发。
                unsafe {
                    SendMessageW(hwnd, host_message::WAKE, Some(WPARAM(0)), Some(LPARAM(0)));
                }
            })),
        });
        host.runtime.click_label("start").expect("start the task");
        host.runtime.wait_for_tasks();
        assert!(
            host.hwnd("details").is_none(),
            "the request waits for a wake delivery"
        );

        assert!(send_context_menu(&mut host, Some((30, 40))).is_none());
        assert_eq!(records.borrow().len(), 1, "the popup opened");
        assert!(
            host.hwnd("details").is_some(),
            "the wake delivered during the popup opened its window"
        );
        assert!(host.scene("details").is_some(), "the new window paints");
    }

    #[test]
    fn closing_the_owner_during_the_open_menu_discards_the_command() {
        let ContextMenuHarness {
            mut host,
            records,
            runs,
        } = context_menu_host(
            Some("edit.copy"),
            Some(Box::new(|hwnd, _| {
                // 弹窗期间用户关闭 owner：同步派发 WM_CLOSE 重入窗口过程。
                // 安全：hwnd 由宿主持有；SendMessageW 在本线程同步派发。
                unsafe {
                    SendMessageW(hwnd, WM_CLOSE, Some(WPARAM(0)), Some(LPARAM(0)));
                }
            })),
            false,
        );
        assert!(send_context_menu(&mut host, Some((15, 25))).is_none());
        assert_eq!(records.borrow().len(), 1);
        assert_eq!(
            runs.get(),
            0,
            "a command chosen for a closed owner is discarded"
        );
        assert!(host.hwnd(MAIN).is_none(), "the owner window is gone");
        assert!(host.runtime.windows().is_empty());
        assert!(
            host.context_menu_presenter.is_some(),
            "the presenter returns even when the owner closes"
        );
    }

    #[test]
    fn a_panicking_context_menu_presenter_is_contained_and_restored() {
        let ContextMenuHarness {
            mut host,
            records,
            runs,
        } = context_menu_host(Some("edit.copy"), None, true);
        let error = send_context_menu(&mut host, Some((5, 5)))
            .expect("the panic cannot cross the window procedure boundary");
        assert!(
            matches!(error, HostError::ContextMenuPresenter(_)),
            "{error}"
        );
        assert_eq!(runs.get(), 0);
        assert!(
            host.context_menu_presenter.is_some(),
            "the presenter returns after a contained panic"
        );

        assert!(send_context_menu(&mut host, Some((5, 5))).is_none());
        assert_eq!(
            records.borrow().len(),
            1,
            "the second open reaches the restored presenter"
        );
        assert_eq!(runs.get(), 1);
    }

    #[test]
    fn an_undeclared_presenter_command_is_a_host_error() {
        let ContextMenuHarness {
            mut host,
            records,
            runs,
        } = context_menu_host(Some("edit.unknown"), None, false);
        let error = send_context_menu(&mut host, Some((5, 5)))
            .expect("a presenter must return a declared command id");
        assert!(
            matches!(
                error,
                HostError::Command(CommandError::UnknownCommand("edit.unknown"))
            ),
            "{error}"
        );
        assert_eq!(runs.get(), 0);
        assert_eq!(records.borrow().len(), 1, "the presenter saw the request");
        assert!(
            host.context_menu_presenter.is_some(),
            "the presenter returns after an invalid command"
        );
    }

    #[test]
    fn context_popup_ignores_menu_bar_state_with_colliding_numeric_ids() {
        let menu_bar = MenuSpec::new()
            .command("file.clear", "Clear")
            .command("view.status", "Show status");
        let context = MenuSpec::new()
            .command("edit.copy", "Copy")
            .command("edit.paste", "Paste");
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(move |catalog| {
                catalog.window(WindowSpec::new(MAIN).primary(), |window| {
                    for id in ["file.clear", "view.status", "edit.copy", "edit.paste"] {
                        window.on_command(id, |_| {}).expect("register");
                    }
                    window
                        .on_menu_state(|command| match command.id() {
                            "file.clear" => MenuCommandState::new(false, false),
                            "view.status" => MenuCommandState::new(true, true),
                            _ => command.default_state(),
                        })
                        .expect("register menu state");
                    view! { <view /> }
                })?;
                catalog.menu(MAIN, menu_bar)?;
                catalog.context_menu(MAIN, context)?;
                Ok(())
            })
            .expect("declare the colliding menus");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open the host");
        let records = Rc::new(RefCell::new(Vec::new()));
        let observed = Rc::new(Cell::new(None));
        let probe = observed.clone();
        host.set_context_menu_presenter(FakeContextMenuPresenter {
            records: records.clone(),
            result: None,
            panic_first: Rc::new(Cell::new(false)),
            during_show: Some(Box::new(move |hwnd, menu| {
                let native = NativeMenu::build_popup(menu).expect("build the popup");
                // 安全：句柄由本闭包持有；只读取菜单项状态。
                let before = unsafe {
                    (
                        GetMenuState(native.handle(), 1, MF_BYCOMMAND),
                        GetMenuState(native.handle(), 2, MF_BYCOMMAND),
                    )
                };
                // 原生菜单的模态循环会向 owner 发送这条初始化消息。
                // 安全：句柄由本闭包持有；SendMessageW 在本线程同步派发。
                unsafe {
                    SendMessageW(
                        hwnd,
                        WM_INITMENUPOPUP,
                        Some(WPARAM(native.handle().0 as usize)),
                        Some(LPARAM(0)),
                    );
                }
                // 安全：句柄由本闭包持有；只读取菜单项状态。
                let after = unsafe {
                    (
                        GetMenuState(native.handle(), 1, MF_BYCOMMAND),
                        GetMenuState(native.handle(), 2, MF_BYCOMMAND),
                    )
                };
                probe.set(Some((before, after)));
            })),
        });

        assert!(send_context_menu(&mut host, Some((10, 10))).is_none());
        assert_eq!(records.borrow().len(), 1, "the popup opened once");
        let Some(((before_copy, before_paste), (after_copy, after_paste))) = observed.get() else {
            panic!("the presenter did not probe the popup");
        };
        assert_eq!(
            (before_copy, before_paste),
            (after_copy, after_paste),
            "the menu-bar state callback must not change the context popup"
        );
        assert_eq!(
            after_copy & MF_GRAYED.0,
            0,
            "popup id 1 stays enabled although menu-bar id 1 is disabled"
        );
        assert_eq!(
            after_paste & MF_CHECKED.0,
            0,
            "popup id 2 stays unchecked although menu-bar id 2 is checked"
        );
    }

    #[test]
    fn a_hidden_window_does_not_open_the_context_menu() {
        let ContextMenuHarness {
            mut host,
            records,
            runs,
        } = context_menu_host(Some("edit.copy"), None, false);
        let window = host.windows[MAIN].ctx;
        host.runtime.hide(window).expect("hide the window");
        assert!(send_context_menu(&mut host, Some((10, 10))).is_none());
        assert!(records.borrow().is_empty(), "a hidden window opens nothing");
        assert_eq!(runs.get(), 0);
        assert!(
            host.context_menu_presenter.is_some(),
            "the presenter was never taken"
        );
    }

    #[test]
    fn a_hidden_host_never_installs_the_native_context_menu_presenter() {
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(|catalog| {
                catalog.window(WindowSpec::new(MAIN).primary(), |window| {
                    window
                        .on_command("edit.copy", |_| {})
                        .expect("register edit.copy");
                    window
                        .on_command("edit.paste", |_| {})
                        .expect("register edit.paste");
                    view! { <view /> }
                })?;
                catalog.context_menu(MAIN, context_menu_declaration())?;
                Ok(())
            })
            .expect("declare the hidden context menu host");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open the host");
        assert!(
            host.context_menu_presenter.is_none(),
            "hidden CI installs no native presenter"
        );
        assert!(send_context_menu(&mut host, Some((3, 4))).is_none());
        assert!(host.context_menu_presenter.is_none());
    }

    /// 与原生原型共用的公有领域测试字体。
    const TUFFY: &[u8] = include_bytes!("../../../examples/native_poc/assets/Tuffy.ttf");

    #[test]
    fn invalid_text_assets_fail_before_a_window_is_created() {
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(|catalog| {
                catalog.window(
                    WindowSpec::new(MAIN)
                        .primary()
                        .titled("ZenUI Bad Assets")
                        .sized(320, 240),
                    |_| view! { <text>"assets"</text> },
                )?;
                Ok(())
            })
            .expect("declare the catalog");
        let error = Host::open(
            runtime,
            HostConfig::hidden().with_text_assets(
                TextAssets::new("Missing").with_font(FontAsset::new(b"not a font")),
            ),
        )
        .err()
        .expect("invalid assets are a startup error");
        assert!(matches!(error, HostError::TextAssets(_)), "{error}");
        // 安全：只按标题查询本进程窗口；标题只属于这个测试。
        let found = unsafe { FindWindowW(w!("ZenUI Bad Assets"), None) };
        assert!(found.is_err(), "the failed host created no native window");
    }

    #[test]
    fn the_configured_font_shapes_the_rendered_window() {
        let mut runtime = Runtime::new();
        zenui_demo::open_todo_window(&mut runtime).expect("open the todo window");
        let mut host = Host::open(
            runtime,
            HostConfig::hidden()
                .with_text_assets(TextAssets::new("Tuffy").with_font(FontAsset::new(TUFFY))),
        )
        .expect("open the host with fixed font assets");
        host.render().expect("render the window");

        let scene = host
            .windows
            .get(&MAIN)
            .and_then(|window| window.scene.as_ref())
            .expect("the rendered scene");
        let mut runs = 0;
        for element in scene.elements() {
            for run in element.text() {
                runs += 1;
                assert_eq!(
                    run.font().data.as_ref(),
                    TUFFY,
                    "the configured font shaped the window text"
                );
            }
        }
        assert!(runs > 0, "the demo window has text runs");
    }

    #[test]
    fn an_idle_unfocused_window_neither_rebuilds_nor_presents() {
        let mut host = open_host();
        host.render().expect("the first frame");
        let before = host.counters;
        std::thread::sleep(Duration::from_millis(1200));
        host.pump().expect("pump idle messages");
        assert_eq!(host.counters, before, "an idle window schedules no work");
    }

    #[test]
    fn an_idle_focused_window_presents_only_caret_blinks() {
        let mut host = open_host();
        host.runtime.focus("What needs to be done?").expect("focus");
        with_main(&mut host, |_, window| {
            window.foreground = Some(true);
            Ok(())
        });
        host.render().expect("the first frame");
        let before = host.counters;
        std::thread::sleep(Duration::from_millis(1200));
        host.pump().expect("pump idle messages");
        let presents = host.counters.presents - before.presents;
        assert!(
            presents <= 3,
            "only caret blinks present while idle, got {presents} presents"
        );
        assert_eq!(
            host.counters.scene_builds - before.scene_builds,
            presents,
            "each caret blink builds from the cached snapshot and presents once"
        );
    }

    #[test]
    fn a_rendered_change_rebuilds_and_presents() {
        let mut host = open_host();
        host.render().expect("the first frame");
        let before = host.counters;
        host.runtime
            .input("What needs to be done?", "idle probe")
            .expect("type into the draft");
        host.render().expect("the changed frame");
        assert!(
            host.counters.scene_builds > before.scene_builds,
            "a changed snapshot rebuilds the scene"
        );
        assert!(
            host.counters.presents > before.presents,
            "a changed snapshot presents"
        );
    }

    /// 一个带行工厂计数器的窗口化列表；测试据此区分纯场景构建与结构 turn。
    #[component]
    fn HostScrollingRows(runs: &'static AtomicUsize) -> MountResult<MountPlan> {
        let store = Store::new();
        let rows = memo(move || store.rows(Filter::All));
        view! {
            <view>
                <button aria:label="Seed" on:click={move || -> Result<(), WriteError> {
                    let mut transaction = Transaction::new();
                    for index in 0..100 {
                        transaction = transaction.write(store.insert(format!("r{index}")));
                    }
                    transaction.finish()
                }}>"Seed"</button>
                <For each={rows} store={store} key={|row: &TodoRow| row.id} row-height={px(20.0)}>
                    {move |_row: RowView<TodoRow>| {
                        runs.fetch_add(1, Ordering::SeqCst);
                        view! { <text>"row"</text> }
                    }}
                </For>
            </view>
        }
    }

    /// 行的 blur 处理器关闭包住整个列表的 Show：blur 后第一遍场景的
    /// 行带已经失效，宿主必须重建并重试，而不是把滚动变成错误。
    #[component]
    fn BlurClosesList() -> MountResult<MountPlan> {
        let store = Store::new();
        let rows = memo(move || store.rows(Filter::All));
        let visible = signal(true);
        view! {
            <view>
                <button aria:label="Seed" on:click={move || -> Result<(), WriteError> {
                    let mut transaction = Transaction::new();
                    for index in 0..100 {
                        transaction = transaction.write(store.insert(format!("r{index}")));
                    }
                    transaction.finish()
                }}>"Seed"</button>
                <Show when={visible}>
                    <For each={rows} store={store} key={|row: &TodoRow| row.id} row-height={px(20.0)}>
                        {move |row: RowView<TodoRow>| view! {
                            <button
                                aria:label={once(format!("row {}", row.key().raw()))}
                                on:blur={move |_event: BlurEvent| { visible.set(false); }}
                            >"row"</button>
                        }}
                    </For>
                </Show>
            </view>
        }
    }

    #[test]
    fn a_blur_that_unmounts_the_list_retries_the_stale_row_window_batch() {
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(|catalog| {
                catalog.window(WindowSpec::new(MAIN).primary().sized(480, 240), |_| {
                    view! { <BlurClosesList /> }
                })
            })
            .expect("declare the window");
        runtime.show(MAIN).expect("show the window");
        runtime.click_label("Seed").expect("seed one hundred rows");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open the host");
        host.render().expect("the first frame");
        let label = host
            .runtime
            .presentation_of(host.windows[MAIN].ctx)
            .expect("snapshot")
            .elements()
            .find_map(|element| {
                element
                    .label()
                    .filter(|label| label.starts_with("row "))
                    .map(str::to_owned)
            })
            .expect("a mounted row button");
        host.runtime.focus(&label).expect("focus the row");

        host.windows.get_mut(MAIN).expect("window").scroll_offset = 400;
        host.render()
            .expect("the blur-rewritten batch is retried instead of failing the frame");
        let snapshot = host
            .runtime
            .presentation_of(host.windows[MAIN].ctx)
            .expect("snapshot");
        assert!(
            snapshot.row_bands().is_empty(),
            "the blur handler closed the Show around the windowed list"
        );
        assert!(
            snapshot.elements().all(|element| element
                .label()
                .is_none_or(|label| !label.starts_with("row "))),
            "the list rows are unmounted after the blur"
        );
    }

    /// 两个窗口化列表：第一个在 Show 里，行的 blur 关闭它；第二个继续存在。
    #[component]
    fn BlurHidesFirstList() -> MountResult<MountPlan> {
        let store = Store::new();
        let rows = memo(move || store.rows(Filter::All));
        let first_visible = signal(true);
        view! {
            <view>
                <button aria:label="Seed" on:click={move || -> Result<(), WriteError> {
                    let mut transaction = Transaction::new();
                    for index in 0..100 {
                        transaction = transaction.write(store.insert(format!("r{index}")));
                    }
                    transaction.finish()
                }}>"Seed"</button>
                <Show when={first_visible}>
                    <For each={rows} store={store} key={|row: &TodoRow| row.id} row-height={px(20.0)}>
                        {move |row: RowView<TodoRow>| view! {
                            <button
                                aria:label={once(format!("first {}", row.key().raw()))}
                                on:blur={move |_event: BlurEvent| { first_visible.set(false); }}
                            >"first"</button>
                        }}
                    </For>
                </Show>
                <For each={rows} store={store} key={|row: &TodoRow| row.id} row-height={px(20.0)}>
                    {move |row: RowView<TodoRow>| view! {
                        <text>{once(format!("second {}", row.key().raw()))}</text>
                    }}
                </For>
            </view>
        }
    }

    #[test]
    fn a_blur_that_removes_one_band_retries_the_surviving_band() {
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(|catalog| {
                catalog.window(WindowSpec::new(MAIN).primary().sized(480, 240), |_| {
                    view! { <BlurHidesFirstList /> }
                })
            })
            .expect("declare the window");
        runtime.show(MAIN).expect("show the window");
        runtime.click_label("Seed").expect("seed one hundred rows");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open the host");
        host.render().expect("the first frame");
        let label = host
            .runtime
            .presentation_of(host.windows[MAIN].ctx)
            .expect("snapshot")
            .elements()
            .find_map(|element| {
                element
                    .label()
                    .filter(|label| label.starts_with("first "))
                    .map(str::to_owned)
            })
            .expect("a mounted first-list row");
        host.runtime.focus(&label).expect("focus the row");

        let before = host.counters;
        host.windows.get_mut(MAIN).expect("window").scroll_offset = 400;
        host.render()
            .expect("the surviving band applies after the stale batch is retried");
        assert_eq!(
            host.counters.scene_builds - before.scene_builds,
            3,
            "a blur rewrite rebuilds pass 1 once and then the changed pass 2"
        );
        let snapshot = host
            .runtime
            .presentation_of(host.windows[MAIN].ctx)
            .expect("snapshot");
        assert_eq!(
            snapshot.row_bands().len(),
            1,
            "the blur handler closed the first list"
        );
        assert!(
            snapshot.row_bands()[0].first_row() > 0,
            "the retried batch applied to the surviving list"
        );
        assert!(
            snapshot.elements().all(|element| element
                .label()
                .is_none_or(|label| !label.starts_with("first "))),
            "the first list's rows are unmounted"
        );
    }

    /// 行工厂在 armed 时拒绝任何新挂载的行。
    #[component]
    fn HostFailingRows(armed: &'static AtomicBool) -> MountResult<MountPlan> {
        let store = Store::new();
        let rows = memo(move || store.rows(Filter::All));
        view! {
            <view>
                <button aria:label="Seed" on:click={move || -> Result<(), WriteError> {
                    let mut transaction = Transaction::new();
                    for index in 0..100 {
                        transaction = transaction.write(store.insert(format!("r{index}")));
                    }
                    transaction.finish()
                }}>"Seed"</button>
                <For each={rows} store={store} key={|row: &TodoRow| row.id} row-height={px(20.0)}>
                    {move |_row: RowView<TodoRow>| {
                        if armed.load(Ordering::SeqCst) {
                            return Err(MountError::Component("row boom"));
                        }
                        view! { <text>"row"</text> }
                    }}
                </For>
            </view>
        }
    }

    #[test]
    fn a_row_factory_failure_during_scroll_keeps_the_host_alive() {
        let armed = leak(AtomicBool::new(false));
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(move |catalog| {
                catalog.window(WindowSpec::new(MAIN).primary().sized(480, 240), move |_| {
                    view! { <HostFailingRows armed={armed} /> }
                })
            })
            .expect("declare the window");
        runtime.show(MAIN).expect("show the window");
        runtime.click_label("Seed").expect("seed one hundred rows");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open the host");
        host.render().expect("the first frame");
        assert!(host.take_diagnostics().is_empty());

        armed.store(true, Ordering::SeqCst);
        host.windows.get_mut(MAIN).expect("window").scroll_offset = 400;
        host.render()
            .expect("a row factory failure stays a recoverable diagnostic");
        let diagnostics = host.take_diagnostics();
        assert!(
            diagnostics
                .iter()
                .any(|message| message.contains("row window sync failed")),
            "the failure is reported instead of killing the session: {diagnostics:?}"
        );
        let snapshot = host
            .runtime
            .presentation_of(host.windows[MAIN].ctx)
            .expect("snapshot");
        assert_eq!(
            snapshot.row_bands()[0].first_row(),
            0,
            "the runtime rolled back to the committed window"
        );
        assert_eq!(
            host.scene(MAIN).expect("scene").scroll_offset(),
            400,
            "the host keeps rendering at the scrolled offset"
        );

        armed.store(false, Ordering::SeqCst);
        host.render().expect("the healed factory applies");
        assert!(
            host.runtime
                .presentation_of(host.windows[MAIN].ctx)
                .expect("snapshot")
                .row_bands()[0]
                .first_row()
                > 0,
            "the next frame applies the requested window once the factory heals"
        );
    }

    #[test]
    fn scene_building_defers_row_window_factories_to_the_structure_turn() {
        let runs = leak(AtomicUsize::new(0));
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(move |catalog| {
                catalog.window(WindowSpec::new(MAIN).primary().sized(480, 240), move |_| {
                    view! { <HostScrollingRows runs={runs} /> }
                })
            })
            .expect("declare the window");
        runtime.show(MAIN).expect("show the window");
        runtime.click_label("Seed").expect("seed one hundred rows");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open the host");
        host.render().expect("the first frame");
        let mounted = runs.load(Ordering::SeqCst);
        assert!(mounted > 0, "the initial window mounts rows");

        // 第一遍场景只计算目标窗口：不运行任何行 factory，也不应用请求。
        let before = host.counters;
        let (targets, first_row, pending) = host
            .with_window(MAIN, |host, window| {
                window.scroll_offset = 400;
                let pending = host.ensure_scene(window)?;
                let targets = pending
                    .as_ref()
                    .map(|pending| pending.row_windows.bands.len())
                    .unwrap_or(0);
                let snapshot = host
                    .runtime
                    .presentation_of(window.ctx)
                    .map_err(HostError::Snapshot)?;
                Ok((targets, snapshot.row_bands()[0].first_row(), pending))
            })
            .expect("the pure first pass");
        assert_eq!(targets, 1, "the first pass reports one pending band");
        assert_eq!(
            host.counters.scene_builds - before.scene_builds,
            1,
            "the pure first pass builds exactly one scene"
        );
        assert_eq!(
            runs.load(Ordering::SeqCst),
            mounted,
            "scene building never runs a row factory"
        );
        assert_eq!(
            first_row, 0,
            "the first pass does not apply the pending window"
        );

        // 结构 turn 在构建调用栈之外挂载进入窗口的行，并推进 revision。
        let pending = pending.expect("the first pass produced a pending scene");
        host.with_window(MAIN, |host, window| {
            host.settle_input_scene(window, pending)
        })
        .expect("the input boundary applies the batch");
        assert_eq!(
            host.counters.scene_builds - before.scene_builds,
            2,
            "a changed batch rebuilds exactly one second pass"
        );
        assert!(
            runs.load(Ordering::SeqCst) > mounted,
            "the structure turn mounts the entering rows outside scene building"
        );
        let snapshot = host
            .runtime
            .presentation_of(host.windows[MAIN].ctx)
            .expect("the committed snapshot");
        assert!(
            snapshot.row_bands()[0].first_row() > 0,
            "the batch applied at the input boundary"
        );
        assert_eq!(
            host.scene(MAIN).expect("the input scene").scroll_offset(),
            400,
            "the input scene follows the requested offset"
        );
    }

    /// 长跑的一行采样；字段与外部 GPU 采样按时间戳对齐。
    #[derive(Clone, Copy, Debug)]
    struct SoakSample {
        elapsed: Duration,
        working_set: usize,
        private_bytes: usize,
        handles: u32,
        scene_builds: usize,
        presents: usize,
        snapshots: u64,
        remaining: u32,
    }

    /// 当前进程的工作集、私有提交与句柄数；只读取，不关闭进程伪句柄。
    fn soak_process_metrics() -> (usize, usize, u32) {
        use windows::Win32::System::ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
        };
        use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessHandleCount};

        let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        let mut memory = PROCESS_MEMORY_COUNTERS {
            cb: size,
            ..Default::default()
        };
        let process = unsafe { GetCurrentProcess() };
        // 安全：当前进程伪句柄无需关闭；计数结构由本栈帧独占。
        unsafe { GetProcessMemoryInfo(process, &mut memory, size) }
            .expect("read the process memory counters");
        let mut handles = 0;
        // 安全：同上。
        unsafe { GetProcessHandleCount(process, &mut handles) }
            .expect("read the process handle count");
        (memory.WorkingSetSize, memory.PagefileUsage, handles)
    }

    /// 从可访问性树读取 “N remaining”；它与画面来自同一份已提交快照。
    fn soak_remaining(host: &Host) -> u32 {
        host.accessibility_tree(MAIN)
            .expect("the visible window publishes its tree")
            .iter()
            .find_map(|node| node.name().strip_suffix(" remaining")?.parse().ok())
            .expect("the remaining counter is in the tree")
    }

    fn soak_render_and_count(host: &mut Host) -> u32 {
        host.render().expect("render the changed window");
        soak_remaining(host)
    }

    fn soak_sample(host: &Host, started: Instant) -> SoakSample {
        let (working_set, private_bytes, handles) = soak_process_metrics();
        SoakSample {
            elapsed: started.elapsed(),
            working_set,
            private_bytes,
            handles,
            scene_builds: host.counters.scene_builds,
            presents: host.counters.presents,
            snapshots: host.runtime.presentation_builds(),
            remaining: soak_remaining(host),
        }
    }

    fn soak_print(label: &str, sample: SoakSample) {
        println!(
            "soak {label} t={:.1}s ws={}KB private={}KB handles={} scene_builds={} presents={} snapshots={} remaining={}",
            sample.elapsed.as_secs_f64(),
            sample.working_set / 1024,
            sample.private_bytes / 1024,
            sample.handles,
            sample.scene_builds,
            sample.presents,
            sample.snapshots,
            sample.remaining
        );
    }

    /// 设置 `ZENUI_SOAK_TRACE=1` 时逐步打印内存，定位长跑中的增长来源。
    fn soak_trace(step: &str) {
        if std::env::var_os("ZENUI_SOAK_TRACE").is_none() {
            return;
        }
        let (working_set, private_bytes, handles) = soak_process_metrics();
        println!(
            "soak trace {step} ws={}KB private={}KB handles={handles}",
            working_set / 1024,
            private_bytes / 1024
        );
    }

    /// 等到任务结果真正投递并重绘；返回时的计数必须与预期一致。
    fn soak_wait_for_remaining(host: &mut Host, expected: u32) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let remaining = soak_render_and_count(host);
            if remaining == expected {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "a task delivery did not settle: expected {expected} remaining, saw {remaining}"
            );
            std::thread::sleep(Duration::from_millis(20));
            host.pump().expect("pump the delivery");
        }
    }

    /// 一个约 10 秒的脚本周期：键入、组合、添加、异步任务、主题、10k 滚动、
    /// 详情窗口的创建/销毁与主窗口隐藏/显示。
    ///
    /// 返回把本周期新增任务计入后的预期剩余数。
    fn soak_cycle(host: &mut Host, cycle: u64, rows: u32, mut expected: u32) -> u32 {
        // 真实 WM_CHAR 字符走窗口过程 → 聚焦编辑器的插入路径。
        host.runtime
            .focus("What needs to be done?")
            .expect("focus the draft");
        for character in format!("Soak note {cycle}").chars() {
            send_msg(
                host,
                MSG {
                    message: WM_CHAR,
                    wParam: WPARAM(character as usize),
                    ..Default::default()
                },
            );
        }
        // 每 4 个周期走一次 preedit → 提交；与真实 IME 共用运行时组合路径。
        if cycle % 4 == 1 {
            let node = host
                .runtime
                .presentation()
                .expect("snapshot")
                .find_label("What needs to be done?")
                .expect("draft")
                .id();
            with_main(host, |host, window| host.ime_start(window));
            with_main(host, |host, window| {
                host.apply_ime_composition(
                    window,
                    node,
                    request(false, true),
                    None,
                    Some(String::from("zhong")),
                )
            });
            with_main(host, |host, window| {
                host.apply_ime_composition(
                    window,
                    node,
                    request(true, false),
                    Some(String::from("中文")),
                    None,
                )
            });
        }
        if cycle % 5 == 4 {
            host.runtime.submit("Add task").expect("add the typed row");
            expected += 1;
        } else {
            host.runtime
                .input("What needs to be done?", "")
                .expect("clear the draft");
        }
        soak_trace("text");
        if cycle.is_multiple_of(2) {
            host.runtime.click_text("Theme").expect("toggle the theme");
            soak_trace("theme");
        }
        if cycle.is_multiple_of(3) {
            host.runtime
                .click_text("Load sample")
                .expect("start the sample task");
            host.runtime.wait_for_tasks();
            expected += 2;
            soak_wait_for_remaining(host, expected);
            soak_trace("task");
        }
        // 10k 行列表：UIA 跳到底部再回顶部，滚轮在两端移动虚拟化窗口。
        host.render().expect("render before scrolling");
        soak_trace("pre-scroll");
        let scroll = scroll_pattern(host);
        unsafe { scroll.SetScrollPercent(UIA_ScrollPatternNoScroll, 100.0) }
            .expect("queue the end jump");
        host.pump().expect("drain the end jump");
        soak_trace("end-jump");
        let scene = host.scene(MAIN).expect("the scrolled scene");
        assert_eq!(
            scene.scroll_offset(),
            scene.max_scroll_offset(),
            "the percent jump reaches the end of the list"
        );
        let minimum = i64::from(rows.saturating_sub(100)) * 37;
        assert!(
            i64::from(scene.scroll_offset()) >= minimum,
            "the list really holds {rows} rows: offset {} < {minimum}",
            scene.scroll_offset()
        );
        for _ in 0..3 {
            send_msg(
                host,
                MSG {
                    message: WM_MOUSEWHEEL,
                    wParam: WPARAM(((120i16 as u16) as usize) << 16),
                    ..Default::default()
                },
            );
        }
        soak_trace("end-wheel");
        unsafe { scroll.SetScrollPercent(UIA_ScrollPatternNoScroll, 0.0) }
            .expect("queue the top jump");
        host.pump().expect("drain the top jump");
        soak_trace("top-jump");
        for _ in 0..3 {
            send_msg(
                host,
                MSG {
                    message: WM_MOUSEWHEEL,
                    wParam: WPARAM(((-120i16 as u16) as usize) << 16),
                    ..Default::default()
                },
            );
        }
        unsafe { scroll.SetScrollPercent(UIA_ScrollPatternNoScroll, 0.0) }
            .expect("return to the top");
        host.pump().expect("drain the top return");
        soak_trace("top-wheel");

        if cycle % 7 == 6 {
            host.runtime
                .click_label("Open details")
                .expect("open the details window");
            // 直接运行时派发不会经过窗口过程；用真实唤醒消息结算窗口请求。
            send_msg(
                host,
                MSG {
                    message: host_message::WAKE,
                    ..Default::default()
                },
            );
            assert!(
                host.hwnd(zenui_demo::DETAILS_WINDOW).is_some(),
                "the details window opens"
            );
            soak_trace("details-open");
            host.render().expect("paint the details window");
            soak_trace("details-render");
            send_msg_in(
                host,
                zenui_demo::DETAILS_WINDOW,
                MSG {
                    message: WM_CLOSE,
                    ..Default::default()
                },
            );
            assert!(
                host.hwnd(zenui_demo::DETAILS_WINDOW).is_none(),
                "the details window closes without a prompt"
            );
            soak_trace("details-close");
        }

        if cycle % 6 == 5 {
            // 任务在可见窗口上启动，唤醒却在隐藏期间到达；隐藏窗口不再发布
            // 可访问性树，因此投递在 pump 中结算、显示后立即核对计数。
            host.runtime
                .click_text("Load sample")
                .expect("start a task before hiding");
            expected += 2;
            host.runtime.wait_for_tasks();
            host.hide(MAIN).expect("hide the window");
            host.pump().expect("pump the delivery while hidden");
            std::thread::sleep(Duration::from_millis(300));
            host.pump().expect("pump the late delivery while hidden");
            host.show(MAIN).expect("show the window");
            host.pump().expect("pump after showing");
            soak_wait_for_remaining(host, expected);
            soak_trace("hide-show");
        }

        let remaining = soak_render_and_count(host);
        assert_eq!(
            remaining, expected,
            "the model stayed consistent in cycle {cycle}"
        );
        assert!(
            host.accessibility_tree(MAIN)
                .expect("tree")
                .find_name("Error")
                .is_none(),
            "cycle {cycle} reported no task error"
        );
        expected
    }

    /// 30 分钟可见长跑；默认忽略，CI 不运行。
    ///
    /// 运行方式（结果输出到文件，外部同时采样 GPU）：
    ///
    /// ```text
    /// ZENUI_SOAK_MINUTES=30 cargo test -p zenui-windows --lib -- \
    ///     --ignored --nocapture --test-threads=1 a_bounded_visible_soak
    /// ```
    ///
    /// 每分钟输出一行 `soak sample ...`。断言覆盖：任务计数与投递一致、
    /// 散热后的静置窗口零重建、工作集/私有提交/句柄的有界增长。
    #[test]
    #[ignore = "a visible 30-minute soak is a local acceptance run"]
    fn a_bounded_visible_soak_stays_flat() {
        use windows::Win32::System::Power::{
            ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
        };

        /// 长跑期间保持系统与显示唤醒；退出时恢复连续状态。
        struct KeepAwake;
        impl Drop for KeepAwake {
            fn drop(&mut self) {
                // 安全：只恢复本线程的连续执行状态。
                unsafe {
                    SetThreadExecutionState(ES_CONTINUOUS);
                }
            }
        }

        let minutes: u64 = std::env::var("ZENUI_SOAK_MINUTES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(30);
        let rows: u32 = std::env::var("ZENUI_SOAK_ROWS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(10_000);
        assert!(minutes >= 1, "the soak needs a minute to sample");

        // 安全：只改变本线程的执行状态；Drop 恢复。
        unsafe {
            SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED);
        }
        let _awake = KeepAwake;

        let mut runtime = Runtime::new();
        zenui_demo::open_todo_window(&mut runtime).expect("open the todo window");
        zenui_demo::seed(&mut runtime).expect("seed the todo rows");
        let seeded = Instant::now();
        for index in 0..rows {
            runtime
                .input("What needs to be done?", &format!("Soak row {index}"))
                .expect("fill the draft");
            runtime.submit("Add task").expect("add a row");
            if (index + 1) % 2_000 == 0 {
                println!(
                    "soak seed {}/{} at {:.1}s",
                    index + 1,
                    rows,
                    seeded.elapsed().as_secs_f64()
                );
            }
        }

        let mut host = Host::open(runtime, HostConfig::visible()).expect("open the visible host");
        // 种子有 2 行、其中 1 行已完成；之后每行都未完成。
        let mut expected = 1 + rows;
        host.render().expect("paint the first frame");
        println!(
            "soak start pid={} rows={} minutes={}",
            std::process::id(),
            rows,
            minutes
        );
        assert_eq!(
            soak_render_and_count(&mut host),
            expected,
            "the seeded model is on screen"
        );

        // 先排掉系统在首次显示时排队的 WM_PAINT，再记录静置基线。
        host.pump().expect("drain the startup paint");
        std::thread::sleep(Duration::from_millis(300));
        host.pump().expect("drain the late startup paint");
        host.render().expect("paint the settled frame");
        host.pump().expect("settle the repaint");
        // 启动后静置：没有焦点编辑器时不允许任何后台重建或呈现。
        let before = host.counters;
        std::thread::sleep(Duration::from_millis(1200));
        host.pump().expect("pump the idle window");
        assert_eq!(
            host.counters, before,
            "a freshly opened visible window idles without rebuilding"
        );

        let started = Instant::now();
        let total = Duration::from_secs(minutes * 60);
        let mut next_sample = Duration::from_secs(60);
        let mut cycle = 0;
        let mut samples = Vec::new();
        soak_print("sample", soak_sample(&host, started));
        while started.elapsed() < total {
            let cycle_started = Instant::now();
            expected = soak_cycle(&mut host, cycle, rows, expected);
            cycle += 1;
            let elapsed = cycle_started.elapsed();
            if elapsed < Duration::from_secs(10) {
                // 留出真实用户在两批操作之间的等待时间，也是空闲上限的探针。
                std::thread::sleep(Duration::from_secs(10) - elapsed);
                host.pump().expect("pump between cycles");
            }
            if started.elapsed() >= next_sample {
                let sample = soak_sample(&host, started);
                soak_print("sample", sample);
                samples.push(sample);
                next_sample += Duration::from_secs(60);
            }
        }

        // 收尾：移出编辑器焦点，排掉失焦帧排队的 WM_PAINT，再静置确认没有
        // 闪烁之外的后台重绘。
        host.runtime.blur().expect("blur the draft");
        host.render().expect("the settled frame");
        host.pump().expect("drain the blur paint");
        // 一次闪烁周期会呈现清除 caret 的最后一帧；之后再记录静置基线。
        std::thread::sleep(Duration::from_millis(1200));
        host.pump().expect("drain the caret erasure");
        let before = host.counters;
        let snapshots = host.runtime.presentation_builds();
        std::thread::sleep(Duration::from_millis(1500));
        host.pump().expect("pump the quiet window");
        assert_eq!(
            host.counters, before,
            "the soaked window stopped rebuilding once idle"
        );
        assert_eq!(
            host.runtime.presentation_builds(),
            snapshots,
            "no stale delivery rebuilt the presentation once idle"
        );
        let quiet = soak_sample(&host, started);
        soak_print("quiet", quiet);
        samples.push(quiet);
        assert_eq!(quiet.remaining, expected, "the final model is consistent");

        let first = samples.first().copied().expect("a start sample");
        let last = samples[samples.len() - 1];
        let middle = samples[samples.len() / 2];
        let working_growth = last.working_set.saturating_sub(first.working_set);
        let private_growth = last.private_bytes.saturating_sub(first.private_bytes);
        let handle_growth = last.handles.saturating_sub(first.handles);
        let late_growth = last.working_set.saturating_sub(middle.working_set);
        println!(
            "soak summary cycles={cycle} working_growth={}KB private_growth={}KB late_working_growth={}KB handle_growth={}",
            working_growth / 1024,
            private_growth / 1024,
            late_growth / 1024,
            handle_growth
        );
        assert!(
            working_growth <= 64 * 1024 * 1024,
            "the working set grew by {working_growth} bytes over the run"
        );
        assert!(
            private_growth <= 64 * 1024 * 1024,
            "private commit grew by {private_growth} bytes over the run"
        );
        assert!(
            late_growth <= 32 * 1024 * 1024,
            "the second half grew by {late_growth} bytes"
        );
        assert!(
            handle_growth <= 32,
            "the process gained {handle_growth} handles over the run"
        );
    }

    #[test]
    fn a_window_without_a_context_menu_leaves_the_presenter_installed() {
        let mut runtime = Runtime::new();
        runtime
            .declare_windows(|catalog| {
                catalog.window(WindowSpec::new(MAIN).primary(), |_| view! { <view /> })?;
                Ok(())
            })
            .expect("declare the plain host");
        let mut host = Host::open(runtime, HostConfig::hidden()).expect("open the host");
        let records = Rc::new(RefCell::new(Vec::new()));
        host.set_context_menu_presenter(FakeContextMenuPresenter {
            records: records.clone(),
            result: None,
            panic_first: Rc::new(Cell::new(false)),
            during_show: None,
        });
        assert!(send_context_menu(&mut host, Some((3, 4))).is_none());
        assert!(records.borrow().is_empty());
        assert!(
            host.context_menu_presenter.is_some(),
            "an undeclared window does not consume the presenter"
        );
    }
}
