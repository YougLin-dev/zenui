---
doc-id: CLAIM-REGISTER
layer: evidence
status: draft
date: 2026-11-18
owner: 顶级研究员（疑难攻坚）
sources:
  - archive/research/README.md
  - archive/research/CONCLUSION.md
  - archive/research/native-macro.md
  - archive/research/runtime-and-rendering.md
  - archive/research/stack-decisions.md
  - archive/research/style-system.md
  - archive/research/tailwind-coverage.md
  - archive/research/validation-and-roadmap.md
  - archive/review/FINAL.md
  - archive/review/VALIDATION.md
  - archive/review/README.md
  - archive/review/round1-experts.md
  - archive/review/round2-debate.md
  - archive/review/round3-review.md
  - archive/review/text-layout-review.md
  - archive/review/validation-matrix-raw.json
  - archive/review/verification-contract.json
  - 源码静态核对（见各条 SRC 证据）
replaces: []
---

# CLAIM 台账：全仓唯一事实来源

> 本文件回答一个问题：**哪句话已经被证明，哪句话只是目标，哪句话已经被推翻。**
> 一条断言只在这里定义一次；`spec/`、`product/`、`decisions/` 需要引用事实时引用 `CLAIM-xxx`，不得复述。
>
> 本表编写方式：逐份通读 `archive/research/*`、`archive/review/*`（含两份 JSON），并把每条结论回溯到
> 执行记录（`EVID-VAL-*`）、官方文档（URL）或仓库源码（`SRC path:line`）。**本文件作者没有亲自执行
> cargo 或 GPU**；H2（macOS/Metal）的复跑由 Test/CI 记录在 `evidence/validation.md`。所有 `VERIFIED`
> 条目的主机与范围都写进断言本身，不把 H1 的 Windows/Vulkan 结果说成 H2 的，反之亦然。

## 0. 读法与规则

1. **断言必须可判真假**：一行一句，不写"架构优雅""基本可用"。
2. **状态只用五个词**：`SPEC` / `PROBE` / `VERIFIED` / `WITHDRAWN` / `OPEN`。
   - `SPEC`：目标要求，尚无实现与执行证据。
   - `PROBE`：有小探针/夹具的最小验证，不等于产品完成。
   - `VERIFIED`：有命令与输出记录（或可静态复核的源码事实），范围随断言限定。
   - `WITHDRAWN`：曾提出、已被裁决推翻；**不删除**，备注写推翻依据。
   - `OPEN`：未决，需要结论。
3. **证据优先级**：`EVID-VAL-*`（执行记录）＞ `EVID-PROBE-*`（探针边界）＞ `SRC`（源码/清单静态事实，`path:line`）＞ 文档自述。
   文档自述只有在无法入库或已明确标注为历史时才使用，并写 `PROBE` + 备注。
4. **一条断言只描述一个可观察事实**。"A 通过 7 个测试"与"A 证明生产运行时可用"是两条，不能合并。

### 0.1 证据键（权威定义见 `docs/evidence/validation.md` §2）

`EVID-VAL-*` 编号已与该文件的 47 条索引对齐。主机定义：

- **H1**：Windows 11 / MSVC / Rust 1.99.1 / NVIDIA RTX 4060 Laptop / Vulkan；上一轮记录，**commit 未记录**。
- **H2**：macOS 27.0.1 / Apple M4 / Metal 4 / Rust 1.99.1；commit `ab5339b`（EVID-VAL-12…19）/ `83b21a4`（EVID-VAL-20/-21）/ `e53ae0e`、`f7a2d62`、`7d78cdc`（EVID-VAL-23…40）、`a24faed`（EVID-VAL-41…47）；本轮 Test/CI 实测。

| 键 | 类别 | 命令 / 位置 | 记录结果 |
|---|---|---|---|
| EVID-VAL-01 | B | `desktop_poc --lib`（H1） | 7 passed |
| EVID-VAL-02 | B | `macro_poc --workspace`（H1） | 2 integration + 1 trybuild runner（3 compile-fail） |
| EVID-VAL-03 | B | `native_poc --lib`（H1） | 4 passed |
| EVID-VAL-04 | B | 根 `cargo test`（H1） | 0 tests |
| EVID-VAL-05 | B | `native_poc` 运行（H1） | PASS；NVIDIA/Vulkan；`[128,0,0,128]` |
| EVID-VAL-06 | B | `desktop_poc` 运行（H1） | PASS；Classic underlay/overlay |
| EVID-VAL-07 | C | 临时 `type_contract`（H1，仓库外） | PASS（仅 shape，不可复跑） |
| EVID-VAL-08 | C | 临时 rstml parse + 嵌套 batch（H1，仓库外） | parse PASS；batch **RED** |
| EVID-VAL-09 | C | 临时 `final-contract`（H1，仓库外） | 首次编译失败 → 修正后 PASS |
| EVID-VAL-10 | C | 临时 `transaction_contract`（H1，仓库外） | 首次失败 → 修正后 PASS |
| EVID-VAL-11 | C | 临时 `copy-probe`（H1，仓库外） | E0204 |
| EVID-VAL-12 | A | `desktop_poc --lib`（H2） | 7 passed |
| EVID-VAL-13 | A | `macro_poc --workspace`（H2） | 2 + 3 compile-fail |
| EVID-VAL-14 | A | `native_poc --lib`（H2） | 4 passed |
| EVID-VAL-15 | A | `native_poc` 运行（H2） | PASS；Apple M4/Metal；`[128,0,0,128]` |
| EVID-VAL-16 | A | `desktop_poc` 运行（H2） | PASS；Metal |
| EVID-VAL-17 | A | 根 `cargo test`（H2） | 0 tests |
| EVID-VAL-18 | A | `#[derive(Copy)] struct { String }`（H2） | E0204 |
| EVID-VAL-19 | A | `todolist/main.rs` 单独编译（H2） | 40 errors（预期不可编译） |
| EVID-VAL-20 | A | `cargo run -p zenui-app --example todolist_headless --locked`（H2，commit `83b21a4`） | 9 步 trace；两次运行逐字节一致 |
| EVID-VAL-21 | A | `cargo test --workspace --locked`（H2，commit `83b21a4`） | 40 passed（34 core + 6 golden） |
| EVID-VAL-22 | D | `cargo test --workspace --locked`（CI 矩阵 ubuntu/macos/windows，commit `213f772`） | 40 passed × 3 平台 |
| EVID-VAL-23 | A | `cargo test -p zenui-core --test gates --locked`（H2，commit `%COMMIT%`） | 4 passed（A0-03/04/06 夹具，含 mutation 判红复核） |
| EVID-VAL-24 | A | `cargo test -p zenui-core --locked`（H2，commit `%COMMIT%`） | 50 passed（44 lib + 4 gates + 2 doctest） |
| EVID-VAL-25 | A | `cargo run -p zenui-app --example todolist_headless --locked`（H2，commit `%COMMIT%`） | 9 步 trace，sha256 与 EVID-VAL-20 相同 |
| EVID-VAL-26 | A | `cargo clippy --workspace --all-targets --locked -- -D warnings`（H2，commit `%COMMIT%`） | exit 0（本机，不是 CI 日志） |
| EVID-VAL-27 | A | `cargo test -p zenui-core --test regressions --locked`（H2，commit `bb16522b03abd7fb0102fb092c49284615416fbc`） | 8 passed（ADR-0017 审计回归夹具） |
| EVID-VAL-28 | A | `cargo test -p zenui-core --test queries --locked`（H2，commit `f7a2d62`） | 4 passed（A0-07/08 计数夹具） |
| EVID-VAL-29 | A | `cargo test -p zenui-core --test lifecycle --locked`（H2，commit `f7a2d62`） | 7 passed（A0-09/10/11 与 A0-02 运行时夹具） |
| EVID-VAL-30 | A | `cargo test -p zenui-core --locked`（H2，commit `f7a2d62`） | 90 passed（54 + 6 + 7 + 4 + 11 + 8 doctest） |
| EVID-VAL-31 | A | `cargo run -p zenui-app --example todolist_headless --locked`（H2，commit `f7a2d62`） | trace sha256 `4da03691…`（A0-08 语义变更后重录） |
| EVID-VAL-32 | A | `cargo clippy --workspace --all-targets --locked -- -D warnings`（H2，commit `f7a2d62`） | exit 0（本机，不是 CI 日志） |
| EVID-VAL-33 | A | `cargo test -p zenui-core --test queries --locked`（H2，commit `7d78cdc`） | 7 passed（A0-07/08 重写夹具；四条 mutation 判红） |
| EVID-VAL-34 | A | `cargo test -p zenui-core --test lifecycle --locked`（H2，commit `7d78cdc`） | 9 passed（A0-09/10/11、A0-02 与两条取舍夹具；三条 mutation 判红） |
| EVID-VAL-35 | A | `cargo test -p zenui-core --test regressions --locked`（H2，commit `7d78cdc`） | 12 passed（含丢弃帧的缓存回归，`invalidate` 判红） |
| EVID-VAL-36 | A | `cargo test -p zenui-app --locked`（H2，commit `7d78cdc`） | 10 passed（2 单测 + 6 golden + 2 mount path；`read_once` 判红） |
| EVID-VAL-37 | A | `cargo test -p zenui-core --locked`（H2，commit `7d78cdc`） | 99 passed（57 + 6 + 9 + 7 + 12 + 8 doctest；doctest 不校验错误码） |
| EVID-VAL-38 | A | `cargo test --workspace --locked`（H2，commit `7d78cdc`） | 109 passed |
| EVID-VAL-39 | A | `cargo run -p zenui-app --example todolist_headless --locked`（H2，commit `7d78cdc`） | trace sha256 `57a4a142…`（接线后重录） |
| EVID-VAL-40 | A | `cargo clippy --workspace --all-targets --locked -- -D warnings`（H2，commit `7d78cdc`） | exit 0（本机，不是 CI 日志） |
| EVID-VAL-41 | A | `cargo test -p zenui-core --test compile_fail --locked`（H2，commit `a24faed`） | 1 passed（9 条 trybuild UI 用例；两条 mutation 判红） |
| EVID-VAL-42 | A | `cargo test -p zenui-core --test queries --locked`（H2，commit `a24faed`） | 7 passed（ADR-0022 决定 6 的五条 mutation 判红） |
| EVID-VAL-43 | A | `cargo test -p zenui-core --locked`（H2，commit `a24faed`） | 92 passed（doctest 0，8 条已迁移到 trybuild） |
| EVID-VAL-44 | A | `cargo test --workspace --locked`（H2，commit `a24faed`） | 102 passed |
| EVID-VAL-45 | A | `cargo run -p zenui-app --example todolist_headless --locked`（H2，commit `a24faed`） | trace sha256 与 `57a4a142…` 相同 |
| EVID-VAL-46 | A | `cargo clippy --workspace --all-targets --locked -- -D warnings`（H2，commit `a24faed`） | exit 0（本机，不是 CI 日志） |
| EVID-VAL-47 | D | `cargo test --workspace --locked`（CI 矩阵 ubuntu/macos/windows，commit `61de7b5`） | 102 passed × 3 平台；trybuild 快照三平台逐字匹配 |
| SRC | — | 仓库源码/清单，`path:line` | 本轮静态核对，可直接复查 |

> **已裁决（C-18/D-02，ADR-0012）**：`docs/README.md` §6 现已承认第三种证据前缀 `SRC path:line`，
> 优先级为 `EVID-VAL-*` ＞ `EVID-PROBE-*` ＞ `SRC`。`SRC` 只证明"代码此刻是这样"，不能证明运行行为；
> 探针的能力边界另见 `evidence/probes.md`（`EVID-PROBE-01…05`）。本表原有 `CODE` 前缀已全部改为 `SRC`。

---

## 1. CLAIM 台账

### 1.1 执行范围、工具链与仓库形态（CLAIM-001 – CLAIM-012）

| ID | 断言 | 状态 | 证据 | 关联 Gate | 备注 |
|---|---|---|---|---|---|
| CLAIM-001 | 真实 rstml 0.13.1 能解析目标 node/component/attribute 形状，但仅验证到 parse-only | PROBE | EVID-VAL-08；EVID-VAL-02/13 | A0 | 与 `archive/research/native-macro.md` 的目标形状一致；不证明 lowering |
| CLAIM-002 | 根 package 在 H1 与 H2 的记录中 `cargo test` 均为 0 tests passed | VERIFIED | EVID-VAL-04/17 | — | 只证明 hello-world 可构建，不是能力证据 |
| CLAIM-003 | 仓库现有 53 个实际通过的库/集成测试（7 + 2 + 4 + 40） | VERIFIED | EVID-VAL-01/02/03（H1）与 12/13/14/21（H2） | A0/B | 13 条 PoC 测试两主机同名通过；40 条 headless 测试**只在 H2 采集**，H1 无对应运行 |
| CLAIM-004 | macro workspace 另有 1 个 trybuild runner，内含 3 个 compile-fail fixture | VERIFIED | EVID-VAL-02/13 | A1 | runner 本身不计入 53 条能力测试 |
| CLAIM-005 | H1 记录中的 13 + 3 执行主机是 Windows 11 + NVIDIA RTX 4060 Laptop + Vulkan | VERIFIED | EVID-VAL-01..06 的记录头 | — | H2 另有 macOS/Metal 复跑，见 CLAIM-003/006 |
| CLAIM-006 | 本台账作者本轮只做静态核对，未亲自执行 cargo；H2 的测试与 GPU 复跑由 Test/CI 记录（`EVID-VAL-12..20`） | VERIFIED | 本会话 `uname -m`/`sw_vers`；`evidence/validation.md` §1、§6 | — | `native_poc` 输出中的 `SCOPE: ... Windows ...` 是硬编码字符串，主机以 validation.md §1 为准 |
| CLAIM-007 | 根 `Cargo.toml` 无 dependencies，`src/main.rs` 仅打印 hello-world | VERIFIED | SRC `Cargo.toml:1-12`、`src/main.rs:1-3` | — | 根 package 是 workspace 成员，`publish = false`；生产 runtime 不在此包 |
| CLAIM-008 | 所有已记录的渲染执行都是 headless 离屏 RGBA readback | VERIFIED | EVID-VAL-05/06、15/16；SRC `examples/native_poc/src/gpu.rs`、`examples/desktop_poc/src/main.rs` | C | 无真实 surface/present |
| CLAIM-009 | 仓库源码中不存在 winit、AccessKit、WindowEvent 或 Surface 的使用 | VERIFIED | SRC 全仓 grep（排除 `target/`） | A1/C | 窗口路径完全缺位 |
| CLAIM-010 | 根 manifest 设置 `autoexamples = false`，设计稿不会被当作二进制构建 | VERIFIED | SRC `Cargo.toml:5` | — | 防止"0-test 绿"掩盖产品错误 |
| CLAIM-011 | 目标命令 `cargo run --bin todolist --features windows-desktop --locked` 当前不存在可执行的 bin/feature | VERIFIED | SRC（无 `[[bin]]`、无 features）；EVID-VAL-19 | A1 | 规划命令 |
| CLAIM-012 | MSRV 1.89 只是理论下限，尚未验证 | OPEN | `archive/research/README.md:32` | C | 本机 rustc 1.99.1 不等于 MSRV |

### 1.2 宏前端与 MountPlan（CLAIM-013 – CLAIM-028）

| ID | 断言 | 状态 | 证据 | 关联 Gate | 备注 |
|---|---|---|---|---|---|
| CLAIM-013 | 探针宏只接受 `view` / `text` / `button` 三种 tag | VERIFIED | SRC `examples/macro_poc/macro/src/lib.rs:46-50` | A1 | 其他 tag 编译错误 |
| CLAIM-014 | 探针宏只接受 `class` 与 `on:click/change/input` 属性 | VERIFIED | SRC `macro/src/lib.rs:60-84` | A1 | 属性面远小于目标语法 |
| CLAIM-015 | `aria:`、`style:`、`role`、`disabled` 等属性在探针宏中直接编译错误 | VERIFIED | SRC `macro/src/lib.rs:77-84` | A1 | 目标 Todo 示例大量使用这些属性 |
| CLAIM-016 | 生成代码硬编码绝对路径 `::zenui_macro_runtime`，renamed dependency 未解决 | VERIFIED | SRC `macro/src/lib.rs:112`、`frontend/src/lib.rs:3`；`macro_poc/README.md:27` | A1 | 独立 rename fixture 尚不存在 |
| CLAIM-017 | 未知 class 的诊断当前覆盖整个 class 字面量并给 byte offset，而不是子 token span | VERIFIED | SRC `macro/src/lib.rs:204-213`；`compile_fail/unknown_class.stderr` | A1 | 与目标"精确 span"有差距 |
| CLAIM-018 | 3 个 compile-fail case 分别是 `html_element`、`script_event`、`unknown_class` | VERIFIED | EVID-VAL-02/13 | A1 | 不覆盖缺 key / 事件 payload |
| CLAIM-019 | 探针 `Node` 是可独立 `evaluate_content()` / `fire()` 的 inspection record，不是生产 MountPlan | VERIFIED | SRC `runtime/src/lib.rs:98-160` | A1 | 改名不能变成生产权威 |
| CLAIM-020 | 探针中显式 `move` 闭包绑定在 mount 时不执行，只在显式求值时读取一次 | VERIFIED | EVID-VAL-02/13；`frontend/tests.rs:33-36` | A1 | 只有探针范围 |
| CLAIM-021 | 探针中 `on:*` handler 在 mount 时不执行 | VERIFIED | EVID-VAL-02/13；`frontend/tests.rs:29-44` | A1 | 事件仅在 dispatch 时跑 |
| CLAIM-022 | 探针事件类型是 `Vec<(&'static str, Box<dyn FnMut()>)>`，不承载 typed payload | VERIFIED | SRC `runtime/src/lib.rs:103-110,138-147` | A1 | TextInput/String/SubmitEvent 均未实现 |
| CLAIM-023 | 同名事件存在多个 handler 时，`fire` 只执行第一个匹配项 | VERIFIED | SRC `runtime/src/lib.rs:142-147` | A1 | 与 typed registry 目标冲突 |
| CLAIM-024 | PascalCase 组件、props、slot、For/Show/Index 在探针宏中均未实现 | VERIFIED | SRC `macro/src/lib.rs:46`；`archive/research/validation-and-roadmap.md:10` | A1 | 目标语法不可用 |
| CLAIM-025 | 普通 Rust `Result<MountPlan, MountError>` 函数体可写 `?`，且 Copy handle 可被 move closure 持有 | PROBE | EVID-VAL-07 | A0 | C 类、仓库外夹具；validation.md 将条目本身标 OPEN，不能升级为产品证据 |
| CLAIM-026 | 该夹具不证明 `#[component]` 宏降级、Plan 构建、错误路径或 runtime commit | VERIFIED | EVID-VAL-07；`evidence/validation.md` §5 | A1 | 仅为类型形状 |
| CLAIM-027 | `zenui-final-contract` 首次复跑编译失败：`WriteError` 缺 `Debug`（E0277）、`dyn Command` 按值调用（E0161） | PROBE | EVID-VAL-09；`evidence/validation.md` §5 | A0 | C 类记录，不称"先前通过" |
| CLAIM-028 | 最小修正后 deferred-command 夹具 exit 0，但只证明语法形状，不证明 staging/rollback | PROBE | EVID-VAL-09/-10；`evidence/validation.md` §5 | A0 | `finish()` 只跑 scratch state |

### 1.3 响应式 PoC 与事务（CLAIM-029 – CLAIM-040）

| ID | 断言 | 状态 | 证据 | 关联 Gate | 备注 |
|---|---|---|---|---|---|
| CLAIM-029 | `desktop_poc` lib 有 7 个通过测试（4 reactive + 3 typed-style） | VERIFIED | EVID-VAL-01/12 | A0/B | 见 `evidence/validation.md` §3/§6 |
| CLAIM-030 | 现 `Signal::with` 在 owner dispose 后仍返回旧值 | VERIFIED | SRC `project_signals.rs:26-45`（最后无条件 `get(&value)`） | A0 | 失效只影响订阅登记 |
| CLAIM-031 | 现 `Signal::set` 在失效时静默 `return`，调用方得到 `()` | VERIFIED | SRC `project_signals.rs:47-50` | A0 | 无 `InvalidHandle` |
| CLAIM-032 | 现依赖图只有平坦 source→sink 边，没有 Memo、拓扑稳定、Store 或子 Owner | VERIFIED | SRC `project_signals.rs:59-186`；`evidence/validation.md` §3/§6 | A0 | 与生产目标差一整层 |
| CLAIM-033 | 现 `flush` 没有 evaluation budget 或重入 phase，sink 在计算期自写可再次入队 | VERIFIED | SRC `project_signals.rs:128-162`；`round1-experts.md:242-245` | A0 | 无 `WriteDuringCompute` |
| CLAIM-034 | 现 `batch` 没有嵌套深度，内层调用立即 flush | VERIFIED | SRC `project_signals.rs:164-170` | A0 | 事务原子性未成立 |
| CLAIM-035 | 嵌套 batch 实际泄露中间值 `true`；现 PoC 的 batch 无嵌套深度 | VERIFIED | EVID-VAL-08；SRC `project_signals.rs:164-170` | A0 | 记录与源码互证 |
| CLAIM-036 | `detach` 只从 observer 集合删除，不移除变空的 source bucket | VERIFIED | SRC `project_signals.rs:75-84` | A0 | 长命 Owner 会积累幽灵桶 |
| CLAIM-037 | `subscriber_count()` 返回 `subscribers.len()`，会把空桶计入订阅数 | VERIFIED | SRC `project_signals.rs:183-185` | A0 | 指标会掩盖泄漏 |
| CLAIM-038 | `dispose` 只翻转每源一个 `bool` liveness，不存在 ViewId/OwnerId generation | VERIFIED | SRC `project_signals.rs:86-120,171-180` | A0 | 无法拒绝迟到命令 |
| CLAIM-039 | 生产运行时中 `MountPlan` 与平台后端尚未实现（原句里的「事务 journal」已由 P2 落地，见 CLAIM-101） | VERIFIED | SRC `crates/core/src/`（无 `MountPlan`、无平台后端模块）；`FINAL.md:305-309` | A0 | 只保留仍然成立的两项；journal 一项由 CLAIM-101 覆盖 |
| CLAIM-040 | A0 的 headless trace 与 query/mount/materialization 计数已存在，可用 `cargo run -p zenui-app --example todolist_headless --locked` 复跑 | VERIFIED | EVID-VAL-20 | A0 | 只覆盖 A0-01；A0-02…11 仍 OPEN |

### 1.4 Store / Query / For / 焦点生命周期（CLAIM-041 – CLAIM-053）

| ID | 断言 | 状态 | 证据 | 关联 Gate | 备注 |
|---|---|---|---|---|---|
| CLAIM-041 | 生产 Store/查询层（字段反向边、成员索引、QueryMemo、字段 handle）未实现；`crates/core` 只有 A0-01 骨架 | VERIFIED | SRC `crates/core/src/store.rs`、`crates/core/src/query.rs`（骨架）；`archive/review/FINAL.md:305-307` | A0 | 骨架不覆盖 SPEC-RT-041…047 |
| CLAIM-042 | 现有 PoC 没有字段反向边、成员索引或查询缓存 | VERIFIED | SRC `project_signals.rs`（仅 source→sink） | A0 | 无法支撑 Active/Done |
| CLAIM-043 | Active/Done 的 `completed` 字段变化必须使对应 filter query cache 标脏，即使该行当前不是 member | SPEC | `FINAL.md:79-80` | A0 | 候选索引必须覆盖可能进入 filter 的行 |
| CLAIM-044 | All 的 membership revision 不因单项 `completed` 改变 | SPEC | `FINAL.md:80,265` | A0 | 与 Active 行为不同 |
| CLAIM-045 | 每个 `(store_epoch, filter)` 的 RowSet 由一个 `QueryCache` 共享，第二个消费者不重新物化 | PROBE | EVID-VAL-21（`second_consumer_shares_the_snapshot`）；`FINAL.md:82` | A0 | 只验证共享与按 epoch 失效；拓扑稳定与反向边未实现 |
| CLAIM-046 | 空态求值必须 `row visits = 0`、RowSet 物化 = 0、String clones = 0 | SPEC | `FINAL.md:265` | A0 | 由独立 counter/membership 索引派生 |
| CLAIM-047 | 用 collect/遍历实现 `is_empty`，即使返回正确 bool 也必须判红 | SPEC | `round2-debate.md:188,194`；`FINAL.md:83` | A0 | 防"用名称替代成本" |
| CLAIM-048 | title 是每个 UI 行 mount 时读取一次的 owned `String`，不进入 Copy row | SPEC | `FINAL.md:85`；`examples/todolist/README.md:3` | A1 | 返场是新 mount，可再读一次 |
| CLAIM-049 | For reorder 必须保留 ViewId、generation、Owner、订阅与局部 caret/focus | SPEC | `FINAL.md:58` | A0/A1 | 与 filtered-out 语义相反 |
| CLAIM-050 | key 消失或 filtered out 默认 dispose UI owner/订阅，但保留 Store row model | SPEC | `FINAL.md:59-60` | A0 | 再次出现是新 UI generation |
| CLAIM-051 | 结构 mutation 前必须捕获 `FocusRemovalIntent`，commit 后按 next→prev→可聚焦 input→toolbar 继任 | SPEC | `FINAL.md:57` | A0/A1 | 用户中途换焦点则取消旧 intent |
| CLAIM-052 | 重复 key 必须在任何 native patch 前被拒绝 | SPEC | `FINAL.md:56` | A0 | 保留之前合法 View 状态 |
| CLAIM-053 | 上述 Store/Query/For 的生产语义没有完整实现或 gate 级测试记录；A0 只有骨架与局部单测 | VERIFIED | EVID-VAL-21；SRC `crates/core/src/lib.rs` | A0/A1 | 具体覆盖与缺口见 EVID-VAL-20「不证明什么」 |

### 1.5 布局与文字（CLAIM-054 – CLAIM-063）

| ID | 断言 | 状态 | 证据 | 关联 Gate | 备注 |
|---|---|---|---|---|---|
| CLAIM-054 | `native_poc` lib 有 4 个通过测试 | VERIFIED | EVID-VAL-03/14 | B/A | 见 `evidence/validation.md` §3/§6 |
| CLAIM-055 | 该组覆盖 Parley CJK shaping + CPU 像素、ICU 词边界、Taffy 测量/失效、premultiplied alpha | VERIFIED | EVID-VAL-03/14；SRC `native_poc/src/lib.rs:20-72` | B | 固定测试文本；两主机 |
| CLAIM-056 | `Paragraph::measure()` 内部计算了 `first_baseline()` | VERIFIED | SRC `native_poc/src/layout.rs:18-30` | B | — |
| CLAIM-057 | `compute_leaf_layout` 的 measure 回调只返回 `.size`，baselines 被丢弃 | VERIFIED | SRC `layout.rs:95-107`（`paragraph.measure(known.width).size`） | B | 与 FINAL/E2 描述一致 |
| CLAIM-058 | 叶子测量只使用 `known.width`，没有区分 available space 与 min/max-content | VERIFIED | SRC `layout.rs:104` | B | 不能证明 intrinsic |
| CLAIM-059 | Parley 0.13.0 官方文档声明 `calculate_content_widths()` 对 mixed-direction text 可能返回错误结果 | VERIFIED | URL: https://docs.rs/parley/0.13.0/parley/struct.Layout.html | B | 必须留 canary/fallback |
| CLAIM-060 | glyph 绘制未读取或转发 Parley `run.synthesis()`，不存在 faux bold/oblique | VERIFIED | SRC `native_poc/src/text.rs:34-63`（全文件无 synthesis） | B | cache key 也未含合成参数 |
| CLAIM-061 | 缺失中文/日文 ICU segmentation model 曾导致运行期错误；启用 `complex-scripts` 后词边界断言通过 | VERIFIED | EVID-VAL-03/14；`research/validation-and-roadmap.md:34` | B | 发布 artifact 必须带词典 |
| CLAIM-062 | 仓库没有锁定字体/ICU artifact 清单或 hash | VERIFIED | SRC 目录无 assets/fonts 清单 | B/C | 发布门要求齐备 |
| CLAIM-063 | emoji、混合 bidi、font fallback 的真实渲染尚未验证 | OPEN | `evidence/validation.md` §8 | B | 只有 CJK 小样 |

### 1.6 渲染、GPU 与 3D（CLAIM-064 – CLAIM-078）

| ID | 断言 | 状态 | 证据 | 关联 Gate | 备注 |
|---|---|---|---|---|---|
| CLAIM-064 | `native_poc` 实跑 vello_gpu 0.4 + vello_cpu 0.4 + parley 0.13 + glifo 0.5 + wgpu 30.0.2 的 headless 图 | VERIFIED | EVID-VAL-05/15 | B/A | H1 Vulkan 与 H2 Metal 两个数据点 |
| CLAIM-065 | 该实跑的半透明红 GPU 像素 `[128,0,0,128]` 与 CPU reference 相等 | VERIFIED | EVID-VAL-05/15 | B/A | premultiplied；两主机同值 |
| CLAIM-066 | `desktop_poc` 实跑 Classic vello 0.12 underlay + foreign wgpu `LoadOp::Load` + straight-alpha overlay | VERIFIED | EVID-VAL-06/16 | D | 仅连通性 |
| CLAIM-067 | Classic 线使用 straight-alpha source-over，sparse-strips 使用 premultiplied，两者不可共享默认 | VERIFIED | SRC `desktop_poc/src/compositor.wgsl:11-19` vs `native_poc/src/gpu.rs`；`evidence/validation.md` §4 | B/D | 换后端会错色 |
| CLAIM-068 | 两个 GPU 探针都把目标格式硬编码为 `Rgba8Unorm` | VERIFIED | SRC `gpu.rs:47,170`；`desktop_poc/src/main.rs:6` | B/C | 无 BGRA/sRGB 路径 |
| CLAIM-069 | 两个探针都用 `request_adapter(Default)` / `request_device(Default)`，无 features/limits/surface capability 检查 | VERIFIED | SRC `gpu.rs:17-28`；`desktop_poc/src/main.rs:189-198` | B/C | 不支持只会 panic |
| CLAIM-070 | 外部纹理当前是固定绿色 `Clear(0,1,0,1)` 的占位 | VERIFIED | SRC `gpu.rs:176-198` | D | 不是模型/视频内容 |
| CLAIM-071 | 该探针的 depth 是 renderer 内部 depth view，不是独立 3D depth | VERIFIED | SRC `gpu.rs:52-55` | D | 3D 需独立 Depth32Float |
| CLAIM-072 | 仓库没有 glTF 依赖、vertex/index buffer、camera、uniform 或 PBR 管线 | VERIFIED | SRC manifests + `gpu.rs`；`archive/research/CONCLUSION.md:75` | D | 绿色占位≠加载模型 |
| CLAIM-073 | GPU readback 成功不证明 retained paint cache、窗口 present 或跨平台正确性 | VERIFIED | `evidence/validation.md` §4/§8 | B/C | 只证明当次输出 |
| CLAIM-074 | `RenderCapabilities` / 能力矩阵尚未实现 | OPEN | `FINAL.md:220` | B/C | 需按 adapter 实际枚举 |
| CLAIM-075 | 真实 winit surface、resize、DPI、device loss 尚未验证 | OPEN | `evidence/validation.md` §8 | A1/C | 无窗口代码 |
| CLAIM-076 | macOS/Metal 的 headless 探针已验证，但 macOS 真实窗口与 Linux Wayland/X11 的渲染仍未验证 | OPEN | EVID-VAL-15/16；`evidence/validation.md` §8 | C | 不能把 headless Metal 外推为窗口/跨平台 |
| CLAIM-077 | vello_gpu 的 mask/filter/composite 能力尚未验证 | OPEN | `FINAL.md:198,220` | B | 不得以 panic 探测 |
| CLAIM-078 | 3D 独立 offscreen color+Depth32Float、TextureView metadata、owner generation 尚未实现 | OPEN | `FINAL.md:226-239` | D | 目标合同 |

### 1.7 平台、窗口与无障碍（CLAIM-079 – CLAIM-082）

| ID | 断言 | 状态 | 证据 | 关联 Gate | 备注 |
|---|---|---|---|---|---|
| CLAIM-079 | 中文 IME composition / 候选 Enter 行为未在真实 IME 上验证 | OPEN | `evidence/validation.md` §8；`FINAL.md:276` | A1/C | reducer 也未实现 |
| CLAIM-080 | AccessKit 语义树、NVDA、VoiceOver、AT-SPI 均未验证 | OPEN | `evidence/validation.md` §8 | C | ARIA 字符串不算证据 |
| CLAIM-081 | DPI、resize、multiwindow、clipboard、drag、nested scroll 未验证 | OPEN | `evidence/validation.md` §8；`FINAL.md:290-294` | C | 三平台均缺 runner |
| CLAIM-082 | winit 0.30.13 / accesskit 0.25.2 / accesskit_winit 0.34.2 的组合未在本仓库编译过 | OPEN | 仅文档 pin；所有 manifest 无这些依赖 | A1/C | 版本组合是待验证基线 |

### 1.8 样式与 Tailwind（CLAIM-083 – CLAIM-088）

| ID | 断言 | 状态 | 证据 | 关联 Gate | 备注 |
|---|---|---|---|---|---|
| CLAIM-083 | 宏探针把 `w-72` 映射为 288 logical points 等少量 literal class 字段 | VERIFIED | EVID-VAL-02/13；`frontend/tests.rs:27-34` | B | 不是 Tailwind 覆盖率 |
| CLAIM-084 | 当前 typed style 只覆盖少量 property 与 hover 条件 | VERIFIED | EVID-VAL-01/12（3 个 style 测试）；SRC `desktop_poc/src/lib/style.rs:195-253` | B | 无 theme/variant 全表 |
| CLAIM-085 | 固定 commit `ac7fd400…` 的 Tailwind 命名 aria 变体只生成 `[aria-pressed="true"]`，没有简单的 false 变体 | VERIFIED | URL: https://github.com/tailwindlabs/tailwindcss/blob/ac7fd4007178123269cc6c8e03c8d957646331fa/packages/tailwindcss/src/variants.ts | B | 与 `style-system`/研究结论一致 |
| CLAIM-086 | 普通 author inline 胜过同字段普通 class/hover 属于 CSS origin 规则，不是 Tailwind 特有 specificity | VERIFIED | URL: https://www.w3.org/TR/css-cascade-5/#cascade-sort | B | hover 是否命中不改变 origin |
| CLAIM-087 | 固定版本 N/D/R/X manifest 与差分测试尚未生成 | OPEN | `FINAL.md:191`；`tailwind-coverage.md:21,150` | B | 无可报告覆盖率 |
| CLAIM-088 | Rust `tailwind 0.4.0`（default-features=false）在 Windows 1.99.1 上可编译并产出 `p-4`/`bg-blue-500`/theme/arbitrary 的 CSS 输出（研究自述） | PROBE | `validation-and-roadmap.md:14`；`style-system.md:109-118` | B | 未入库、本轮未复跑；不能当官方一致性 |

### 1.9 撤销清单（CLAIM-089 – CLAIM-100）

| ID | 曾提出的断言 | 状态 | 证据/推翻依据 | 关联 Gate | 备注 |
|---|---|---|---|---|---|
| CLAIM-089 | 组件返回 `impl View`，并在每次更新时重建 | WITHDRAWN | `FINAL.md:37-47,135`；`research/README.md:9` | A1 | 现行：`MountResult<MountPlan>`，mount 一次 |
| CLAIM-090 | `class:if-*` / `class:selected` 可作为条件语法 | WITHDRAWN | `FINAL.md:189-190`；`research/README.md:9` | B | 现行：aria 同源、typed inline |
| CLAIM-091 | 活跃 condition 可跨 origin 胜过 ordinary inline（D4 全序） | WITHDRAWN | `FINAL.md:174-186`；W3C cascade；`round2-debate.md:5` | B | 现行：ordinary inline 胜 |
| CLAIM-092 | hover 激活后 class 应胜过 ordinary inline（旧 B golden） | WITHDRAWN | `validation-matrix-raw.json:2`；`verification-contract.json:9`；`round3-review.md:5,71` | B | 旧 RED/golden 已作废 |
| CLAIM-093 | `TodoRow` 同时 `derive(Copy)` 并持有 `title: String` | WITHDRAWN | EVID-VAL-11/18（E0204）；当前 `examples/todolist/main.rs:20-22` | A1 | 现行：id + CompletedField，title owned |
| CLAIM-094 | 通用 `Memo<Vec<T>>::get()` 返回 owned Vec | WITHDRAWN | `FINAL.md:81`；`round2-debate.md:27-34` | A0 | 现行：`get()` 仅 `T: Copy`，其余 `with` |
| CLAIM-095 | 过滤默认使用 retained-visibility / hide-store 保留行 | WITHDRAWN | `FINAL.md:59-60,244`；`round2-debate.md:130-135` | A0/A1 | 现行：默认 dispose，恢复需 opt-in |
| CLAIM-096 | 把 `macro_poc::Node` 改名即可充当生产 MountPlan | WITHDRAWN | `FINAL.md:46`；`evidence/validation.md` §3/§6 | A1 | inspection record 不是 native View |
| CLAIM-097 | 宏已证明组件可用 `?` 做错误 mount | WITHDRAWN | `FINAL.md:119,135`；EVID-VAL-07 仅为普通 Rust 形状 | A1 | 宏 lowering 仍属 A1 |
| CLAIM-098 | title 在 row model 创建时读取 | WITHDRAWN | `examples/todolist/README.md:3`；`FINAL.md:85` | A1 | 现行：每个 UI 行 mount 时读取一次 |
| CLAIM-099 | 生产 facade 可沿用绝对 `::zenui_macro_runtime` 路径 | WITHDRAWN | `FINAL.md:142`；`macro_poc/README.md:27` | A1 | 现行：entry facade / package resolution |
| CLAIM-100 | 可用一个无订阅的 Vec 作为 Store 查询的"第二现实" | WITHDRAWN | `FINAL.md:78`；`round2-debate.md:182-188` | A0 | 现行：共享 query cache + 字段边 |
| CLAIM-101 | `crates/core` 已有单帧事务层：模型、memo、view、owner、layout 写入先暂存，最外层退出时一次发布；回滚等于 drop，`Drop` 不跑用户代码也不 panic；`commit` 在 swap 之后没有失败路径 | VERIFIED | SRC `crates/core/src/txn.rs`；EVID-VAL-23、EVID-VAL-25 | A0 | 取代 CLAIM-039；`MountPlan` 与平台后端仍未实现 |
| CLAIM-102 | A0-03 有 gate 级夹具：跨 runtime 写被拒绝、remount 后旧 handle 不 alias 复用槽位；去掉 generation 推进即判红 | VERIFIED | EVID-VAL-23（含 mutation 表）；SRC `crates/core/tests/gates.rs` | A0 | **不覆盖**跨 runtime 读与 A0-02 的 trybuild fixture |
| CLAIM-103 | A0-05 有夹具：diamond 与 7 级长链在同一 epoch 内不产生 glitch；在 `Topo`/`Binding` 相位下的写返回 `ValueError::Phase(PhaseViolation { phase, .. })` 并锁存致命错误，帧返回 `Fatal::PhaseViolation`、`commits` 不增、下一帧可恢复；自读 memo 环返回 `Fatal::GraphCycle` | VERIFIED | EVID-VAL-24；SRC `crates/core/src/graph.rs`、`crates/core/src/runtime.rs` | A0 | 只覆盖被强制相位下的写路径与自读环，不是「所有写路径」与「所有环形状」 |
| CLAIM-104 | A0-06 的映射子句由 core 内零依赖 `LayoutMap` 承担：`plan` 只对帧内可见，`preflight` 拒绝 mount+unmount 同 view、缺失 plan 与重复 id，`discard` 清空计划 | VERIFIED | EVID-VAL-23、EVID-VAL-24；SRC `crates/core/src/layout.rs` | A0 | **不是** Taffy 适配器；与真实布局引擎对接仍 OPEN |
| CLAIM-105 | 帧事务的失败语义已被回归夹具钉住：嵌套失败中毒、discard 不回退 row key、暂存删除压过暂存值、同帧 insert+remove 归还预留槽位 | VERIFIED | EVID-VAL-27；SRC `crates/core/src/txn.rs`、`crates/core/tests/regressions.rs` | A0 | 不含 memo cycle 检测 |
| CLAIM-106 | preflight 会拒绝局部 view order、指向已删行的 mount 与指向本帧将卸载 view 的 focus，因而换入的 view 树必然与 journal 一致 | VERIFIED | EVID-VAL-27；SRC `crates/core/src/txn.rs` | A0 | 只能在单线程 headless 下验证 |
| CLAIM-107 | 越阶段写返回 typed `ValueError::Phase` 并毒化事务，帧不会提交；debug 构建不再靠 `debug_assert!` 中止 | VERIFIED | EVID-VAL-24；SRC `crates/core/src/runtime.rs` | A0 | 仅在强制相位下测过 |
| CLAIM-108 | `LayoutMap` 是 core 私有类型（`mod layout;`，不导出），只经 `Runtime` 的 preflight 与只读访问器对外可见，与 ADR-0017 决定 12 一致 | VERIFIED | SRC `crates/core/src/lib.rs`、`crates/core/src/layout.rs` | A0 | 无夹具断言下游无法命名该类型 |
| CLAIM-109 | memo 环被检测为领域错误：body 读到仍在计算栈上的节点即 `GraphError::Cycle { node }`，事务锁存 `Fatal::GraphCycle { node }`、丢弃 stage、`commits` 不增，下一帧可正常提交 | VERIFIED | EVID-VAL-24；SRC `crates/core/src/graph.rs`、`crates/core/src/runtime.rs` | A0 | 只覆盖自读环；互读环走同一条 computing 栈 |
| CLAIM-110 | `SPEC-RT-053` 的括号列表被 ADR-0019 解释为检测点示例：写 API 在进入 journal 之前拒绝的 duplicate key 是值错误，preflight 阶段检测到的不一致才丢弃整帧；代码按此执行 | VERIFIED | SRC `crates/core/src/txn.rs`；`docs/decisions/adr-0019-…md` 决定 1、2 | A0 | 由 ADR-0019 裁决，不再是未决项 |
| CLAIM-111 | `ValueError` 的形状是 `Handle / Store / Phase(PhaseViolation) / Aborted`；不存在通用的 `ValueError::Fatal` 通道，`PhaseViolation` 与 commit 侧的 `Fatal::PhaseViolation` 是同一个结构 | VERIFIED | EVID-VAL-24；SRC `crates/core/src/txn.rs` | A0 | ADR-0019 决定 3、5、8 |
| CLAIM-112 | 写 API 阶段违规返回 `ValueError::Phase` 并锁存致命错误；帧体即使返回 `Ok`，最外层也返回锁存的 `Fatal`、`commits` 不增、下一帧可提交 | VERIFIED | EVID-VAL-24（`a_write_during_compute_is_a_domain_error`）；SRC `crates/core/src/runtime.rs` | A0 | 夹具断言帧级结果，不只看调用点 |
| CLAIM-113 | owner cleanup 在换入之后、`Phase::Cleanup` 下运行；cleanup 相位的 view 写返回 typed 错误而不是静默应用，下一帧可正常 staging | VERIFIED | EVID-VAL-24；SRC `crates/core/src/txn.rs` | A0 | cleanup 闭包是 `'static`，类型上无法访问 `Tx`，所以「只入 journal」以「拒绝 + 下一帧可提交」表达 |
| CLAIM-114 | query cache 的键是 `(membership revision[filter], order revision)`：completed 写只推进 Active/Completed，reorder 只推进 order revision，insert 推进 All+Active，remove/clear 推进它们真正改变的集合；标题不参与任何 filter 的成员判定，因此不在键内 | VERIFIED | EVID-VAL-33；SRC `crates/core/src/query.rs`、`crates/core/src/txn.rs`、`crates/core/src/store.rs` | A0 | 反向边收敛到 filter 维度，不记录逐行边；修订号在 staging 点推进、由 `Store::install` 原样采纳，因此帧内构建的快照在 commit 后仍有效 |
| CLAIM-115 | 空 filter 由成员计数回答：`cache_builds`/`id_materializations`/`row_visits` 均为 0；删掉这条短路会让 3 条夹具判红。把 `is_empty_for` 换成数一遍 `rows_for(All)`（布尔不变）会让 6 条夹具判红，含 1000 行空态（`row_visits` 2000 → 3000）——扫描计数的归属在视图侧，缓存按求值前后的差值记 `row_visits` | VERIFIED | EVID-VAL-42；SRC `crates/core/src/query.rs`、`crates/core/src/txn.rs` | A0 | 只覆盖本 crate 的三个 filter；裁决来自 ADR-0022 决定 6。§14（EVID-VAL-33）记的是 ADR 之前的契约，那里同一条变异 7 条全绿 |
| CLAIM-116 | For 强制 key：同一 RowKey 的第二次 mount 在任何 patch 之前返回 `Fatal::DuplicateViewKey`；reorder 不产生 unmount/setup 且保留 ViewId/owner/focus/caret | VERIFIED | EVID-VAL-29；SRC `crates/core/src/txn.rs` | A0 | 只覆盖 row 级 view；嵌套 For 未实现 |
| CLAIM-117 | FocusRemovalIntent 在结构变更前记录、commit 末尾结算一次；用户同帧转移焦点会取消；批量 Clear 只结算一次 | VERIFIED | EVID-VAL-29；SRC `crates/core/src/txn.rs` | A0 | 回退顺序只有 next → previous；input/toolbar 节点在 A0 不存在 |
| CLAIM-118 | 1000+ 交错 mount/unmount/filter churn 后 live owner、edges、active source 与 layout 映射回到基线 | VERIFIED | EVID-VAL-29；SRC `crates/core/tests/lifecycle.rs` | A0 | 确定性脚本，不是随机压力 |
| CLAIM-119 | A0-02 的契约由 trybuild UI 快照与运行时 read 计数共同钉住（E0204 `Copy` 类型不能拥有 `String`；E0599 权威无拥有式 `get`；一次 mount 恰一次 owned 读），A0-03 的六条身份域互斥固定在 E0308 | VERIFIED | EVID-VAL-41、EVID-VAL-36；SRC `crates/core/tests/ui/`、`crates/app/src/lib.rs` | A0 | 原 `compile_fail` doctest 已删除：它只证明"不能编译"，笔误也能保持绿色（ADR-0021、EVID-VAL-41 的 mutation） |

---

## 2. 覆盖统计

| 状态 | 条数 | 主要领域 |
|---|---|---|
| VERIFIED | 80 | 执行记录、源码事实、两条官方 CSS/Tailwind 事实 |
| SPEC | 9 | Store/Query/For/焦点契约中尚未实现的部分 |
| OPEN | 12 | MSRV、bidi/emoji、capabilities、surface/DPI、Mac/Linux、mask/filter、3D depth、IME、AT、平台矩阵、版本组合 |
| WITHDRAWN | 12 | 旧 mount 形状、条件语法、优先级、Copy+String、retained-visibility 等 |
| PROBE | 6 | rstml parse-only、3 条 C 类夹具记录、tailwind crate 研究探针、A0 query 缓存共享 |
| **合计** | **119** | — |

> 注意：`VERIFIED` 只表示"该断言在其限定范围内有记录"，不表示整个子系统完成。例如
> CLAIM-064 只证明 H1/H2 两个 headless 数据点，不证明窗口、surface 或其他 Linux 后端。

---

## 3. 审计：跨文档矛盾与重复定义

编号 `C-01…C-21`。每条给出对照位置与建议胜出方。**本节只记录，不修改原文。**

### C-01 治理契约自相矛盾（必须由队长裁决）

- `docs/README.md:29-35`：`SPEC` 只允许出现在 spec 层；`WITHDRAWN` 只允许出现在 decisions/archive。
- `docs/README.md:112-117`：又要求 evidence 层的 claim 台账"状态只能取五个词之一"，且被推翻的断言改为
  `WITHDRAWN` 保留；"没有证据就只能写 `SPEC` 或 `OPEN`"。
- **裁决建议**：以前者定"权威落点"、后者定"引用状态"。evidence 层可出现 `SPEC`/`WITHDRAWN`，但必须
  注明"引用自 spec/decisions，非在 evidence 层定义"。否则本表无法同时满足两条规则。
- **处置（D-01，已执行）**：`docs/README.md` §2 已改写——`SPEC`/`WITHDRAWN` 在 evidence 层表示
  **断言的可信度**，不是规范条文；唯一的硬规则保留为"spec 层永不出现 `VERIFIED`"。未采纳"必须注明
  引用出处"这一附加要求：本表每行的 `证据` 列已经承担溯源，再加一层标注只会制造重复。裁决记录见 `ADR-0012`。

### C-02 文档入口三处并列

- `docs/README.md:3`：本文件是"全部文档的唯一入口"。
- `archive/research/README.md:7`：先从 `../archive/review/README.md` 阅读。
- `archive/review/README.md:3`：FINAL 是当前唯一推荐架构。
- **胜出方**：`docs/README.md`。两份旧 README 在归档时应改为指向 archive 的说明页。

### C-03 版本 pin 四处重复，且精度不一致

- `archive/research/README.md:25-30`：taffy 0.15.1、parley 0.13.0、vello_gpu/vello_cpu 0.4.0、glifo 0.5.0、wgpu 30.0.2、winit 0.30.13 / accesskit 0.25.2 / accesskit_winit 0.34.2。
- `archive/research/CONCLUSION.md:11`：同一组。
- `archive/research/stack-decisions.md:7-19`：同一组，附可替换性。
- `archive/review/FINAL.md:16` 写 "Taffy 0.15.X"，`:23` 写 "taffy 0.15.1"。
- **胜出方**：`spec/01-route.md`（本次整理指定为唯一 pin 出处）；在它落地前以 `stack-decisions.md` 最精确。
  `FINAL.md:16` 的 "0.15.X" 应视为行文简写，不是第二个 pin。

### C-04 两套 Gate 字母表，同名不同义

- `archive/research/validation-and-roadmap.md:57-88`：Gate A = 交互模型与宏 API；Gate B = layout/text/style 基线；
  Gate C = 垂直桌面窗口切片；Gate D = 模型 viewport 与性能。
- `archive/review/FINAL.md:256-301`：A0 = 纯 Rust core + reducer；A1 = 单 Windows Todo 窗；B = layout/text/style；
  C = 三平台真实发布门；D = 可选 3D 与性能。
- **风险**：任何人引用 "Gate A/C" 都可能指两个不同门槛。
- **胜出方**：`FINAL.md` §9 + `docs/spec/08-gates.md`（本轮已建）。research 文档归档时必须改名（例如 "R-Gate"）或只保留历史说明。

### C-05 `class:if-*` 在 research 中仍以现行语气出现

- `archive/research/native-macro.md:19`：示例直接使用 `class:if-selected`；`:42` 称"草案"。
- `archive/research/style-system.md:128,137`：继续描述其字段替换行为，未标撤销。
- `archive/research/CONCLUSION.md:54`：称其为"候选的原生条件注册语法"。
- 撤销依据：`archive/review/FINAL.md:189-190`；`archive/research/README.md:9` 已声明撤销。
- **胜出方**：FINAL §6。归档时这三处必须加 `superseded-by: FINAL §6` 头，否则读者会实现第二套条件语法。

### C-06 ordinary inline vs 活跃 condition（最危险的一组）

- `archive/review/round2-debate.md:42`（D4）：排序键 `condition_rank` 在 `origin_rank` 之前，inline 永不压过活跃条件。
- `archive/review/FINAL.md:174-186`：§6「优先级单一规则」——origin/importance 先比较，condition 是否命中只决定 class 声明是否 eligible，不能跨 origin 胜过 ordinary inline。
- `archive/review/round3-review.md:69`：建议 `variant_rank > 0` 胜过 inline；同文件 `:5,:71` 自注该建议已废止（注意其引用行号对应旧版 FINAL，见 C-17）。
- `archive/review/validation-matrix-raw.json:2,257`：旧 RED 断言 hover=true 时 class 应胜 inline。
- `archive/review/verification-contract.json:9,75-77`：已修正为 hover=false/true 都取 inline。
- **胜出方**：FINAL §6 + W3C `#cascade-sort`（CLAIM-086/091/092）。raw JSON 只能当归档，不能作 golden。

### C-07 Mount 形状：`impl View` vs `MountResult<MountPlan>`

- `archive/research/native-macro.md:11,113,127`：组件返回 `impl View`。
- `archive/review/FINAL.md:37-47,135`：`Component::mount -> Result<MountPlan, MountError>`，组件只执行一次。
- `archive/research/README.md:9` 已声明 `impl View` mount 被撤销。
- **胜出方**：FINAL §1.2（CLAIM-089）。

### C-08 测试计数标签不一致

- `archive/research/CONCLUSION.md:60-62`：称 "7 个 reactive/style tests"。
- `archive/research/validation-and-roadmap.md:9`：拆为 4 reactive + 3 typed-style。
- `archive/review/VALIDATION.md:16-38`：合并为 7 passed，并明确警告不要把 runner 算成 17。
- `docs/evidence/validation.md` §3/§6 已收拢为 EVID-VAL-01/-12（H1/H2 各 7 passed）。
- **胜出方**：`evidence/validation.md`（CLAIM-003/004）。总量一致，标签不同；引用时写分子分母来源。

### C-09 同一批运行有两份证据记录

- `archive/research/validation-and-roadmap.md:5-14`：研究期记录，措辞把 3 个 style 测试计入 lib 行。
- `archive/review/VALIDATION.md:14-38`：上一轮的复跑记录，逐命令列出。
- **胜出方**：`evidence/validation.md`（新证据层）与 archive 中的原 VALIDATION。research 那份应标 historical，不能作为"本轮证据"。

### C-10 失效交叉引用

- `archive/review/round1-experts.md:12`：写"结果见 FINAL.md 的验证记录"。
- `archive/review/FINAL.md:3`：却把已运行证据指向 `VALIDATION.md`，正文没有执行记录章节。
- **胜出方**：`evidence/validation.md`（archive 保留原文）。round1 该句是过期引用。

### C-11 A0 验收在四份文件中重复

- `archive/review/FINAL.md:258-268`。
- `archive/review/verification-contract.json` contracts[0..4]。
- `archive/review/validation-matrix-raw.json` R4 contracts。
- `archive/review/round2-debate.md:217-218`（Gate 拆分）。
- 差异：`verification-contract` 是 11 组，raw 是 12 组且含已撤销的 hover golden。
- **胜出方**：FINAL §9 + `docs/spec/08-gates.md`（已建）；两份 JSON 降为规划/历史。

### C-12 "空态 zero row visits" 的评审结论已过期

- `archive/review/round3-review.md:73-76`：称 FINAL 只写"不必物化 N 行"，没写 zero row visits。
- 但 `archive/review/FINAL.md:265` 已明确"空态的 row visits 与 RowSet materialization 均为零"。
- **胜出方**：FINAL:265（CLAIM-046）。round3 该条若被当作待修缺陷，会误导实现。

### C-13 baseline 缺陷被三处重复描述

- `archive/review/FINAL.md:212`；archive `VALIDATION.md:119-120`；`docs/evidence/validation.md` §7 SRC-01/02；源码 `examples/native_poc/src/layout.rs:18-30,95-107`。
- 三处一致：measure 算了 baseline，回调只传 `.size`。**不是矛盾，是重复定义**。
- **胜出方**：本表 CLAIM-056/057 + `docs/spec/05-text.md`（已建）；其他地方只引用 ID。

### C-14 过滤生命周期两种默认

- `archive/review/round1-experts.md:348-353`（X1）：主张默认 retained-visibility。
- `archive/review/round2-debate.md:130-135`（D6）：默认 dispose UI owner、保留 row model。
- `archive/review/FINAL.md:59-60`：采纳 dispose；恢复需显式 `retained_visibility`/`dispose_and_snapshot`。
- **胜出方**：FINAL §1.3（CLAIM-095）。X1 的主张是 WITHDRAWN 的候选，不是未决项。

### C-15 Copy+String 的旧 file:line 会误导复查

- `archive/review/round1-experts.md:101,189,336` 与 `archive/review/FINAL.md:245` 都以"`TodoRow` 同时有 Copy 与 String"举例。
- 当前 `examples/todolist/main.rs:20-22` 已改为 `{ id: TodoId, completed: CompletedField }`，README:3 明确 title 是 mount 时 owned 读取。
- **胜出方**：当前源码。评审记录保留为历史，但不能作为"当前缺陷"引用。

### C-16 `RwStore` 与 "行创建时读取 title" 的旧文案已修

- `archive/review/round3-review.md:139` 引用旧 README:5/:12（RwStore、行创建时读取）。
- 当前 `examples/todolist/README.md` 已无 `RwStore`，并写明"每个 UI 行 mount 时读取一次"。
- **胜出方**：当前 README（CLAIM-048/098）。该评审条目的待办已完成。

### C-17 "FINAL 三种优先级措辞"的行号已过期

- `archive/review/round3-review.md:67-70` 引用 `FINAL.md:141`、`:142`、`:197` 作为三种措辞。
- 在当前 `archive/review/FINAL.md` 中：`:141-142` 是源码诊断与 proc-macro facade 段，`:197` 是 Typography
  支持表行，**都已不再指向优先级文本**。
- 当前唯一优先级规则在 `FINAL.md:174-186`（§6「优先级单一规则」），与 round3 的担忧不再冲突。
- **胜出方**：FINAL:174-186（CLAIM-086/091）。round3 的行号与"三份措辞"判断只能按 historical 对待；
  不要据此去 FINAL 里找第二套规则。

### C-18 证据层文件缺口（治理风险）

- `docs/README.md:53,92-93` 规划了 `evidence/probes.md` 与 `EVID-PROBE-*`。
- 本轮分工只指派了 `validation.md`、`index.md`、`matrix.json`、`08-gates.md`，没有 owner 写 `probes.md`。
- 后果：源码级事实（本表 `CODE` 条目）没有合法证据族，`EVID-PROBE` 引用会 dangling。`evidence/validation.md` §7 已自行用 `SRC-01..04` 记录其中四条，与本表 `CODE` 前缀形成第二套命名，必须统一。
- **处置**：已采纳（`DOCS` §6 + `ADR-0012`），本表 `CODE` 全部改为 `SRC path:line`，`validation.md` §7 的 `SRC-01..04` 同源，无第二套命名。
- **处置**：`probes.md` 已由测试与 CI 负责人交付（`EVID-PROBE-01…05`），不再是缺口；`SRC` 由 `docs/README.md` §6 承认为第三种证据前缀，不需另设 `EVID-SRC` 附录。

### C-19 规划文件与执行证据混在同一目录层级

- `archive/review/verification-contract.json:2-12` 与 `validation-matrix-raw.json:2` 用顶层 `status` 自降级为规划/历史。
- 但 `archive/review/README.md:25,27` 仍把它们与 VALIDATION 并列索引；不看 `status` 的读者会把 `green` 条件当已执行。
- **胜出方**：`evidence/validation.md` 是唯一执行记录；两份 JSON 只能进 archive/decisions。

### C-20 执行环境事实重复

- `archive/research/README.md:32`、archive `VALIDATION.md:3` 与 `docs/evidence/validation.md` §1 都写 Rust 1.99.1；前两者是历史/执行环境，后者是当前权威。
- **胜出方**：`evidence/validation.md` §1 保留"执行环境"权威；research 只作历史。

### C-21 测试名沿用已撤销的优先级措辞

- `examples/desktop_poc/src/lib/style.rs:216` 的测试名 `hover_only_supersedes_the_matching_property` 会被读成
  "hover 胜过 ordinary inline 已通过"。
- `docs/evidence/validation.md` §7 已核对：该测试只断言同一字段上 hover variant 覆盖 base variant，
  与 inline/class 的 origin 排序无关；仓库内没有 inline-vs-class fixture。
- **风险**：测试名与 FINAL:174-186 的措辞相反，会成为"旧结论已通过"的假证据。
- **胜出方**：FINAL §6。建议实现负责人重命名（不在本表范围）。

---

## 4. 待裁决与下一步

需要队长/其他 owner 裁决的点（按影响排序）：

1. **C-01**：`SPEC`/`WITHDRAWN` 能否在 evidence 层出现。若不能，本表 9 条 SPEC 与 12 条 WITHDRAWN 需要改挂在
   `spec/` 与 `decisions/` 下再被引用。
2. **C-18（已关闭）**：`probes.md` 由测试与 CI 负责人交付（`EVID-PROBE-01…05`）；`CODE` 前缀统一为 `SRC`，见 §0.1。
3. **C-04**：两套 Gate 字母表必须只保留一套；research 版需改名归档。
4. **C-03**：版本 pin 收敛到 `spec/01-route.md` 后，其余四处改为引用 `SPEC-ROUTE-00x`。
5. **C-06**：两份 JSON 中旧的 hover/inline golden 必须在归档时加 `superseded-by`，否则实现者可能照着 old RED 写测试。
6. **baseline（C-13）**：`evidence/validation.md` §7 SRC-01/02 已确认源码计算 `first_baseline` 但回调未传递，B Gate 应写成硬判据
   "`compute_leaf_layout` 的 measure 返回完整 `LayoutOutput`，known-size 早退也带同 epoch baseline"，
   而不是笼统写"baseline 未修"。

本轮**没有**做的事（避免读者误读）：

- 本文件作者没有复跑 `cargo test`、没有跑 GPU、没有修改任何源码或旧文档；H2 的复跑由 Test/CI 记录在 `evidence/validation.md` §6。
- 本文件作者没有验证 macOS/Metal；H2 的 headless Metal 观测见 EVID-VAL-15/16，真实窗口/surface 仍为 OPEN。
- 没有为 60–100 条之外的"架构好坏"下判断；本表只判真假。

后续若有人接手：

1. 先跑 `cargo test --manifest-path examples/{desktop,macro,native}_poc/...` 复现 CLAIM-002..004，记录主机。
2. 用 `rg` 复查 `SRC` 条目；源码若已变，先更新 CLAIM 再改 spec。
3. Store/Query/For 的 SPEC 条目在 A0 落地后逐条改 `VERIFIED` 并补 `EVID-VAL`；不得直接改状态而不补证据。
