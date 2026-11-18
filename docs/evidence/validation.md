---
doc-id: EVID-VAL
layer: evidence
status: normative
date: 2026-11-18
owner: 测试与 CI/发布负责人（10y+）
sources:
  - archive/review/VALIDATION.md
  - archive/research/validation-and-roadmap.md
  - archive/review/text-layout-review.md
  - examples/（commit ab5339b 的源码，本轮可直接复跑）
replaces: []
---

# 执行记录 EVID-VAL

本文件是**运行事实**的唯一出处：每条记录 = 一条命令 + 一台主机 + 一个可对照的退出结果。
规范在 `spec/`，事实断言在 `evidence/claim-register.md`（`CLAIM-*` 引用这里的 `EVID-VAL-*`）。
**本文件不写"应该"，也不写"已实现"** —— 通过一个探针命令不等于产品能力成立。

## 0. 读法：三类证据，不可混用

| 类别 | 含义 | 可信度 | 出现在 |
|---|---|---|---|
| **A 本轮实测** | 本轮（2026-11-18）在主机 H2 上亲自执行、看到输出 | 可复跑到命令级 | EVID-VAL-12 起 |
| **B 上一轮转述** | 上一轮在主机 H1 执行，本轮仅转述其记录 | 只能证明"当时记录如此" | EVID-VAL-01…06 |
| **C 仓库外夹具** | 位于仓库之外的临时目录，**本仓库无法复跑** | 一次性，永久不可复现 | EVID-VAL-07…11 |
| **D CI 托管 runner** | 本轮在 GitHub Actions 的 ubuntu/macos/windows runner 上执行，日志与 run URL 可查；本机不是那些主机，不能拿本机结果代替 | 可复跑到 run 级（重跑 workflow） | EVID-VAL-22 |

两条铁律：

1. **不得跨主机复制结论。** H2 通过只说明在 H2 通过；H1 的 Windows/Vulkan 结论不能由 H2 的 macOS/Metal 结果替代，反之亦然。
2. **不得把 0 tests 的 target 算作完成。** 根 package 的 `zenui` 目前没有任何测试（EVID-VAL-04 / -17），它的 `exit 0` 只证明能构建。

## 1. 主机与溯源

| 主机 | OS | Rust | GPU / adapter | commit | 证据性质 |
|---|---|---|---|---|---|
| **H1** | Windows 11，MSVC 工具链 | `1.99.1 x86_64-pc-windows-msvc` | NVIDIA GeForce RTX 4060 Laptop GPU / Vulkan | 未记录 | 上一轮记录（转述） |
| **H2** | macOS 27.0.1 (26A434)，Apple M4（10 核 GPU，Metal 4） | `1.99.1 (58cf8f9c3 2026-10-13) aarch64-apple-darwin` | Apple M4 / `metal` | `ab5339b`（EVID-VAL-12…19）/ `83b21a4`（EVID-VAL-20/-21）/ bb16522b03abd7fb0102fb092c49284615416fbc（EVID-VAL-23…26） | **本轮实测**（commit 逐条见各条目） |
| **CI** | GitHub Actions 矩阵：`ubuntu-26.04` / `macos-26-arm64` / `windows-2025-vs2026` | `rustc 1.99.1 (58cf8f9c3 2026-10-13)`，三个 runner 一致（取自 `rust-toolchain.toml` 的 pin） | 无（headless；A0 不需要 GPU） | `213f772`（main 上的 PR #5 merge commit） | **本轮 CI 实测**，run [`44284925421`](https://github.com/YougLin-dev/zenui/actions/runs/44284925421) |

> H1 的 commit 在原始记录中未写下，这是本轮无法补的缺口；引用 H1 结果时应同时声明"commit 未知"。

## 2. ID 索引（本文件声明过的全部运行）

| ID | 类别 | 命令（短） | 主机 | 结果 | 状态 |
|---|---|---|---|---|---|
| EVID-VAL-01 | B | `desktop_poc --lib` | H1 | 7 passed | VERIFIED（H1 记录） |
| EVID-VAL-02 | B | `macro_poc --workspace` | H1 | 2 + 3 compile-fail | VERIFIED（H1 记录） |
| EVID-VAL-03 | B | `native_poc --lib` | H1 | 4 passed | VERIFIED（H1 记录） |
| EVID-VAL-04 | B | 根 `cargo test` | H1 | 0 tests | VERIFIED（H1 记录） |
| EVID-VAL-05 | B | `native_poc` 运行 | H1 | PASS，NVIDIA/Vulkan | VERIFIED（H1 记录） |
| EVID-VAL-06 | B | `desktop_poc` 运行 | H1 | PASS，NVIDIA/Vulkan | VERIFIED（H1 记录） |
| EVID-VAL-07 | C | 临时 `type_contract` | H1 | PASS（shape） | OPEN（不可复跑） |
| EVID-VAL-08 | C | 临时 rstml parse + 嵌套 batch | H1 | parse PASS / batch **RED** | OPEN（反向证据，见 A0-04） |
| EVID-VAL-09 | C | 临时 `final-contract` | H1 | 首次失败 → 修正后 PASS | OPEN（不可复跑） |
| EVID-VAL-10 | C | 临时 `transaction_contract` | H1 | 首次失败 → 修正后 PASS | OPEN（不可复跑） |
| EVID-VAL-11 | C | 临时 `copy-probe` | H1 | **E0204 失败** | OPEN（同类已由 -18 在 H2 复现） |
| EVID-VAL-12 | A | `desktop_poc --lib` | **H2** | 7 passed | VERIFIED |
| EVID-VAL-13 | A | `macro_poc --workspace` | **H2** | 2 + 3 compile-fail | VERIFIED |
| EVID-VAL-14 | A | `native_poc --lib` | **H2** | 4 passed | VERIFIED |
| EVID-VAL-15 | A | `native_poc` 运行 | **H2** | PASS，Apple M4/Metal | VERIFIED |
| EVID-VAL-16 | A | `desktop_poc` 运行 | **H2** | PASS，Apple M4/Metal | VERIFIED |
| EVID-VAL-17 | A | 根 `cargo test` | **H2** | 0 tests | VERIFIED（0 = 无测试） |
| EVID-VAL-18 | A | `#[derive(Copy)] struct{String}` | **H2** | **E0204** | VERIFIED |
| EVID-VAL-19 | A | 目标示例 `todolist/main.rs` 单独编译 | **H2** | **40 errors** | VERIFIED（不可编译=预期） |
| EVID-VAL-20 | A | `cargo run -p zenui-app --example todolist_headless --locked` | **H2** | 9 步 trace，两次运行逐字节一致 | VERIFIED（仅 A0-01 的命令与 trace；A0-02…11 仍 OPEN） |
| EVID-VAL-21 | A | `cargo test --workspace --locked` | **H2** | 40 passed（34 + 6） | VERIFIED |
| EVID-VAL-22 | D | `cargo test --workspace --locked` | **CI** | 40 passed × 3 平台 | VERIFIED（CI 日志） |
| EVID-VAL-23 | A | `cargo test -p zenui-core --test gates --locked` | **H2** | 6 passed（A0-03/04/06 夹具） | VERIFIED（四条 mutation 逐一判红，见条目） |
| EVID-VAL-24 | A | `cargo test -p zenui-core --locked` | **H2** | 65 passed（45 + 6 + 8 + 6 doctest） | VERIFIED |
| EVID-VAL-25 | A | `cargo run -p zenui-app --example todolist_headless --locked` | **H2** | 9 步 trace，sha256 与 EVID-VAL-20 相同 | VERIFIED（语义未变，不是重新录制） |
| EVID-VAL-26 | A | `cargo clippy --workspace --all-targets --locked -- -D warnings` | **H2** | exit 0 | VERIFIED（仅本机，C-04 仍 OPEN） |
| EVID-VAL-27 | A | `cargo test -p zenui-core --test regressions --locked` | **H2** | 8 passed（审计回归夹具） | VERIFIED（三条 mutation 判红，见条目） |

## 3. 仓库内测试：H1 原始记录（转述）

以下四条转述自 `archive/review/VALIDATION.md` §2，**本轮未在 H1 观测**。命令均在仓库根执行。

### EVID-VAL-01 desktop_poc 库单测

- 命令：`cargo test --manifest-path examples/desktop_poc/Cargo.toml --lib`
- 结果：`exit 0`，**7 passed** —— 4 条 reactive（局部 sink、source 切换的 edge 清理、batch 基础去重、dispose）+ 3 条 typed style（字段映射、局部条件覆盖、未知 token 定位）。
- 不证明：完整 Memo/effect/resource、For 行生命周期、Tailwind cascade 正确性。

### EVID-VAL-02 macro_poc workspace

- 命令：`cargo test --manifest-path examples/macro_poc/Cargo.toml --workspace`
- 结果：`exit 0`，**2 个 integration test + 1 个 trybuild runner**；runner 内 **3 个 compile-fail**（`html_element`、`script_event`、`unknown_class`）。其余 lib/doc-test target 为 **0 tests**，不得计入。
- 不证明：完整 intrinsic / PascalCase component / For / Show、renamed facade、生产 MountPlan。

### EVID-VAL-03 native_poc 库单测

- 命令：`cargo test --manifest-path examples/native_poc/Cargo.toml --lib`
- 结果：`exit 0`，**4 passed** —— ICU 复杂文字边界、CPU premultiply、Taffy+Parley 测量与文字变更失效、CJK shaping + CPU glyph 像素。
- 不证明：mixed-bidi intrinsic、真实 baseline、字体合成、系统输入/无障碍。

### EVID-VAL-04 根 package

- 命令：`cargo test --manifest-path Cargo.toml`
- 结果：`exit 0`，**0 tests passed**。
- 结论：**只证明根 hello-world package 能构建测试 target**，不能作为任何能力的完成证明。

## 4. 仓库内 GPU 探针：H1 原始记录（转述）

### EVID-VAL-05 native_poc 运行（sparse-strips GPU）

- 命令：`cargo run --manifest-path examples/native_poc/Cargo.toml`
- 结果：`exit 0`；`Adapter: NVIDIA GeForce RTX 4060 Laptop GPU / Vulkan`；PASS：sparse-strips GPU、真实 Parley/Glifo CJK 像素、普通外部纹理绑定；半透明红 readback `[128,0,0,128]`，与 CPU reference 一致（premultiplied）。
- 不证明：真实窗口、surface/resize/DPI、device-loss、mask/filter、retained paint cache、性能。

### EVID-VAL-06 desktop_poc 运行（Classic vello 0.12）

- 命令：`cargo run --manifest-path examples/desktop_poc/Cargo.toml`
- 结果：`exit 0`；同机 NVIDIA/Vulkan；PASS：Vello underlay、foreign wgpu `LoadOp::Load`、straight-alpha overlay 合成。
- 边界：Classic 0.12 是 **probe**，不是第二个生产后端（FINAL §0）；straight-alpha 与 premultiplied 不能互相推导。

## 5. 仓库外夹具：H1 一次性证据（C 类）

这些夹具位于上一轮的临时目录（Windows `%TEMP%`），**本仓库不含其源码，无法复跑**。引用时只能是"曾经观察到"，状态一律 `OPEN`。

### EVID-VAL-07 `zenui-type-contract` 类型形状

- 内容：普通 Rust `Result<MountPlan, MountError>` 函数体内可写 `let title = title?;`，同时 `Copy` handle 可被 move 闭包独占。
- 结论范围：**只是语言形状**。不证明 `#[component]`/`view!` 会保留该形状，也不证明 Props lowering、Plan 创建或 runtime commit。

### EVID-VAL-08 rstml parse-only + 嵌套 batch 反例

- PASS 部分：目标 node/component/attribute 形状能被真实 `rstml` 解析。
- **RED 部分（关键）**：复用现有 PoC 的嵌套 batch —— 外层写 `a=true`、内层写 `b=true`、外层再写 `b=false`，实际产出两条 patch 且包含中间值 `true`。这**是 A0 事务原子性的现存反例**，并未因同一批里别的夹具通过而消失（映射到 `SPEC-GATE-A0-04`）。

### EVID-VAL-09 / -10 `final-contract` 与 `transaction_contract`

- EVID-VAL-09 首次复跑**编译失败**：`WriteError` 未实现 `Debug`（E0277）、`dyn Command` 按值调用（E0161）。为临时自写夹具补 `Debug` 并把 trait 改为 `fn apply(self: Box<Self>, ...)` 后 `exit 0`，仅证明"Result mount + Copy handle + deferred transaction API 在 safe Rust 中可编译"。
- EVID-VAL-10 首次复跑失败（泛型缺 `Default` bound），补 `AppPending: Default` 后 PASS，输出自述 "actual staging/commit remains A0 work"。
- **换算规则**：这两条说明"改到能编译"很容易，"行为正确"一点也没被证明。`finish()` 只在 scratch state 上运行，不模拟 current model、journal、stable topo 或 View/Taffy/focus checkpoint。

### EVID-VAL-11 Copy+String 历史失败

- 上一轮独立 `zenui-copy-probe` 在 E0204 处实际失败：`#[derive(Copy)]` 的结构不能持有 `String`。本轮已在 H2 用最小夹具独立复现同类错误（EVID-VAL-18）。

## 6. 本轮在 H2（macOS / Apple M4 / Metal）的复跑

主机与 commit 见 §1。以下输出为本轮实际观测，命令均以仓库根为工作目录。

### EVID-VAL-12 desktop_poc 库单测（H2）

```text
$ cargo test --manifest-path examples/desktop_poc/Cargo.toml --lib
running 7 tests
test style::tests::complete_utility_keys_lowering_to_typed_fields ... ok
test style::tests::hover_only_supersedes_the_matching_property ... ok
test style::tests::invalid_values_and_unknown_classes_report_source_location ... ok
test project_signals::tests::multiple_writes_in_one_per_frame_batch_deduplicate_the_sink ... ok
test project_signals::tests::changing_dependencies_removes_inactive_edges ... ok
test project_signals::tests::disposing_owner_drops_sinks_and_disables_future_updates ... ok
test project_signals::tests::source_update_runs_only_matching_sink ... ok
test result: ok. 7 passed; 0 failed; 0 ignored
```

- 新事实：H1 的 7 条断言在 **aarch64-apple-darwin + Metal** 上逐条同名通过 → 这些纯逻辑/解析层断言不依赖 Windows 或 GPU。
- 边界不变：仍是平坦 sink 探针，不是 production Owner/arena。

### EVID-VAL-13 macro_poc workspace（H2）

```text
$ cargo test --manifest-path examples/macro_poc/Cargo.toml --workspace
     Running tests/compile_fail.rs
test tests/compile_fail/html_element.rs ... ok
test tests/compile_fail/script_event.rs ... ok
test tests/compile_fail/unknown_class.rs ... ok
test unsupported_macro_inputs_emit_native_diagnostics ... ok
test result: ok. 1 passed            (compile_fail runner)
     Running tests.rs
test html_shape_compiles_classes_and_preserves_rust_closures ... ok
test literal_rust_text_is_static_content_not_a_child_container ... ok
test result: ok. 2 passed            (integration)
其余 lib/doc-test target: 0 tests
```

- 新事实：`trybuild` 的 3 个 compile-fail 期望在 macOS 上逐字匹配（`.stderr` 快照无平台漂移）。这对 CI 跨平台是一个正面信号：诊断文本目前是稳定的。
- 不证明：macro 写死 `::zenui_macro_runtime`，renamed-facade 仍未验证。

### EVID-VAL-14 native_poc 库单测（H2）

```text
$ cargo test --manifest-path examples/native_poc/Cargo.toml --lib
running 4 tests
test tests::complex_script_dictionary_actually_changes_word_boundaries ... ok
test tests::software_semi_transparency_is_premultiplied ... ok
test tests::cjk_parley_produces_glyph_runs_and_software_pixels ... ok
test tests::taffy_measures_parley_and_invalidates_changed_text ... ok
test result: ok. 4 passed; 0 failed
```

- 新事实：`parley complex-scripts` + ICU 词典在 macOS 上同样装载成功且边界断言成立 —— 说明词典数据缺失问题不是 Windows 专有，发布 artifact 必须在每个平台各自验证存在。
- 不证明：baseline、intrinsic、字体合成（见 §8 源码核对）。

### EVID-VAL-15 native_poc 运行（H2，首个非 NVIDIA 适配器观测）

```text
$ cargo run --manifest-path examples/native_poc/Cargo.toml
Adapter: "Apple M4" / metal
PASS: sparse-strips GPU, real Parley/Glifo CJK pixel runs, ordinary external texture binding.
Transparent semi-red GPU pixel: [128, 0, 0, 128]; CPU reference matches.
SCOPE: headless Windows RGBA graph; ...
```

- 新事实：**同一 sparse-strips 场景在 Metal 上得到与 Vulkan 相同的 readback 值**，且与 CPU reference 一致。这是本仓库第一条跨 OS + 跨图形后端的 GPU interop 观测。
- **必须点名的缺陷**：程序自己打印的 `SCOPE: ... Windows ...` 是**硬编码字符串**，与实际主机不符。任何人引用这段输出都会误报主机。该字符串需要由实现负责人修（不在我的文件范围），在修好之前，引用本条目必须以本文件的 H2 主机表为准。
- 不证明：窗口/surface、resize/DPI、device-loss、mask/filter、性能、glTF。两条后端不足以称"GPU 矩阵"，只能记作 2 个数据点。

### EVID-VAL-16 desktop_poc 运行（H2，Classic probe）

```text
$ cargo run --manifest-path examples/desktop_poc/Cargo.toml
Adapter: "Apple M4" / metal
PASS: Vello underlay, foreign wgpu LoadOp::Load, and straight-alpha overlay compositing.
SCOPE: Windows headless RGBA texture graph only; ...
```

- 新事实：Classic 0.12 的 straight-alpha overlay 行为在 Metal 上复现；同样带硬编码 "Windows" SCOPE 字符串。
- 边界：Classic 仍是 probe，不构成第二个生产后端承诺。

### EVID-VAL-17 根 package（H2）

```text
$ cargo test --manifest-path Cargo.toml
running 0 tests
test result: ok. 0 passed
```

- 与 EVID-VAL-04 一致：**0 tests**。

### EVID-VAL-18 Copy+String 编译失败（H2，最小夹具）

```text
error[E0204]: the trait `Copy` cannot be implemented for this type
 --> copy_string.rs:2:8
  | struct Row { id: u64, title: String }
```

- 用途：为 `SPEC-GATE-A0-02` 提供"语言层不可能"的确证。它是**最小语法夹具**，不是仓库测试；正式 fixture 仍须进 `crates/core/tests/`。
- 复跑：任意目录新建该 5 行文件，`rustc --edition 2024 --crate-type lib copy_string.rs`。

### EVID-VAL-19 目标示例不可编译（H2）

```text
$ rustc --edition 2024 --crate-type lib --emit=metadata examples/todolist/main.rs
error[E0433]: cannot find module or crate `zenui` in this scope
error: cannot find attribute `component` in this scope
error: cannot find macro `view` in this scope
error[E0425]: cannot find type `CompletedField` in this scope
... 共 40 个 error
```

- 用途：把"`examples/todolist/main.rs` 是 design-only、不是可运行程序"从叙述变成可复跑的观测。任何把该文件当 A1 证据的说法都可以据此驳回。

### EVID-VAL-20 Headless core 与 `todolist_headless` trace（H2，A0-01 闭环）

- 分支：`feat/MORT-4-a0-headless-core`；被测代码 commit `83b21a46795e4ef1f7e9955b31e76cc2fe430468`（后续只有 docs 提交，不改变 trace 与测试结果）。
- 主机：H2（macOS 27.0.1 / Apple M4）；`rustc 1.99.1 (58cf8f9c3 2026-10-13) aarch64-apple-darwin`；`cargo 1.99.1 (5f94df478 2026-08-27)`。
- 场景顺序与 `SPEC-GATE-A0-01` 一致：empty → insert A/B/C → toggle B → Active → reorder → delete focused → bulk clear → invalid id rollback → 下一次 insert 成功。

```text
$ cargo run -p zenui-app --example todolist_headless --locked
[step 01 empty]
  detail=no rows
  state store_rows=0 visible_rows=0 remaining=0 has_completed=false filter=all store_titles=- visible_titles=- focus=- caret=-
  delta query_evals=1 id_materializations=1 mounts=0 unmounts=0 memo_recomputes=3 commits=1 focus_moves=0 caret_ops=0 io_reads=0 io_writes=0 string_clones=0
  graph edges=3 active_sources=1 geometry_epoch=1 live_owners=1 cleanup_runs=0 sink=0

... (steps 02–04 略，逐字见 golden 文件)

[step 05 reorder]
  detail=reorder C,A,B; focus A
  state store_rows=3 visible_rows=2 remaining=2 has_completed=true filter=active store_titles=C,A,B visible_titles=C,A focus=view#1 caret=view#1:1
  delta query_evals=1 id_materializations=1 mounts=0 unmounts=0 memo_recomputes=3 commits=1 focus_moves=1 caret_ops=1 io_reads=0 io_writes=1 string_clones=0
  graph edges=4 active_sources=1 geometry_epoch=7 live_owners=3 cleanup_runs=1 sink=0

[step 06 delete-focused]
  detail=delete focused A
  state store_rows=2 visible_rows=1 remaining=1 has_completed=true filter=active store_titles=C,B visible_titles=C focus=view#3 caret=view#3:1
  delta query_evals=1 id_materializations=1 mounts=0 unmounts=1 memo_recomputes=3 commits=1 focus_moves=1 caret_ops=1 io_reads=0 io_writes=1 string_clones=0
  graph edges=4 active_sources=1 geometry_epoch=8 live_owners=2 cleanup_runs=2 sink=0

[step 08 invalid-rollback]
  detail=result=InvalidHandle(vacant)
  state store_rows=1 visible_rows=1 remaining=1 has_completed=false filter=active store_titles=C visible_titles=C focus=view#3 caret=view#3:1
  delta query_evals=0 id_materializations=0 mounts=0 unmounts=0 memo_recomputes=0 commits=1 focus_moves=0 caret_ops=0 io_reads=0 io_writes=0 string_clones=0
  graph edges=4 active_sources=1 geometry_epoch=8 live_owners=2 cleanup_runs=2 sink=0


[summary]
  steps=9
  state store_rows=2 visible_rows=2 remaining=2 has_completed=false filter=active store_titles=C,D visible_titles=C,D focus=view#3 caret=view#3:1
  total query_evals=8 id_materializations=8 mounts=4 unmounts=2 memo_recomputes=22 commits=9 focus_moves=2 caret_ops=2 io_reads=4 io_writes=8 string_clones=4
  graph edges=4 active_sources=1 geometry_epoch=10 live_owners=3 cleanup_runs=2 sink=0
[status] outcome=ok
```

```text
$ cargo run -p zenui-app --example todolist_headless --locked > run1.txt
$ cargo run -p zenui-app --example todolist_headless --locked > run2.txt
$ diff -u run1.txt run2.txt
（无输出）

$ sha256sum run1.txt crates/app/tests/golden/todolist_headless.trace
fb16d571748949225b46a5eb78abc72e5badd13e3e2b6cede7b6b7aefd392f21  run1.txt
fb16d571748949225b46a5eb78abc72e5badd13e3e2b6cede7b6b7aefd392f21  crates/app/tests/golden/todolist_headless.trace
```

- 测试命令与计数见 EVID-VAL-21：一个 `EVID-VAL` 条目只对应一条命令。
- 依赖边界：`crates/core/Cargo.toml` 的 `[dependencies]` 为空（纯 `std`）；`crates/app` 只依赖 `crates/core`；根 package、`zenui-core`、`zenui-app` 均 `publish = false`。`Cargo.lock` 只新增 `zenui-core` 与 `zenui-app` 两个 package。
- 本机附加检查（不在 CI 里）：`cargo clippy --workspace --all-targets --locked -- -D warnings` 与 `cargo fmt --all -- --check` 均通过。
- 计数口径：`io_writes` 定义为「一次改变模型的 store 变更方法调用 = 一次写入 = 一次 epoch 推进」；`clear_completed` 删 N 行仍记 1 次，顺序未变的 `reorder` 与同值 `set_completed` 记 0 次。`store_rows`/`store_titles` 是整个 Store，`visible_rows`/`visible_titles` 是当前挂载的可见集：两者在 `step 04…06` 不相等（3/2、3/2、2/1），在 `step 07…09` 因只剩可见行而重新相等（1/1、1/1、2/2）。
- 确定性构造：全 crate 不使用 `HashMap`/`HashSet`（`BTreeMap`/`BTreeSet` 与 `Vec` 顺序）；trace 不含地址、耗时、线程 id；`RowSet` 只含稳定 `RowKey`，不含 `String`。
- **不证明什么**：这条只说明 `SPEC-GATE-A0-01` 规定的命令可跑、trace 确定。测试通过与否见 EVID-VAL-21，两者都不证明 A0-02…A0-11 的任何一条：
  - `step 01` 的空 Store 上 `id_materializations=1`——空态仍靠物化集合判定，`SPEC-GATE-A0-08` 未满足；
  - 每次 store 写都按整 epoch 重算，`step 03/05/06/07` 的 `id_materializations=1`——`SPEC-GATE-A0-07` 的 membership 索引与反向边未接线；
  - `step 07` 的批量 Clear 在 trace 中没有任何 focus/caret 操作——被清掉的 B 在此之前已被 Active filter unmount——所以 bulk-clear「只结算一次 intent」的语义没有夹具覆盖，`FocusRemovalIntent` 的完整顺序与取消语义也未实现（`SPEC-GATE-A0-10`）；
  - 无 staging journal 与阶段守卫，`SPEC-GATE-A0-04`/`A0-06` 未验证（`TODO(A0-04)`、`TODO(A0-06)`）；`graph` 无 cycle / `WriteDuringCompute` 领域错误（`A0-05`）；
  - `A0-02` 无 trybuild fixture；`A0-03` 有 4 条 core 单元测试——2 条在 `store`（`handle_from_another_store_is_rejected`、`removed_row_rejects_its_old_handle`）、2 条在 `arena`（`recycled_slot_rejects_the_previous_generation`、`freed_slot_without_reuse_is_vacant`）——但 handle 侧 `get`/`with`、remount 的 generation 复用与 gate 级 stale dispatch 夹具未实现（`TODO(A0-03)`）；`A0-09` 的 unmount/setup 计数**已采集**（`reorder_preserves_view_ids` 断言 reorder 后 `mounts`/`unmounts` 不变，trace `step 05` 同样为 0/0），仍缺重复 key 在 view 层的拒绝与专属 setup/identity 夹具；`A0-11` 的 churn 基线未采集。
- Gate 状态：`docs/spec/08-gates.md` 中 `SPEC-GATE-A0-01` 仍写作 `OPEN`。该列属于 spec 层，本次未改动；翻转由文档负责人在归档本条目后单独提交。

### EVID-VAL-21 Headless workspace 测试（H2）

- 分支：`feat/MORT-4-a0-headless-core`；被测代码 commit `83b21a46795e4ef1f7e9955b31e76cc2fe430468`。
- 主机：H2（同 EVID-VAL-20）。与 EVID-VAL-20 是同一次会话里的两条命令，按「一条记录一个命令」拆开。

```text
$ cargo test --workspace --locked
     Running unittests src/lib.rs (zenui_core)          34 passed; 0 failed
     Running tests/golden_trace.rs (zenui_app)           6 passed; 0 failed
     其余 target（根 package 与 zenui-app lib）           0 tests
（合计 40 passed / 0 failed）
```

- 覆盖：`zenui-core` 的 identity/slot、Store 计数、query 缓存、memo 图、owner 释放顺序、view 焦点与 caret；`zenui-app` 的 golden trace 六个断言。
- `#[cfg(debug_assertions)]` 的 `unbalanced_end_panics_without_committing` 只在 debug 构建运行；release 下由 `#[cfg(not(debug_assertions))]` 的版本接替，两种构建的用例总数都是 34。
- **不证明什么**：0-test target（根 package、`zenui-app` lib）不计为通过；这些单测不构成 A0-02…A0-11 的 gate 级证据。

## 6.1 CI 矩阵：main 上的 workspace 运行

### EVID-VAL-22 三平台 `cargo test --workspace --locked`（CI，commit `213f772`）

- 主机：**CI**（见 §1），不是 H2。三个 runner 分别是 `ubuntu-26.04`、`macos-26-arm64`、`windows-2025-vs2026`。
- 被测 commit：`213f772452cbb49b0330d9290d7d8cfc14ed673d`（PR #5 的 merge commit，即 workspace 门禁落地的提交）。
- workflow：[`workspace` run 44284925421](https://github.com/YougLin-dev/zenui/actions/runs/44284925421)，三个 job 全部 `success`；每个 job 的 `Test` 步骤都输出同样的四个 target。

```text
$ cargo test --workspace --locked        # ubuntu-26.04 / macos-26-arm64 / windows-2025-vs2026 各一次
     Running unittests src/main.rs (zenui)              0 passed
     Running unittests src/lib.rs (zenui_app)           0 passed
     Running tests/golden_trace.rs (zenui_app)          6 passed
     Running unittests src/lib.rs (zenui_core)         34 passed
（每个平台合计 40 passed / 0 failed）
```

- **这条记录只对应一条命令**：`cargo test --workspace --locked`。同一 workflow 里的 `cargo fmt --all -- --check` 与 `cargo clippy --workspace --all-targets --locked -- -D warnings` 是同一 job 的单独步骤，日志在同一个 run 里可查，但**没有独立的 EVID 条目**——本文件不把它们并入这条记录，也不据此宣布任何 lint 结论。
- 新事实：同一 commit 的 40 条测试在三个平台各自通过，且 `rustc` 三处都是 `1.99.1`——`rust-toolchain.toml` 的 pin 在三个平台上真的被解析成同一个编译器，而不是各 runner 自带的默认版本。
- 新事实：`tests/golden_trace.rs` 的逐字节比对在 `windows-2025-vs2026` 上成立。这是 `.gitattributes` 强制 LF 的实证：若 Windows 检出为 CRLF，`include_str!` 的内容与 golden 文件都会变形，测试与 `EVID-VAL-20` 的 sha256 会同时失败。
- **不证明什么**：任何 GPU / 窗口 / 平台后端能力（A0 是纯 `std` 的 headless core）；`cargo package`、renamed facade 与发布门（`SPEC-GATE-C-04`）——命令集里没有它们，这条记录不能把 C-04 标绿。
- 计数口径：`testsPassedCi = 120` 是 40 条 × 3 个 runner 的**执行次数**，不是 120 条不同的测试，也不得与 H2 的 53 相加（见 `EVID-MATRIX.countsNote`）。

## 7. 本轮源码核对（不是测试，但可复核）

以下四条是**读源码得到的事实**，用于防止"测试绿了所以没问题"的误判。逐条给出可对照的位置。

| 编号 | 事实 | 位置 | 与哪条规范冲突 |
|---|---|---|---|
| SRC-01 | Parley 首 baseline 被算出来了，但传给 Taffy 的 measure 闭包只返回 `.size`，baseline 被丢弃 | `examples/native_poc/src/layout.rs:27`（计算）→ `:104`（`.size` 丢弃） | `SPEC-GATE-B-01` |
| SRC-02 | measure 只用 `known.width` 调测量，不区分 available space / min-max-content | `examples/native_poc/src/layout.rs:104` | `SPEC-GATE-B-02` |
| SRC-03 | 全 crate 零处引用 `synthesis`/`embolden`/`skew`：Parley run 的字体合成没有转给 glifo | `examples/native_poc/src/`（全目录 grep 计数 0） | `SPEC-GATE-B-03` |
| SRC-04 | GPU 探针用固定绿色 `LoadOp::Clear{0,1,0,1}` 当目标背景 | `examples/native_poc/src/gpu.rs:184-189` | `SPEC-GATE-D-01`（绿色 clear ≠ 加载了模型） |

### 关于测试命名的一处警告

`examples/desktop_poc/src/lib/style.rs:216` 的 `hover_only_supersedes_the_matching_property` 断言的是：

- 同一 `background` 上，hover 变体覆盖 base 变体（`bg-blue-600` → `hover:bg-blue-800`）；
- 非命中变体不产生声明；
- 与 inline 与 class 之间的 origin 排序**无关**。

因此**行为上与 FINAL §6 不冲突**，但**测试名字沿用了已被撤销的"condition 压过 origin"措辞**。风险很具体：后续有人拿这个测试名当"hover 胜过 ordinary inline 已通过"的证据。应重命名为类似 `active_hover_variant_overrides_only_its_own_property`（属实现负责人改动，不在我的文件范围）。这条同时说明：`style.rs` 中**根本没有** inline-vs-class 的 fixture，`SPEC-GATE-B-06` 目前零覆盖。

## 8. 本轮未覆盖（不得从中推出任何结论）

| 领域 | 状态 | 原因 |
|---|---|---|
| 真实 winit 窗口 / surface / resize / DPI | OPEN | 仓库无可运行应用的窗口路径 |
| Windows / Linux 实机（含 X11、Wayland 分列） | OPEN | 本轮主机是 macOS；跨平台不得复制结果 |
| 真实屏幕阅读器（NVDA / VoiceOver / AT-SPI） | OPEN | 无 runner，无语义树输出 |
| 中文 IME / caret / selection 坐标换算 | OPEN | 无输入设备与编辑器实现 |
| device-loss 恢复、atlas 重建 | OPEN | 未构造该故障 |
| mask / filter / 分层 clip | OPEN | 探针未涉及；官方亦提示会 panic 的风险 |
| baseline / intrinsic / 字体合成 | OPEN | §7 SRC-01…03 显示仍有缺陷 |
| glTF / 独立 depth / 3D | OPEN | 只有占位纹理与绿色 clear |
| 性能（任何 p50/p95/p99、帧预算、idle 能耗） | OPEN | 本轮未采任何计时数据；**禁止填 0** |
| `cargo clippy --workspace` 与 `cargo package` 的 CI 日志 | OPEN | 两者都是 `SPEC-GATE-C-04` 的判据；本机 `clippy --all-targets -- -D warnings` 通过，但仓库没有对应的 CI 日志，因此 C-04 仍 `OPEN` |

## 9. 数字台账（仅汇总本文件已定义的事实，供 CLAIM 引用）

- H2 实际通过测试：**53** 条（7 + 2 + 4 + 40），来源 EVID-VAL-12/13/14/21；其中 40 条来自新的 headless workspace。这是 commit `83b21a4` 的计数。
- H2 在 P2 之后的通过**执行数**：**132**（53 加上 EVID-VAL-23 的 6、EVID-VAL-24 的 65 与 EVID-VAL-27 的 8）。它是同一批用例在新 commit 上的复跑，**不是** 107 条互不相同的测试；`EVID-VAL-21` 的 40 与 `EVID-VAL-24` 的 50 有重叠（core lib 由 34 增至 44），不得相加当作"新增覆盖"。
- H2 compile-fail fixture：**9** 条 = EVID-VAL-13 的 3 条 trybuild + EVID-VAL-24 的 6 条 `compile_fail` doctest。
- 0-test target：根 package，来源 EVID-VAL-17。
- 跨图形后端 GPU 观测：**2** 个数据点（Vulkan/NVIDIA 来自 H1 转述，Metal/Apple M4 来自 EVID-VAL-15），**不是** "GPU 矩阵已通过"。
- CI 实际通过测试：**120** 次执行 = 40 条 × 3 个 runner（`EVID-VAL-22`）。它是**执行次数**，不是 120 条不同的测试，也不得与 H2 的 53 相加；同一批 40 条测试在 H2 与 CI 上各跑过。
- 可复跑性分布：27 条中 **15 条**（A 类）可在本机原样复跑；**11 条**（B/C 类）依赖上一轮记录或仓库外夹具；**1 条**（D 类，`EVID-VAL-22`）在 CI runner 上，重跑 workflow 可复现，本机不能代替。

不得使用"基本完成""大部分支持""应该可以"描述以上任何数字。

## 11. P2 轮（MORT-5）在 H2 的复跑

本节四条记录的是 `feat/MORT-5-transaction-journal` 上的运行，被测代码 commit
bb16522b03abd7fb0102fb092c49284615416fbc。与 §6 一样，**一条记录只对应一条命令**；分支与 commit 相同不构成合并记录的许可。

### EVID-VAL-23 A0 gate 夹具（H2）

- 分支：`feat/MORT-5-transaction-journal`；被测代码 commit bb16522b03abd7fb0102fb092c49284615416fbc。
- 主机：H2（macOS 27.0.1 / Apple M4）；`rustc 1.99.1 (58cf8f9c3 2026-10-13) aarch64-apple-darwin`。

```text
$ cargo test -p zenui-core --test gates --locked
test a0_03_cross_runtime_access_is_rejected ... ok
test a0_03_stale_handle_does_not_alias_the_reused_slot ... ok
test a0_04_nested_batch_publishes_once_with_the_outer_value ... ok
test a0_06_rejected_commit_restores_the_previous_commit ... ok
test a0_03_a_failed_read_does_not_advance_io_reads ... ok
test a0_06_a_duplicate_key_is_a_value_error_that_keeps_the_frame_alive ... ok
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

每条夹具比对的权威快照只含**会回滚的状态**（`titles`、`order`、`epoch`、`remaining`、
`has_completed`、`visible`、`focused`、`caret`、`geometry_epoch`、`live_owners`、`layout_nodes`）；
单调计数器被刻意排除，因为 `SPEC-RT-053` 允许它们在被丢弃的帧上推进。

**判红复核（去掉实现即失败）**。本轮逐条做了最小 mutation，跑完立即还原，工作树干净：

| 夹具 | mutation | 结果 |
|---|---|---|
| A0-06 | `preflight` 中 `removals` 与同 key 暂存写冲突的判定短路为 `false` | `a0_06_rejected_commit_restores_the_previous_commit` **FAILED**，其余 3 条 ok |
| A0-03（stale） | `Slots::free` 不再推进 generation | `a0_03_stale_handle_does_not_alias_the_reused_slot` **FAILED**，其余 3 条 ok |
| A0-04 | `Tx::batch` 在内层正常返回时也压入一条 `Publication` | `a0_04_nested_batch_publishes_once_with_the_outer_value` **FAILED**，其余 3 条 ok |
| A0-05 | `Tx::require` 的阶段守卫恒为真（`true \|\| permits`） | `a_write_during_compute_is_a_domain_error` **FAILED**（该条在 `--lib` 目标，见 EVID-VAL-24） |

- **不证明什么**：
  - A0-03 仍缺 gate 级的**跨 runtime 读**夹具（本条只覆盖写侧拒绝与 stale 复用）与 `A0-02` 的 trybuild fixture。
  - A0-04 的夹具覆盖「一次交互只发布一次、发布值是外层值」，**不覆盖**预检失败时 `LayoutMap` 与 `ViewTree` 的部分安装；那条路径由 `LayoutMap::discard` 与 `Txn::discard` 的结构保证，没有独立夹具。
  - A0-05 只证明「阶段守卫会拒绝写」与「下一帧可恢复」，**不证明** memo 图存在环检测。
  - A0-06 的 `LayoutMap` 是 core 内零依赖映射，**不是** Taffy 适配器；与真实布局引擎对接的部分仍 OPEN。
  - 四条夹具都不触碰窗口、GPU、平台后端，也不构成任何性能结论。

### EVID-VAL-24 `zenui-core` 全量测试（H2）

- 分支与 commit 同 EVID-VAL-23。命令只跑 `zenui-core` 包，不含 `zenui-app`。

```text
$ cargo test -p zenui-core --locked
     Running unittests src/lib.rs (zenui_core)    45 passed; 0 failed
     Running tests/gates.rs (zenui_core)           6 passed; 0 failed
     Running tests/regressions.rs (zenui_core)     8 passed; 0 failed
   Doc-tests zenui_core                            6 passed; 0 failed
（合计 65 passed / 0 failed）
```

- 45 条单测里与本次 gate 直接相关的：
  - A0-05 的图侧夹具：`a_diamond_never_observes_two_epochs`（同一 epoch 的两个分支在 join 处必须相等）、`a_long_chain_keeps_the_final_value_consistent`（7 级链逐级 `+1`）；事务侧夹具：`runtime::tests::a_write_during_compute_is_a_domain_error`（`Phase::Topo` 下 `insert` 返回 `Fatal::PhaseViolation`，下一帧正常提交）。
  - A0-06 的映射单测：`layout::tests` 3 条（plan 只读可见、`discard` 遗忘计划、mount+unmount 同 view 与缺失 plan 均被 preflight 拒绝）。
  - A0-03 的槽位单测：`arena` 与 `store` 下 4 条（见 EVID-VAL-20 的「不证明什么」）。
- 6 条 doctest 是 `compile_fail` 夹具，锁 `SPEC-RT-002` / `SPEC-GATE-A0-03` 的身份域**双向**不互通：`ViewId`/`RowKey`/`LayoutNodeId` 两两互传都必须 **E0308**。这是 EVID-VAL-13 三个 compile-fail fixture 之外的新增数量，统计见 §9。
- **不证明什么**：`zenui-app` 的 6 条 golden 测试不在本命令内（见 EVID-VAL-25 与 EVID-VAL-21）；单测数量不等于 gate 覆盖数。

### EVID-VAL-25 重构后的 trace 复跑（H2）

- 分支与 commit 同 EVID-VAL-23。命令与 EVID-VAL-20 相同，**记录的目标不同**：这里是「同一 trace 在新实现上是否仍然逐字节相同」。

```text
$ cargo run -p zenui-app --example todolist_headless --locked > /tmp/p2trace.txt
$ shasum -a 256 /tmp/p2trace.txt crates/app/tests/golden/todolist_headless.trace
fb16d571748949225b46a5eb78abc72e5badd13e3e2b6cede7b6b7aefd392f21  /tmp/p2trace.txt
fb16d571748949225b46a5eb78abc72e5badd13e3e2b6cede7b6b7aefd392f21  crates/app/tests/golden/todolist_headless.trace
```

- golden 文件**未重新录制**：这次重构把模型、memo、view、owner、layout 的写入全部搬进一个帧事务，可观测输出一个字节都没动。这是本轮最重要的负面结论：重构没有改变语义。
- **不证明什么**：trace 相同只说明 `SPEC-GATE-A0-01` 的那 9 步没变，**不说明**事务层正确；事务层由 EVID-VAL-23 的夹具与 mutation 复核承担。trace 中也不含任何 staging 计数。

### EVID-VAL-26 clippy 全 workspace（H2）

- 分支与 commit 同 EVID-VAL-23。

```text
$ cargo clippy --workspace --all-targets --locked -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s)
（exit 0）
```

- 同轮 `cargo fmt --all -- --check` 通过。
- **不证明什么**：这**不是** CI 日志。`SPEC-GATE-C-04` 的判据是仓库 CI 上的 `cargo clippy` 与 `cargo package` 记录，两者仍然缺失，因此 **C-04 保持 OPEN**。本机通过只说明本机通过。

## 12. ADR-0017 一致性审计的回归（H2）

被测代码 commit `bb16522b03abd7fb0102fb092c49284615416fbc`。审计由 5 个独立审查视角并发执行（每个只看一组 ADR 决策或 spec 条款），
候选缺陷再经过一轮对抗验证；下面的 8 条是修完后固化的回归夹具。

### EVID-VAL-27 审计回归夹具（H2）

```text
$ cargo test -p zenui-core --test regressions --locked
test a_swallowed_nested_failure_poisons_the_frame ... ok
test a_discarded_frame_does_not_rewind_row_keys ... ok
test a_removal_wins_over_a_staged_value ... ok
test an_insert_removed_in_the_same_frame_leaves_no_row ... ok
test a_partial_view_order_is_rejected ... ok
test a_mount_for_a_removed_row_is_rejected ... ok
test a_focus_on_an_unmounted_view_is_rejected ... ok
test a_discarded_frame_does_not_burn_layout_ids ... ok
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

**审计发现的真实缺陷（修前全部可复现）**

| 夹具 | 修前行为 | 条款 |
|---|---|---|
| `a_swallowed_nested_failure_poisons_the_frame` | 内层 `batch` 返回 `Err` 被外层吞掉后，内层 staged 写仍然提交 | ADR-0017 决定 8 |
| `a_discarded_frame_does_not_rewind_row_keys` | `discard` 后 key 分配器回退，第三次 insert 会铸出重复 `RowKey` | `SPEC-RT-002` |
| `a_removal_wins_over_a_staged_value` | 同帧删除后 `with_title`/`completed`/`read_once` 仍返回旧值 | ADR-0017 决定 4 |
| `an_insert_removed_in_the_same_frame_leaves_no_row` | 同帧 insert+remove 仍然填槽，槽位永久泄漏 | `SPEC-RT-051` |
| `a_partial_view_order_is_rejected` | 局部 view order 在 apply 时被静默过滤，view 失联 | `SPEC-RT-052` |
| `a_mount_for_a_removed_row_is_rejected` | 为已删行挂载的 view 可以提交 | `SPEC-RT-052` |
| `a_focus_on_an_unmounted_view_is_rejected` | focus 指针可与 journal 不一致地提交 | `SPEC-RT-052` |
| `a_discarded_frame_does_not_burn_layout_ids` | 失败帧推进 `LayoutNodeId` 分配器 | `SPEC-RT-051` |

**判红复核**（每条 mutation 跑完立即还原）：

| 夹具 | mutation | 结果 |
|---|---|---|
| `a_discarded_frame_does_not_rewind_row_keys` | `discard` 不再以高水位重种 journal | **FAILED** |
| `a_swallowed_nested_failure_poisons_the_frame` | `batch` 不再设置 poison | **FAILED** |
| `a_partial_view_order_is_rejected` | 删除 preflight 的 view order 置换检查 | **FAILED** |
| `a_removal_wins_over_a_staged_value` | `effective_completed` 忽略暂存删除 | **FAILED** |

- **不证明什么**：
  - **memo cycle 没有检测**。`GraphError` 没有 cycle 变体，环仍表现为 compute 重入并用陈旧值算出结果；A0-05 中「cycle 是领域错误」这一半没有实现，也没有夹具。
  - **Cleanup 阶段本身没有被任何帧进入过**：cleanup 回调现在在换入之后、`Phase::Cleanup` 下执行，但「cleanup 中 unmount 的意图只入 journal」没有独立夹具。
  - 审计指出 `LayoutMap` 原本是公开类型（`pub use layout::LayoutMap`），与 ADR 决定 12 的「core 私有」不一致；本轮已改为 `mod layout;`（不再导出），但**尚无夹具**断言它不可被下游命名。
  - `SPEC-RT-053` 把 duplicate key 列为必须丢弃事务的 preflight 失败，而 ADR-0017 决定 7 把它归为值错误并使帧继续提交。代码按 ADR 执行；这是 spec 与 ADR 的冲突，需要裁决，不由实现者就地解释。
