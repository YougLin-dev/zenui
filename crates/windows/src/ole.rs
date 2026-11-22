//! 宿主拥有的 OLE 套间。
//!
//! Shell 文件对话框与后续剪贴板/拖放都需要 UI 线程先初始化 STA。这里只在
//! 第一次需要时初始化，并让 `Host` 持有唯一的所有权：每个成功的
//! `OleInitialize`（`S_OK` 或 `S_FALSE`）恰好配对一次 `OleUninitialize`。
//! 注入 presenter 的隐藏宿主不初始化 OLE。

use std::fmt;

use windows::Win32::Foundation::{E_FAIL, RPC_E_CHANGED_MODE};
use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};
use windows::core::HRESULT;

/// OLE 初始化失败；调用方把它映射到自己的公开错误。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OleError {
    /// UI 线程已经以不同的套间模型初始化过 COM。
    ChangedMode,
    /// 其它 HRESULT 失败；保留平台描述。
    Failed(String),
}

impl fmt::Display for OleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ChangedMode => f.write_str("the UI thread is already in another COM apartment"),
            Self::Failed(message) => write!(f, "OLE initialization failed: {message}"),
        }
    }
}

impl std::error::Error for OleError {}

/// UI 线程的 OLE 初始化守卫；成功构造后恰好配对一次 `OleUninitialize`。
pub(crate) struct OleApartment;

impl OleApartment {
    /// 在 UI 线程上初始化 STA；`S_OK` 与 `S_FALSE` 都视为成功。
    pub(crate) fn initialize() -> Result<Self, OleError> {
        // 安全：pvReserved 必须为空；宿主只在 UI 线程上创建与释放。
        // windows-rs 把非负 HRESULT（含 S_FALSE）映射为 Ok；两种情况都要求
        // 一次 OleUninitialize，由 Drop 配对。
        match unsafe { OleInitialize(None) } {
            Ok(()) => Ok(Self),
            Err(error) => Err(ole_error(error.code())),
        }
    }
}

impl Drop for OleApartment {
    fn drop(&mut self) {
        // 安全：只与成功返回的 OleInitialize 配对；Host 在 UI 线程上释放。
        unsafe {
            OleUninitialize();
        }
    }
}

/// 把失败的 HRESULT 映射成内部错误；成功码由 `Result` 提前分流。
fn ole_error(result: HRESULT) -> OleError {
    if result == RPC_E_CHANGED_MODE {
        OleError::ChangedMode
    } else if result == E_FAIL {
        OleError::Failed(String::from("E_FAIL"))
    } else {
        OleError::Failed(format!("OleInitialize failed with {result:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_foreign_apartment_and_generic_failure_are_distinct() {
        assert_eq!(ole_error(RPC_E_CHANGED_MODE), OleError::ChangedMode);
        assert_eq!(ole_error(E_FAIL), OleError::Failed(String::from("E_FAIL")));
        assert!(matches!(
            ole_error(HRESULT(0x8007_000E_u32 as i32)),
            OleError::Failed(message) if message.contains("OleInitialize failed")
        ));
    }
}
