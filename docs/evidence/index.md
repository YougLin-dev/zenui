---
doc-id: EVID-INDEX
layer: evidence
status: normative
date: 2026-11-18
owner: 测试与 CI/发布负责人（10y+）
sources:
  - archive/review/VALIDATION.md
  - archive/review/verification-contract.json
replaces: []
---

# 证据层索引

证据层只回答一个问题：**跑过什么，输出是什么。** 不写"应该"，不写"计划要验"。要建什么看 `spec/`，为什么这样定看 `decisions/`。

## 1. 本层文件与责任

| 文件 | 定义什么 | 状态 |
|---|---|---|
| `index.md`（本文件） | 复跑入口、主机约定、引用规则、陷阱 | normative |
| `validation.md` | `EVID-VAL-01…40`：每条命令 + 主机 + 结果 + 边界 | normative |
| `matrix.json` | `EVID-MATRIX`：上述运行与 11 组合同的机器投影 | normative |
| `claim-register.md` | `CLAIM-*`：可判真假的单句断言（唯一事实来源） | normative（研究员交付） |
| `probes.md` | `EVID-PROBE-01…05`：探针能力与证伪边界 | normative（本文件负责人，见 §1.1） |

### 1.1 `EVID-PROBE-*` 的归属

`EVID-PROBE-*` 由本文件（`index.md`）与 [`probes.md`](probes.md) 共同负责，不再"无人认领"。当前五条：

| ID | 探针 | 一句话 |
|---|---|---|
| `EVID-PROBE-01` | `examples/macro_poc` | rstml 只降级 `view/text/button` + `class` + `on:click\|change\|input`，产出 inspection `Node` |
| `EVID-PROBE-02` | `desktop_poc/lib/project_signals.rs` | 平坦 field sink：能去重/换边/整图释放；无 generation、`set` 不返回 `Result` |
| `EVID-PROBE-03` | `desktop_poc/lib/style.rs` | 9 个精确 class + 4 个前缀族、单激活状态；无 origin/importance/canonical order |
| `EVID-PROBE-04` | `examples/native_poc` | Parley/Taffy/sparse-strips 的最小适配器；baseline/intrinsic/synthesis 三处缺陷已证实 |
| `EVID-PROBE-05` | `examples/todolist` | 目标语法，**不可编译**（40 errors），只作 negative evidence |

分工：`probes.md` 写"能/不能证明什么"，`validation.md` 写"某次运行看到了什么"。同一事实两边都引用时，以 `CLAIM-*` 作为唯一断言行。

## 2. 复跑清单（干净机器）

### 2.1 前置

- 工具链：pin 只在 `SPEC-ROUTE-001` 定义；根 `rust-toolchain.toml` 是该 pin 的机器投影，本地与 CI 因此解析到同一个编译器。
- 根 `Cargo.toml` 自 M0 P1 起是 workspace（members `crates/core`、`crates/app`，`resolver = "3"`）；根 package `zenui` 仍是 workspace 成员。
- **三个 PoC 示例各自是独立 workspace**（各自有 `Cargo.toml` 的 `[workspace]`），因此对 PoC 的命令必须带 `--manifest-path`，缓存也按 workspace 分开；workspace 内的 crate 用 `-p` 选择。
- 根 `Cargo.toml` 设 `autoexamples = false`：`examples/todolist/main.rs` 是 design-only，**不应该**被当成可构建目标。
- 首次构建约 2–3 分钟编译依赖，三个 workspace 各自 `target/` 合计约 2.5 GB（`**/target/` 已被 gitignore）。

### 2.2 命令与期望结果

| # | 命令（仓库根） | 期望 | 对应 ID |
|---|---|---|---|
| 1 | `cargo test --manifest-path examples/desktop_poc/Cargo.toml --lib` | 7 passed | `EVID-VAL-12` |
| 2 | `cargo test --manifest-path examples/macro_poc/Cargo.toml --workspace` | 2 integration + 1 trybuild runner（内含 3 个 compile-fail）；其余 target 0 tests | `EVID-VAL-13` |
| 3 | `cargo test --manifest-path examples/native_poc/Cargo.toml --lib` | 4 passed | `EVID-VAL-14` |
| 4 | `cargo run --manifest-path examples/native_poc/Cargo.toml` | 打印 adapter，PASS，`[128, 0, 0, 128]` 与 CPU reference 一致 | `EVID-VAL-15` |
| 5 | `cargo run --manifest-path examples/desktop_poc/Cargo.toml` | 打印 adapter，PASS（underlay + LoadOp::Load + straight-alpha overlay） | `EVID-VAL-16` |
| 6 | `cargo test --manifest-path Cargo.toml` | **0 tests** —— 不是通过 | `EVID-VAL-17` |
| 7 | `rustc --edition 2024 --crate-type lib <tmp>/copy_string.rs`（源码见 `validation.md` `EVID-VAL-18`） | 必须 **E0204**（预期失败） | `EVID-VAL-18` |
| 8 | `rustc --edition 2024 --crate-type lib --emit=metadata --out-dir <tmp> examples/todolist/main.rs` | 必须 **40 个 error**（预期失败，证明 design-only） | `EVID-VAL-19` |
| 9 | `cargo run -p zenui-app --example todolist_headless --locked` | 9 步 trace；两次运行逐字节一致 | `EVID-VAL-20` |
| 10 | `cargo test --workspace --locked` | 40 passed（34 core + 6 golden），0 failed | `EVID-VAL-21` |

1–6 与 `EVID-VAL-12…17` 一一对应（顺序按 ID 排）；7–8 是两处**预期失败**的独立检查，对应 `EVID-VAL-18…19`；9–10 是 M0 的 headless 闭环，分别对应 `EVID-VAL-20`（trace）与 `EVID-VAL-21`（测试）。

第 10 条同一个命令也由 `.github/workflows/workspace.yml` 在 `ubuntu` / `macos` / `windows` 上执行，`EVID-VAL-22` 记录的是 main（`213f772`）上的那一次运行。CI 是**另一类主机**，它本机复跑不了，H2 的结果也不能替代它——结论按主机隔离（见 `validation.md` §1、§6.1）。

一键脚本（只读仓库；只往 `mktemp -d` 里写）：

```sh
set -eu
run() { echo "--- $* "; "$@"; }
run_fail() { echo "--- (expect failure) $* "; "$@" || echo "exit=$? (expected)"; }

run cargo test --manifest-path examples/desktop_poc/Cargo.toml --lib
run cargo test --manifest-path examples/macro_poc/Cargo.toml   --workspace
run cargo test --manifest-path examples/native_poc/Cargo.toml  --lib
run cargo run  --manifest-path examples/native_poc/Cargo.toml
run cargo run  --manifest-path examples/desktop_poc/Cargo.toml
run cargo test --manifest-path Cargo.toml
run cargo run -p zenui-app --example todolist_headless --locked
run cargo test  --workspace --locked

tmp=$(mktemp -d)
cat > "$tmp/copy_string.rs" <<'EOF'
#[derive(Copy, Clone)]
struct Row { id: u64, title: String }
fn main() {}
EOF
run_fail rustc --edition 2024 --crate-type lib --out-dir "$tmp" "$tmp/copy_string.rs"
run_fail rustc --edition 2024 --crate-type lib --emit=metadata --out-dir "$tmp" examples/todolist/main.rs
```

**GPU 两条命令（4、5）在没有 adapter 的机器上会失败**：这必须记成 `blocked`（缺前置条件），不是 `failed`，更不能拿别的机器的 readback 顶替。用 `--offline` 复跑时注意 `trybuild` 不在离线索引里，第 2 条会解析失败——那是环境问题，不是代码问题。

### 2.3 一次复跑要记录什么

最少五样：**commit、主机（OS + 架构 + adapter/driver）、命令、退出码、输出摘要**。缺任一项，结论就不能升级为 `VERIFIED`。

## 3. 主机约定

- `H1`：上一轮 Windows 11 / NVIDIA RTX 4060 Laptop / Vulkan，**commit 未记录**。
- `H2`：本轮 macOS 27.0.1 / Apple M4 / Metal，commit `ab5339b`（EVID-VAL-12…19）/ `83b21a4`（EVID-VAL-20/-21）；逐条 commit 见 `validation.md`。
- 结论**按主机隔离**：`EVID-VAL-05` 与 `EVID-VAL-15` 是同一个探针在两台机器上的两次观测，可并列引用，不可互相替代。
- 这台机器（H2）**没有** Windows/Linux runner、没有中文 IME、没有屏幕阅读器。C Gate 的任何结论在这里都只能是 `blocked`。

## 4. 引用规则

- 引用运行事实：`[EVID-VAL-15](validation.md)`（条目名与 ID 在 `validation.md` §2 索引表里定位；不在正文里手写锚点，避免标题改动后断链）。
- 引用探针的能力边界：`[EVID-PROBE-04](probes.md)`（"这份代码能/不能证明什么"），**不要**用 `EVID-VAL-*` 代替——后者只说明某次运行的结果。
- 引用 JSON：`EVID-MATRIX.runs[i]` / `EVID-MATRIX.contracts[i]`。
- 一句断言（"某能力成立/不成立"）只能在 `claim-register.md` 里落 `CLAIM-*`，然后**由它**引用 `EVID-*`。spec 与 product 文档只引用 `CLAIM-*` 或 `EVID-*`，不自行复述数字。
- 报告覆盖度必须给**分子/分母 + 来源**，例如 `2/42 条 Gate 判据处于 PROBE 级，来源 EVID-MATRIX.coverage`。

## 5. 新增一条 EVID-VAL 的流程

1. **只增不改**：已有 ID 的结论变了，新开一条（顺延一个 ID）并在旧条目写明 `supersededBy`；不要原地改写历史输出。
2. **失败也记**：编译失败、断言红、跑不起来都是证据（`EVID-VAL-09/-11/-18/-19` 都是失败记录），比"没跑"有用得多。
3. **写清边界**：每条必须有一行"不证明什么"。没有这一行的条目视为不合格。
4. **同步 `matrix.json`**：加 run 对象；若影响 Gate 判据，改 `08-gates.md` 的状态列并更新 `coverage` 计数。
5. **不把夹具放进仓库外的目录**：一旦在临时目录里做验证，就接受它"永久不可复跑"（C 类）。要变成长期证据，必须落到 `crates/*/tests` 或 `examples/*/tests`。

## 6. 已知陷阱（引用证据前先读）

| 陷阱 | 表现 | 正确做法 |
|---|---|---|
| 把 0 tests 当通过 | 根 package `exit 0` 但 0 tests | 记为"可构建"，不记能力 |
| 目标示例被当可运行程序 | `examples/todolist/main.rs` 有 40 个编译错误 | 它是 design-only（`EVID-VAL-19`）；根 `autoexamples=false` 是有意为之，不是掩盖错误 |
| 程序自报主机错误 | 两个 GPU 探针都硬编码打印 `SCOPE: ... Windows ...` | 主机以 H 表为准；该字符串待实现负责人修 |
| 测试名误导 | `hover_only_supersedes_the_matching_property` 沿用已撤销措辞 | 它只断言 class 内部的 hover 变体覆盖，不涉及 inline vs class 的 origin 顺序 |
| 文档未记录 commit | H1 的所有结论没有 commit | 引用时声明"commit 未知"，不得当作可精确复现的基线 |
| 探针通过 ⇒ 产品能力 | 53 条测试全绿（含 M0 的 40 条） | 它们分别只覆盖平坦 sink、rstml 形状、Taffy/Parley 测量、headless 身份/事务骨架；见各自条目的"不证明" |

## 7. 与 Gate 的对应

Gate 判据（`SPEC-GATE-*`）在 `docs/spec/08-gates.md`。当前映射：42 条判据中 **0 条 VERIFIED、6 条 PROBE、36 条 OPEN**（`EVID-MATRIX.coverage`）。

最容易被误读的两处已在该文件显式标注：

- `SPEC-GATE-A0-04`（事务原子性）**已有反例**：`EVID-VAL-08` 的嵌套 batch 外泄中间值。它现在是红的，不是待验的灰。
- `SPEC-GATE-B-01/-02/-03`（baseline / intrinsic / 字体合成）**已有源码级缺陷**：`validation.md` §7 的 SRC-01…03。测试绿不改变这些缺陷。

## 8. 维护

本轮新增：`index.md`、`validation.md`、`matrix.json`、`probes.md`（commit `ab5339b` 之后、队长统一提交前）。`claim-register.md` 由研究员交付，本层只引用不改写。原始记录 `archive/review/VALIDATION.md`、`archive/review/validation-matrix-raw.json` 保持原文不动，归档后由本层接管引用。

编号一致性：`claim-register.md` 的证据键表已覆盖 `EVID-VAL-01…40`，`matrix.json` 的 `runs` 与 `counts` 已同步（`runsTotal=40`、`testsPassedH2=492`、`testsPassedCi=120`、`runsReproducibleHere=28`、`runsOnCi=1`）。

源码事实的引用形式（回应 `claim-register.md` 的 C-18）：不新增 `EVID-SRC`，统一写成 `EVID-PROBE-0N`（`probes.md` 条目内附 `SRC path:line`）。这样源码事实仍落在 evidence 层、仍是 `EVID-*` 形态，且附带"能/不能证明什么"的边界，比裸 `path:line` 更难被断章取义。
