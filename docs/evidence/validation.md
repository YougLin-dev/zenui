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

两条铁律：

1. **不得跨主机复制结论。** H2 通过只说明在 H2 通过；H1 的 Windows/Vulkan 结论不能由 H2 的 macOS/Metal 结果替代，反之亦然。
2. **不得把 0 tests 的 target 算作完成。** 根 package 的 `zenui` 目前没有任何测试（EVID-VAL-04 / -17），它的 `exit 0` 只证明能构建。

## 1. 主机与溯源

| 主机 | OS | Rust | GPU / adapter | commit | 证据性质 |
|---|---|---|---|---|---|
| **H1** | Windows 11，MSVC 工具链 | `1.99.1 x86_64-pc-windows-msvc` | NVIDIA GeForce RTX 4060 Laptop GPU / Vulkan | 未记录 | 上一轮记录（转述） |
| **H2** | macOS 27.0.1 (26A434)，Apple M4（10 核 GPU，Metal 4） | `1.99.1 (58cf8f9c3 2026-10-13) aarch64-apple-darwin` | Apple M4 / `metal` | `ab5339b4fec807faef3128587721a1599f5aab9b` | **本轮实测** |

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
| 全 workspace 的 `cargo test / clippy / package` | OPEN | 根 package 不是统一 workspace（EVID-VAL-04/-17） |

## 9. 数字台账（仅汇总本文件已定义的事实，供 CLAIM 引用）

- H2 实际通过测试：**13** 条（7 + 2 + 4），来源 EVID-VAL-12/13/14。
- H2 compile-fail fixture：**3** 条，来源 EVID-VAL-13。
- 0-test target：根 package，来源 EVID-VAL-17。
- 跨图形后端 GPU 观测：**2** 个数据点（Vulkan/NVIDIA 来自 H1 转述，Metal/Apple M4 来自 EVID-VAL-15），**不是** "GPU 矩阵已通过"。
- 可复跑性分布：19 条中 **8 条**（A 类）可在本机原样复跑；**11 条**（B/C 类）依赖上一轮记录或仓库外夹具。

不得使用"基本完成""大部分支持""应该可以"描述以上任何数字。
