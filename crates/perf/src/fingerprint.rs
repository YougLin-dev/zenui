//! 运行环境指纹：时间门禁只在与基线同一台机器、同一工具链时启用。
//!
//! 计数门禁是确定性的，任何机器都必须通过；时间受 CPU、字体与系统负载
//! 影响，所以基线文件记录指纹，不匹配时只报告趋势（§20.3）。

use std::process::Command;

use serde::{Deserialize, Serialize};

/// 记录在基线文件里的运行环境。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub os: String,
    pub arch: String,
    /// Windows 的 `PROCESSOR_IDENTIFIER`；其它平台记录架构名。
    pub cpu: String,
    pub logical_cores: usize,
    /// `rustc --version` 的完整输出；工具链变化会改变代码生成。
    pub rustc: String,
    /// `release` 或 `debug`；时间门禁只接受 release。
    pub profile: String,
    /// 场景构建使用的字体来源；当前门禁使用系统字体默认集合。
    pub fonts: String,
}

impl Fingerprint {
    /// 读取当前进程的环境指纹。
    pub fn collect() -> Self {
        let cpu = std::env::var("PROCESSOR_IDENTIFIER")
            .unwrap_or_else(|_| std::env::consts::ARCH.to_owned());
        let rustc = Command::new("rustc")
            .arg("--version")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .unwrap_or_else(|| String::from("unknown"));
        Self {
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            cpu,
            logical_cores: std::thread::available_parallelism().map_or(0, |value| value.get()),
            rustc,
            profile: if cfg!(debug_assertions) {
                String::from("debug")
            } else {
                String::from("release")
            },
            fonts: String::from("system-default"),
        }
    }

    /// 与另一指纹的差异字段；空表示时间可比。
    pub fn mismatches(&self, other: &Self) -> Vec<&'static str> {
        let mut fields = Vec::new();
        if self.os != other.os {
            fields.push("os");
        }
        if self.arch != other.arch {
            fields.push("arch");
        }
        if self.cpu != other.cpu {
            fields.push("cpu");
        }
        if self.logical_cores != other.logical_cores {
            fields.push("logical_cores");
        }
        if self.rustc != other.rustc {
            fields.push("rustc");
        }
        if self.profile != other.profile {
            fields.push("profile");
        }
        if self.fonts != other.fonts {
            fields.push("fonts");
        }
        fields
    }

    /// 一行人类可读描述，写入基线文件后的日志。
    pub fn describe(&self) -> String {
        format!(
            "{} {} / {} / {} cores / {}",
            self.os, self.arch, self.cpu, self.logical_cores, self.rustc
        )
    }
}
