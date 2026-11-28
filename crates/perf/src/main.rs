//! ZenUI 性能/内存门禁入口。
//!
//! 工作负载、计数语义与噪声预算见 `docs/实现/性能门禁.md`。固定 runner
//! 通过 `--runner`（或 `ZENUI_PERF_RUNNER`）标识；基线文件在
//! `bench/baselines/<bench>.json`。

#![deny(unsafe_op_in_unsafe_fn)]

mod alloc;
mod baseline;
mod fingerprint;
mod harness;
mod workloads;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use fingerprint::Fingerprint;
use harness::{PerfResult, fail};
use workloads::{Measurement, WORKLOADS, Workload};

/// 每条负载的正式样本数；中位数比均值更抗系统抖动。
pub const SAMPLES: usize = 51;
/// 丢弃首帧缓存、焦点与文本整形冷启动。
pub const WARMUP: usize = 5;

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("zenui-perf: {error}");
            ExitCode::from(2)
        }
    }
}

fn run() -> PerfResult<ExitCode> {
    let options = Options::parse(std::env::args().skip(1))?;
    if options.command == Command::List {
        for workload in WORKLOADS {
            println!("{}", workload.name);
        }
        return Ok(ExitCode::SUCCESS);
    }
    if cfg!(debug_assertions) {
        return Err(fail(
            "performance gates require a release build: cargo run -p zenui-perf --release -- …",
        ));
    }
    let selected = select(&options)?;
    match options.command {
        Command::Run => {
            for workload in selected {
                let measurement = (workload.run)()?;
                print_measurement(workload.name, &measurement);
            }
        }
        Command::Record => {
            let fingerprint = Fingerprint::collect();
            let runner = options.runner_label();
            for workload in selected {
                let measurement = (workload.run)()?;
                let baseline = baseline::Baseline::capture(
                    workload.name,
                    &runner,
                    fingerprint.clone(),
                    &measurement,
                );
                let path = baseline::Baseline::path(&options.baseline_dir, workload.name);
                baseline.store(&path)?;
                println!(
                    "recorded {} on runner {runner} ({})",
                    path.display(),
                    fingerprint.describe()
                );
                print_measurement(workload.name, &measurement);
            }
        }
        Command::Check => {
            let fingerprint = Fingerprint::collect();
            let runner = options.runner_label();
            let mut failed = false;
            for workload in selected {
                let path = baseline::Baseline::path(&options.baseline_dir, workload.name);
                let baseline = match baseline::Baseline::load(&path, workload.name) {
                    Ok(baseline) => baseline,
                    Err(error) => {
                        print_check(
                            workload.name,
                            &Measurement::empty(),
                            &baseline::CheckReport {
                                failures: vec![error.to_string()],
                                ..baseline::CheckReport::default()
                            },
                        );
                        failed = true;
                        continue;
                    }
                };
                let mut measurement = (workload.run)()?;
                let mut report = baseline::check(
                    &baseline,
                    &measurement,
                    &runner,
                    &fingerprint,
                    options.require_time,
                );
                // 时间门禁对一次性系统负载重测一次；计数器硬上限从不重试。
                if report.counter_failures == 0 && report.time_failures > 0 {
                    measurement = (workload.run)()?;
                    report = baseline::check(
                        &baseline,
                        &measurement,
                        &runner,
                        &fingerprint,
                        options.require_time,
                    );
                    if report.is_ok() {
                        report.warnings.push(String::from(
                            "time failure cleared on the retry; the first run was affected by external load",
                        ));
                    }
                }
                print_check(workload.name, &measurement, &report);
                failed |= !report.is_ok();
            }
            if failed {
                return Ok(ExitCode::FAILURE);
            }
        }
        Command::List => unreachable!("handled above"),
    }
    Ok(ExitCode::SUCCESS)
}

fn select<'a>(options: &Options) -> PerfResult<Vec<&'a Workload>> {
    match &options.bench {
        Some(name) => WORKLOADS
            .iter()
            .find(|workload| workload.name == name)
            .map(|workload| vec![workload])
            .ok_or_else(|| {
                fail(format!(
                    "unknown bench {name:?}; run `zenui-perf list` for the registry"
                ))
            }),
        None => Ok(WORKLOADS.iter().collect()),
    }
}

fn print_measurement(name: &str, measurement: &Measurement) {
    let timing = measurement.timing();
    println!("{name}:");
    for (counter, value) in &measurement.counters {
        let limit = measurement
            .limits
            .get(counter)
            .map(|limit| format!(" limit={limit}"))
            .unwrap_or_default();
        println!("  {counter} = {value}{limit}");
    }
    if timing.samples > 0 {
        println!(
            "  time fast={} ns min={} ns median={} ns p50={} ns p95={} ns samples={} repeats={} clock={} ns",
            timing.fast_ns,
            timing.min_ns,
            timing.median_ns,
            timing.p50_ns,
            timing.p95_ns,
            timing.samples,
            timing.repeats,
            measurement.clock_ns
        );
    }
    for note in &measurement.notes {
        println!("  note: {note}");
    }
}

fn print_check(name: &str, measurement: &Measurement, report: &baseline::CheckReport) {
    let timing = measurement.timing();
    let verdict = if report.is_ok() { "ok" } else { "FAILED" };
    println!(
        "{name}: {verdict} (time gate {}, {} samples)",
        if report.time_enforced {
            "enforced"
        } else {
            "report-only"
        },
        timing.samples
    );
    for failure in &report.failures {
        println!("  failure: {failure}");
    }
    for warning in &report.warnings {
        println!("  warning: {warning}");
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    List,
    Run,
    Record,
    Check,
}

struct Options {
    command: Command,
    bench: Option<String>,
    baseline_dir: PathBuf,
    runner: Option<String>,
    require_time: bool,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> PerfResult<Self> {
        let mut args = args.peekable();
        let command = match args.peek().map(String::as_str) {
            Some("list") => Command::List,
            Some("run") => Command::Run,
            Some("record") => Command::Record,
            Some("check") => Command::Check,
            Some(other) => return Err(usage(format!("unknown command {other:?}"))),
            None => return Err(usage("missing command")),
        };
        let _ = args.next();
        let mut options = Self {
            command,
            bench: None,
            baseline_dir: workspace_root().join("bench").join("baselines"),
            runner: None,
            require_time: false,
        };
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--bench" => {
                    options.bench = Some(
                        args.next()
                            .ok_or_else(|| usage("--bench needs a workload name"))?,
                    );
                }
                "--baseline-dir" => {
                    options.baseline_dir = PathBuf::from(
                        args.next()
                            .ok_or_else(|| usage("--baseline-dir needs a path"))?,
                    );
                }
                "--runner" => {
                    options.runner =
                        Some(args.next().ok_or_else(|| usage("--runner needs a label"))?);
                }
                "--require-time" => options.require_time = true,
                other => return Err(usage(format!("unknown option {other:?}"))),
            }
        }
        if options.require_time && options.command != Command::Check {
            return Err(usage("--require-time is only valid for check"));
        }
        if matches!(options.command, Command::Record | Command::Check) && options.runner.is_none() {
            options.runner = std::env::var("ZENUI_PERF_RUNNER").ok();
            if options.runner.is_none() {
                return Err(usage(
                    "record/check need --runner or ZENUI_PERF_RUNNER (for example perf-windows)",
                ));
            }
        }
        Ok(options)
    }

    fn runner_label(&self) -> String {
        self.runner.clone().unwrap_or_else(|| String::from("dev"))
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .to_path_buf()
}

fn usage(message: impl AsRef<str>) -> harness::PerfError {
    fail(format!(
        "{}\nusage:
  zenui-perf list
  zenui-perf run [--bench NAME]
  zenui-perf record --runner LABEL [--bench NAME] [--baseline-dir DIR]
  zenui-perf check [--runner LABEL] [--require-time] [--bench NAME] [--baseline-dir DIR]",
        message.as_ref()
    ))
}
