//! Windows 剪贴板提供者。
//!
//! 宿主通过 [`ClipboardProvider`] 这个 seam 读写剪贴板；可见宿主安装
//! `CF_UNICODETEXT` 原生实现，隐藏宿主默认没有 provider，CI 不触碰系统
//! 剪贴板。剪贴板失败对用户是静默无操作，不产生 `HostError`，也不改写文本。

use std::fmt;
use std::time::Duration;

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::Win32::System::Ole::CF_UNICODETEXT;

const OPEN_ATTEMPTS: usize = 5;
const OPEN_RETRY: Duration = Duration::from_millis(10);

/// 剪贴板读写失败。
///
/// 宿主把读写失败当作静默无操作；类型保留失败原因供嵌入方诊断，不进入
/// 用户可见错误路径。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardError {
    message: String,
}

impl ClipboardError {
    /// 用诊断消息构造错误。
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// 失败的诊断消息。
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ClipboardError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ClipboardError {}

/// 宿主剪贴板读写 seam。
///
/// 可见宿主默认安装 `CF_UNICODETEXT` 原生实现；隐藏宿主没有 provider，
/// 因此 `Ctrl+C`/`Ctrl+V` 不产生任何系统剪贴板 I/O。测试与嵌入方可以注入
/// 自己的实现。方法在 UI 线程调用，panic 会被宿主 contain 为无操作。
pub trait ClipboardProvider {
    /// 读取 Unicode 文本；剪贴板没有文本时返回 `Ok(None)`。
    fn read_text(&mut self) -> Result<Option<String>, ClipboardError>;

    /// 用 `text` 替换剪贴板 Unicode 文本；失败返回错误。
    fn write_text(&mut self, text: &str) -> Result<(), ClipboardError>;
}

/// 单行输入接受的剪贴板文本：去掉 NUL、换行与其它控制字符，保留 Unicode。
pub(crate) fn single_line_text(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_control())
        .collect()
}

fn decode_utf16(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

/// 原生 `CF_UNICODETEXT` provider。
pub(crate) struct NativeClipboard;

impl ClipboardProvider for NativeClipboard {
    fn read_text(&mut self) -> Result<Option<String>, ClipboardError> {
        read_windows_clipboard()
    }

    fn write_text(&mut self, text: &str) -> Result<(), ClipboardError> {
        write_windows_clipboard(text)
    }
}

/// 打开剪贴板；成功后由 guard 保证每条路径都关闭。
struct ClipboardGuard;

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        // 安全：只关闭本线程已获得的剪贴板所有权；失败没有可恢复状态。
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

fn open_clipboard() -> Result<ClipboardGuard, ClipboardError> {
    for attempt in 0..OPEN_ATTEMPTS {
        // 安全：OpenClipboard 只操作当前线程；失败表示别的进程短暂持有剪贴板。
        if unsafe { OpenClipboard(None) }.is_ok() {
            return Ok(ClipboardGuard);
        }
        if attempt + 1 < OPEN_ATTEMPTS {
            std::thread::sleep(OPEN_RETRY);
        }
    }
    Err(ClipboardError::new(
        "OpenClipboard failed after bounded retries",
    ))
}

fn read_windows_clipboard() -> Result<Option<String>, ClipboardError> {
    let _guard = open_clipboard()?;
    // 安全：格式名是系统常量；返回的 HANDLE 由剪贴板拥有，不能释放。
    let handle = match unsafe { GetClipboardData(u32::from(CF_UNICODETEXT.0)) } {
        Ok(handle) if !handle.is_invalid() => handle,
        _ => return Ok(None),
    };
    let memory = HGLOBAL(handle.0);
    // 安全：锁定的缓冲在 GlobalUnlock 前有效；只读取 GlobalSize 范围内的 u16。
    let pointer = unsafe { GlobalLock(memory) } as *const u16;
    if pointer.is_null() {
        return Err(ClipboardError::new("GlobalLock failed for clipboard text"));
    }
    let units = unsafe { GlobalSize(memory) } / 2;
    let mut utf16 = Vec::with_capacity(units);
    for index in 0..units {
        // 安全：index < units，且剪贴板缓冲至少容纳 GlobalSize 字节。
        let unit = unsafe { *pointer.add(index) };
        if unit == 0 {
            break;
        }
        utf16.push(unit);
    }
    // 安全：与上面的 GlobalLock 配对；文本已复制到本地 Vec。
    unsafe {
        let _ = GlobalUnlock(memory);
    }
    Ok(Some(decode_utf16(&utf16)))
}

fn write_windows_clipboard(text: &str) -> Result<(), ClipboardError> {
    let mut utf16: Vec<u16> = text.encode_utf16().collect();
    utf16.push(0);
    let bytes = utf16.len() * 2;
    let _guard = open_clipboard()?;
    // 安全：清空与写入都发生在本线程持有剪贴板期间。
    unsafe { EmptyClipboard() }.map_err(|error| ClipboardError::new(error.to_string()))?;
    // 安全：GMEM_MOVEABLE 由系统分配；成功交给 SetClipboardData 后不再释放。
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) }
        .map_err(|error| ClipboardError::new(error.to_string()))?;
    // 安全：锁定的缓冲在 GlobalUnlock 前有效；复制 utf16.len() 个 u16。
    let pointer = unsafe { GlobalLock(memory) } as *mut u16;
    if pointer.is_null() {
        // 安全：分配仍由本函数拥有。
        let _ = unsafe { GlobalFree(Some(memory)) };
        return Err(ClipboardError::new("GlobalLock failed for clipboard text"));
    }
    // 安全：目标至少 GlobalSize 字节，源是本地 Vec。
    unsafe {
        std::ptr::copy_nonoverlapping(utf16.as_ptr(), pointer, utf16.len());
        let _ = GlobalUnlock(memory);
    }
    // 安全：成功后剪贴板拥有内存；失败路径仍由本函数释放。
    match unsafe { SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(memory.0))) } {
        Ok(_) => Ok(()),
        Err(error) => {
            // 安全：SetClipboardData 失败时系统未接管内存。
            let _ = unsafe { GlobalFree(Some(memory)) };
            Err(ClipboardError::new(error.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line_text_drops_controls_and_keeps_unicode() {
        assert_eq!(single_line_text("a\r\nb\0c\td"), "abcd");
        assert_eq!(single_line_text("你好🙂 x"), "你好🙂 x");
        assert_eq!(single_line_text("\r\n\t\0"), "");
    }

    #[test]
    fn invalid_utf16_decodes_with_replacement_characters() {
        let decoded = decode_utf16(&[0xD800, 0x0061]);
        assert_eq!(decoded, "\u{FFFD}a");
    }
}
