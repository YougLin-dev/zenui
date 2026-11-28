//! 基线文件与比较规则（§20.3）。
//!
//! 计数按硬上限检查，任何机器都必须通过；时间只在与基线同一 runner 且
//! 指纹完全一致时检查，允许 10% 噪声，超过 15% 失败。`--require-time`
//! 让固定 runner 在尚未重录基线时直接失败，而不是静默跳过。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::fingerprint::Fingerprint;
use crate::harness::{PerfResult, fail};
use crate::workloads::Measurement;

/// 当前基线格式版本；字段语义变化时一起提升。
pub const SCHEMA: u32 = 1;

/// 时间样本统计；纳秒。median 是门禁使用的值，p50/p95 记录趋势。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timing {
    pub samples: usize,
    pub median_ns: u64,
    pub p50_ns: u64,
    pub p95_ns: u64,
    /// 最快样本；仅记录趋势。
    pub min_ns: u64,
    /// 最快十分位（至少 3 个样本）的平均值；时间门禁使用它。
    ///
    /// 单个 min 在两个进程间像抽奖；对最有利的一小段取均值既压住调度
    /// 抖动，又能对真实回归保持敏感。
    #[serde(default)]
    pub fast_ns: u64,
    /// 每个样本包含的连续 turn 数；样本时间是这些 turn 的总和。
    #[serde(default)]
    pub repeats: usize,
}

impl Timing {
    /// 从原始样本计算排序统计；空样本保留全 0。
    pub fn from_samples(times: &[u64], repeats: usize) -> Self {
        if times.is_empty() {
            return Self {
                repeats,
                ..Self::default()
            };
        }
        let mut sorted = times.to_vec();
        sorted.sort_unstable();
        let len = sorted.len();
        let median_ns = if len.is_multiple_of(2) {
            sorted[len / 2 - 1].saturating_add(sorted[len / 2]) / 2
        } else {
            sorted[len / 2]
        };
        let p50_ns = sorted[len / 2];
        // 最近秩：第 ceil(0.95 * n) 个样本。
        let rank = (len * 95).div_ceil(100).max(1);
        let p95_ns = sorted[rank - 1];
        let fast_count = (len / 10).max(3).min(len);
        let fast_ns = sorted[..fast_count]
            .iter()
            .copied()
            .fold(0_u64, u64::saturating_add)
            / fast_count as u64;
        Self {
            samples: len,
            median_ns,
            p50_ns,
            p95_ns,
            min_ns: sorted[0],
            fast_ns,
            repeats,
        }
    }
}

/// 释放前必须通过的噪声预算。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Noise {
    /// 超过基线这个比例时输出警告，仍然通过。
    pub warn: f64,
    /// 超过基线这个比例时失败。
    pub fail: f64,
}

impl Default for Noise {
    fn default() -> Self {
        Self {
            warn: 0.10,
            fail: 0.15,
        }
    }
}

/// 一个 bench 的已提交基线。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Baseline {
    pub schema: u32,
    pub bench: String,
    /// 录制基线的固定 runner 标签；与当前标签不同则时间不可比。
    pub runner: String,
    pub fingerprint: Fingerprint,
    pub git: String,
    pub recorded_unix_seconds: u64,
    pub noise: Noise,
    /// 硬上限；`check` 只读取这里。
    pub limits: BTreeMap<String, u64>,
    /// 录制时的实际值；用于趋势与审阅，不作为阈值。
    pub measured: BTreeMap<String, u64>,
    pub timing_ns: Timing,
    /// 录制时的时钟校准最快值；时间门禁先用它归一化。
    #[serde(default)]
    pub clock_ns: u64,
}

impl Baseline {
    /// 用当前测量构造待写入的基线。
    pub fn capture(
        bench: &str,
        runner: &str,
        fingerprint: Fingerprint,
        measurement: &Measurement,
    ) -> Self {
        Self {
            schema: SCHEMA,
            bench: bench.to_owned(),
            runner: runner.to_owned(),
            fingerprint,
            git: git_revision(),
            recorded_unix_seconds: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |value| value.as_secs()),
            noise: Noise::default(),
            limits: measurement.limits.clone(),
            measured: measurement.counters.clone(),
            timing_ns: measurement.timing(),
            clock_ns: measurement.clock_ns,
        }
    }

    /// 基线文件名：`<bench>.json`。
    pub fn path(directory: &Path, bench: &str) -> PathBuf {
        directory.join(format!("{bench}.json"))
    }

    /// 读取并校验 schema 与 bench 名。
    pub fn load(path: &Path, bench: &str) -> PerfResult<Self> {
        let text = fs::read_to_string(path).map_err(|error| {
            fail(format!(
                "cannot read baseline {}: {error}; record it with `record --bench {bench}`",
                path.display()
            ))
        })?;
        let baseline: Self = serde_json::from_str(&text)
            .map_err(|error| fail(format!("invalid baseline {}: {error}", path.display())))?;
        if baseline.schema != SCHEMA {
            return Err(fail(format!(
                "baseline {} has schema {}, expected {SCHEMA}",
                path.display(),
                baseline.schema
            )));
        }
        if baseline.bench != bench {
            return Err(fail(format!(
                "baseline {} names bench {:?}, expected {bench:?}",
                path.display(),
                baseline.bench
            )));
        }
        Ok(baseline)
    }

    /// 写入格式化 JSON；目录不存在时创建。
    pub fn store(&self, path: &Path) -> PerfResult<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| fail(format!("cannot create {}: {error}", parent.display())))?;
        }
        let mut text = serde_json::to_string_pretty(self)
            .map_err(|error| fail(format!("cannot serialize baseline: {error}")))?;
        text.push('\n');
        fs::write(path, text)
            .map_err(|error| fail(format!("cannot write {}: {error}", path.display())))
    }
}

/// 一次检查的结论。
#[derive(Clone, Debug, Default)]
pub struct CheckReport {
    pub warnings: Vec<String>,
    pub failures: Vec<String>,
    /// 时间门禁是否真正执行（runner 与指纹匹配）。
    pub time_enforced: bool,
    /// 失败里有多少来自计数硬上限；计数失败不重试。
    pub counter_failures: usize,
    /// 失败里有多少来自时间噪声预算。
    pub time_failures: usize,
}

impl CheckReport {
    pub fn is_ok(&self) -> bool {
        self.failures.is_empty()
    }
}

/// 比较当前测量与基线；`require_time` 在时间门禁不可执行时直接失败。
pub fn check(
    baseline: &Baseline,
    measurement: &Measurement,
    runner: &str,
    fingerprint: &Fingerprint,
    require_time: bool,
) -> CheckReport {
    let mut report = CheckReport::default();

    for (name, limit) in &baseline.limits {
        match measurement.counters.get(name) {
            Some(value) if value > limit => {
                report.counter_failures += 1;
                report.failures.push(format!(
                    "counter {name} = {value} exceeds the hard limit {limit}"
                ));
            }
            Some(_) => {}
            None => {
                report.counter_failures += 1;
                report.failures.push(format!(
                    "counter {name} is required by the baseline but was not measured"
                ));
            }
        }
    }

    let runner_matches = baseline.runner == runner;
    let mismatches = baseline.fingerprint.mismatches(fingerprint);
    let comparable = runner_matches && mismatches.is_empty();
    // `dev*` 是本机临时基线：时间受开发负载影响，默认只报告趋势；
    // `--require-time` 或固定 runner 标签（例如 perf-windows）才阻塞。
    let stable_runner = !runner.starts_with("dev");
    let has_timing = measurement.timing().samples > 0;
    if comparable && !has_timing {
        // soak 等状态型负载没有时间样本；`--require-time` 不因此失败。
        report
            .warnings
            .push(String::from("this bench records state counters only"));
    } else if comparable && (stable_runner || require_time) {
        report.time_enforced = true;
        let current = measurement.timing();
        // 用最快十分位均值比较：单个中位数包含调度/频率抖动的上半尾。
        if baseline.timing_ns.fast_ns > 0 {
            let raw = current.fast_ns as f64 / baseline.timing_ns.fast_ns as f64;
            // 用同进程时钟校准确认机器频率状态；缺失校准（旧基线）时按原始比例。
            let clock = if baseline.clock_ns > 0 && measurement.clock_ns > 0 {
                measurement.clock_ns as f64 / baseline.clock_ns as f64
            } else {
                1.0
            };
            let ratio = raw / clock;
            let change = (ratio - 1.0) * 100.0;
            if ratio > 1.0 + baseline.noise.fail {
                report.time_failures += 1;
                report.failures.push(format!(
                    "fast time {:.1}% above baseline ({} ns vs {} ns)",
                    change, current.fast_ns, baseline.timing_ns.fast_ns
                ));
            } else if ratio > 1.0 + baseline.noise.warn {
                report.warnings.push(format!(
                    "fast time {:.1}% above baseline, beyond the {:.0}% noise budget but below the {:.0}% failure line",
                    change,
                    baseline.noise.warn * 100.0,
                    baseline.noise.fail * 100.0
                ));
            }
        }
    } else {
        let reason = if !comparable {
            if runner_matches {
                format!("fingerprint differs ({})", mismatches.join(", "))
            } else {
                format!(
                    "baseline runner {:?} differs from {:?}",
                    baseline.runner, runner
                )
            }
        } else {
            format!("runner {runner:?} is a development runner")
        };
        let message = format!("time gate skipped: {reason}; counters were still enforced");
        if require_time {
            report.time_failures += 1;
            report.failures.push(format!(
                "{message}. Re-record the baseline on this runner with `record --bench {}`.",
                baseline.bench
            ));
        } else {
            report.warnings.push(message);
        }
    }

    report
}

fn git_revision() -> String {
    let revision = std::process::Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| String::from("unknown"));
    // 基线在提交前录制是常态；`-dirty` 让审阅者知道 sha 不是被测代码本身。
    let dirty = std::process::Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()
        .ok()
        .is_some_and(|output| {
            output.status.success() && !String::from_utf8_lossy(&output.stdout).trim().is_empty()
        });
    if dirty {
        format!("{revision}-dirty")
    } else {
        revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::Counters;

    fn fingerprint() -> Fingerprint {
        Fingerprint {
            os: String::from("windows"),
            arch: String::from("x86_64"),
            cpu: String::from("test cpu"),
            logical_cores: 8,
            rustc: String::from("rustc test"),
            profile: String::from("release"),
            fonts: String::from("system-default"),
        }
    }

    fn measurement(times: &[u64], counters: &[(&str, u64)]) -> Measurement {
        Measurement {
            counters: counters
                .iter()
                .map(|(name, value)| ((*name).to_owned(), *value))
                .collect::<Counters>(),
            limits: BTreeMap::new(),
            times_ns: times.to_vec(),
            notes: Vec::new(),
            repeats: 1,
            clock_ns: 1_000,
        }
    }

    fn baseline(times: &[u64], limits: &[(&str, u64)], runner: &str) -> Baseline {
        let mut baseline =
            Baseline::capture("bench", runner, fingerprint(), &measurement(times, &[]));
        baseline.limits = limits
            .iter()
            .map(|(name, value)| ((*name).to_owned(), *value))
            .collect();
        baseline
    }

    #[test]
    fn timing_reports_sorted_statistics() {
        let timing = Timing::from_samples(&[50, 10, 30, 20, 40], 3);
        assert_eq!(timing.samples, 5);
        assert_eq!(timing.median_ns, 30);
        assert_eq!(timing.p50_ns, 30);
        assert_eq!(timing.min_ns, 10);
        assert_eq!(timing.fast_ns, 20, "the fastest decile averages 10 and 20");
        assert_eq!(timing.repeats, 3);
    }

    #[test]
    fn a_counter_over_its_limit_fails_without_retry() {
        let baseline = baseline(&[1_000; 10], &[("bindings_evaluated", 0)], "perf-windows");
        let report = check(
            &baseline,
            &measurement(&[1_000; 10], &[("bindings_evaluated", 1)]),
            "perf-windows",
            &fingerprint(),
            true,
        );
        assert!(!report.is_ok());
        assert_eq!(report.counter_failures, 1);
        assert_eq!(report.time_failures, 0);
    }

    #[test]
    fn time_beyond_the_failure_budget_fails_on_a_stable_runner() {
        let baseline = baseline(&[1_000; 10], &[], "perf-windows");
        let report = check(
            &baseline,
            &measurement(&[1_200; 10], &[]),
            "perf-windows",
            &fingerprint(),
            true,
        );
        assert!(!report.is_ok());
        assert_eq!(report.time_failures, 1);
        assert!(report.failures[0].contains("20.0%"));
    }

    #[test]
    fn time_inside_the_noise_budget_only_warns() {
        let baseline = baseline(&[1_000; 10], &[], "perf-windows");
        let report = check(
            &baseline,
            &measurement(&[1_120; 10], &[]),
            "perf-windows",
            &fingerprint(),
            true,
        );
        assert!(report.is_ok());
        assert_eq!(report.time_failures, 0);
        assert_eq!(report.warnings.len(), 1);
    }

    #[test]
    fn a_state_only_bench_does_not_require_time() {
        let baseline = baseline(&[], &[("storage_node_slots", 32)], "perf-windows");
        let report = check(
            &baseline,
            &measurement(&[], &[("storage_node_slots", 32)]),
            "perf-windows",
            &fingerprint(),
            true,
        );
        assert!(report.is_ok());
        assert!(!report.time_enforced);
        assert_eq!(report.warnings.len(), 1);
    }

    #[test]
    fn a_development_runner_reports_time_without_blocking() {
        let baseline = baseline(&[1_000; 10], &[], "dev");
        let report = check(
            &baseline,
            &measurement(&[2_000; 10], &[]),
            "dev",
            &fingerprint(),
            false,
        );
        assert!(report.is_ok());
        assert!(!report.time_enforced);
        assert_eq!(report.warnings.len(), 1);
    }

    #[test]
    fn require_time_fails_when_the_baseline_was_recorded_elsewhere() {
        let baseline = baseline(&[1_000; 10], &[], "perf-windows");
        let report = check(
            &baseline,
            &measurement(&[1_000; 10], &[]),
            "dev",
            &fingerprint(),
            true,
        );
        assert!(!report.is_ok());
        assert_eq!(report.time_failures, 1);
        assert_eq!(report.counter_failures, 0);
    }
}
