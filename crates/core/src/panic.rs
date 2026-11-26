//! 用户闭包 panic 的捕获与诊断。
//!
//! 运行时在每一个用户代码边界调用 [`catch_panic`]，把 unwind 收敛为
//! [`PanicInfo`]；这样一次 panic 只毒化当前节点或槽位，不穿过公开入口。

use std::any::Any;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// 一次被运行时捕获的用户闭包 panic。
///
/// 只保留稳定的诊断消息；panic 的原始 payload 不跨边界保存，避免把
/// 用户类型泄漏进运行时状态。实现 [`Eq`] 是为了让上层错误类型保持可比较。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PanicInfo {
    message: String,
}

impl PanicInfo {
    /// 用一条诊断消息构造；消息应描述用户闭包失败的原因。
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// 从标准库的 panic payload 提取可读消息。
    ///
    /// 字符串 payload 保留原文；其他类型（包括 `panic_any` 的自定义值）只
    /// 报告一个稳定的占位描述。
    pub fn from_payload(payload: &(dyn Any + Send)) -> Self {
        let message = if let Some(message) = payload.downcast_ref::<&'static str>() {
            (*message).to_owned()
        } else if let Some(message) = payload.downcast_ref::<String>() {
            message.clone()
        } else {
            String::from("non-string panic payload")
        };
        Self { message }
    }

    /// 被捕获 panic 的消息文本。
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for PanicInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "user code panicked: {}", self.message)
    }
}

impl std::error::Error for PanicInfo {}

/// 在 unwind 边界内运行用户闭包；panic 变成可比较的 [`PanicInfo`]。
///
/// 调用方仍负责恢复闭包外的运行状态（phase、owner、事件槽）；本函数只
/// 收敛 unwind，不修改 Runtime。containment 依赖 `panic = "unwind"` 构建；
/// `panic = "abort"` 下进程直接终止，不承诺恢复。
pub fn catch_panic<R>(f: impl FnOnce() -> R) -> Result<R, PanicInfo> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(value) => Ok(value),
        Err(payload) => Err(PanicInfo::from_payload(payload.as_ref())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_string_payload_keeps_its_message() {
        let panic = catch_panic(|| panic!("memo compute failed"));
        assert_eq!(
            panic.expect_err("the closure panicked").message(),
            "memo compute failed"
        );
    }

    #[test]
    fn a_non_string_payload_has_a_stable_description() {
        let panic = catch_panic(|| std::panic::panic_any(7_u32));
        assert_eq!(
            panic.expect_err("the closure panicked").message(),
            "non-string panic payload"
        );
    }

    #[test]
    fn a_successful_closure_returns_its_value() {
        assert_eq!(catch_panic(|| 7_u32), Ok(7));
    }
}
