//! 无控制台 GUI 进程的启动错误呈现。

use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
use windows::core::HSTRING;

/// 用模态消息框显示启动错误；调用方决定随后是否退出。
///
/// release 模板使用 Windows GUI 子系统，`stderr` 不可见；启动失败必须有一个
/// 用户能看到的出口，而不是静默退出。
pub fn show_startup_error(caption: &str, message: &str) {
    let caption = HSTRING::from(caption);
    let message = HSTRING::from(message);
    // 安全：消息框只读取两个在调用期间存活的宽字符串；窗口所有者为空。
    unsafe {
        MessageBoxW(None, &message, &caption, MB_OK | MB_ICONERROR);
    }
}
