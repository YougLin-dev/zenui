---
doc-id: ADR-0021
layer: decisions
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - docs/spec/08-gates.md
  - docs/spec/01-route.md
  - docs/decisions/adr-0009-version-pin-single-source.md
  - docs/decisions/adr-0011-docs-four-layers.md
  - docs/decisions/adr-0012-evidence-status-and-prefixes.md
  - docs/evidence/validation.md
  - crates/core/src/lib.rs
  - crates/core/Cargo.toml
  - examples/macro_poc/frontend/tests/compile_fail.rs
replaces: []
---

# ADR-0021：错误码只由 trybuild 快照钉住，`compile_fail` 只证明「不编译」

## 背景

`SPEC-GATE-A0-02` 的判据原文点名 trybuild（「`#[derive(Copy)] struct{String}` 必须 E0204」），
`SPEC-GATE-A0-03` 要求 `ViewId` / `RowKey` / `LayoutNodeId` 互传必须编译失败。两个 gate 的现状不同：

- `main`（`7676f1f`）：`crates/core/src/lib.rs:24-84` 有 **6 条**互斥用例，全部声明 E0308，由
  `EVID-VAL-24` 计数；`SPEC-GATE-A0-03` 的 `PROBE` 依据就是这 6 条。
- `SPEC-GATE-A0-02`：仓库内 fixture 尚未建立，当前只有语言层探针 `EVID-VAL-18`。`P3` 分支
  （`f6ea74e`，未合并）在 `lib.rs:29-105` 新增 2 条，声明 E0204 与「拥有式 `get` 不编译」，使该分支
  总数为 8 条；该分支的证据层另记新条目，尚未并入 `main`，本条不引用其 ID。

`compile_fail` 的语义只是「这段代码不能编译通过」。它不读注释、不比对诊断、不校验错误码，连
`compile_fail,E0204` 这样的后缀也只是被吞掉。本轮在 `SPEC-ROUTE-001` 的 pin 工具链上用 `rustdoc --test`
逐条实测：

| 输入 | 按判据应当 | 实测 |
|---|---|---|
| 标注 `compile_fail`，正文为 `let _ = undefined_symbol_xyz();` | 失败（错因不是类型规则） | `compile fail ... ok` |
| 标注 `compile_fail,E0204`，正文同上 | 失败（错误码不符） | `compile fail ... ok` |
| 标注 `compile_fail`，正文是合法代码 | 失败（这段代码能编译） | `FAILED` |

第三行说明它确实检查「能不能编译」，前两行说明它完全不检查「为什么不能编译」。于是任何笔误——拼错
方法名、漏一个 `use`、参数个数不对——都能让一条自称验证 E0204 的用例保持绿色。现有 doctest 因此只保证
「这些片段编译不过」，不保证「因为要求的类型规则编译不过」；把「不可编译」读成「按预期不可编译」，
正是本轮把绿灯当成验收的问题。

仓库里有可复用的先例与成本基线：`examples/macro_poc` 用 trybuild 承载 3 条 compile-fail
（`frontend/tests/compile_fail.rs` 加同名 `.stderr` 快照），`EVID-VAL-13` 记录了在 H2（macOS）上连同
诊断文本一起逐字匹配的结果。它证明机制在该平台上可用，不证明跨平台；跨平台由 CI 矩阵回答。

## 备选项

1. **保持 doctest，把期望错误码写在注释里**：改动为零。否决——注释不参与判定，等于没有断言；它恰好
   制造了最贵的一种绿灯：看起来钉住了错误码，实际只钉住了「不编译」。
2. **用 `compile_fail,E0204` 收紧**：看起来精确。否决——实测该后缀不校验错误码（背景表第二行），
   是假精确；写进判据只会让人误以为已经验证。
3. **在测试里直接调 `rustc` / `cargo` 并匹配 stderr**：可行，但等于自建轮子。否决——要自己处理
   edition、extern crate 路径、诊断归一化、路径与列号差异和跨平台输出；trybuild 已经解决这些，仓库里
   也有现成快照，自建会成为第二套需要长期维护的机制。
4. **改用 `ui_test` 或 rustc 的 `compiletest`**：`ui_test` 功能更全但仓库无先例、需要自己定初始配置；
   `compiletest` 是 rustc 内部工具链的一部分，不是稳定的第三方依赖。否决：前者引入新的配置面，后者
   依赖 nightly/内部接口，均不如沿用已有先例划算。
5. **编译期断言（`#[deny]`、const 断言、trait bound 技巧）**：只能表达正向的「这段代码必须编译通过」，
   无法表达「这个具体片段必须因为 E0204 失败」。否决。
6. **`trybuild` 作为 `[dev-dependencies]`，UI 用例与 `.stderr` 快照进仓库**（选中）：判定强度等于
   「诊断逐字匹配」，错误码、span 与消息都在快照里；不进入发布物，正常依赖树不变。

## 决定

1. **「必须因某个错误码编译失败」的判据，唯一承载方式是 trybuild UI 测试。** 运行入口是顶层测试目标
   `crates/core/tests/compile_fail.rs`（Cargo 只自动发现 `tests/*.rs`；放在 `tests/ui/` 里的 `#[test]`
   不会被执行），内容为 `TestCases::compile_fail("tests/ui/*.rs")`，glob 相对 crate 根。用例是
   `crates/core/tests/ui/<case>.rs` 与同名 `.stderr`。断言强度以 `.stderr` 逐字匹配为准，包含
   `error[E0xxx]` 与 span。
2. **`compile_fail` doctest 不得再承担 gate 判据，且迁移必须原子。** 顺序是：新增 UI 用例 → 证据层
   用新 `EVID-*` 记录 → 删除对应 doctest，三件事在同一个 PR。A0-03 的 `PROBE` 依据在同一个 PR 内由
   6 条 doctest 切换为 UI 快照；只删不补时，A0-03 必须退回 `OPEN`，不得保留悬空引用。`lib.rs` 保留
   类型域的文字说明，代码片段只在 `tests/ui/` 出现。理由见 `ADR-0011`（一个事实只定义一次）：同一
   判据不能同时有「能失败」和「不能失败」两个载体，也不能一个都没有。
3. **只钉 SPEC 点名的错误码，并把加强说清楚。** A0-02 点名 E0204；「拥有式 `get` 不编译」没有点名
   错误码，UI 用例按编译器实际诊断写快照，并在 EVID 里记下实际代码，不得替它发明一个。A0-03 只要求
   「必须编译失败」，本条把 6 条互斥用例固定到 E0308，是**严于 spec 的加强**，目的是排除「因笔误而
   失败」；它的代价（诊断变动即红）由决定 5 的工具链协议承担。
4. **依赖边界。** `trybuild` 只出现在 `zenui-core` 的 `[dev-dependencies]`；`[dependencies]` 保持为空
   （`crates/core/Cargo.toml:9-11` 的 crate 边界约束，不是 `SPEC-GATE-A0-01` 的条文），
   `cargo tree -e normal` 不变，发布物不携带它。跨 workspace 同版本是**本条新增的约定**，不是
   `SPEC-ROUTE-001` 已有的 MUST（该条范围是「同一 workspace 内」）：两个 workspace 各以自己的
   `Cargo.lock` 记录，当前没有 CI 校验版本相等，由评审在升级时检查——不假装有机械防线。它的传递闭包
   不小（serde、serde_json、toml、glob、termcolor 等），这正是离线风险与撤销条件 3 的来源。
5. **快照与工具链同源。** `.stderr` 绑定 `SPEC-ROUTE-001` 的 pin。升级工具链必须在同一次变更里重新
   bless 快照、把 diff 写进变更说明并附证据；re-bless 由 Release Engineer 执行、Core Engineer 复核，
   不得把快照漂移当作 flaky 重跑。
6. **诊断质量是判据的一部分。** 每条 UI 用例的 `.stderr` 必须显示错误码与指向用户代码的 span。宏自
   定义诊断没有错误码时，必须固定完整文本（先例：`examples/macro_poc` 的 3 条快照），不得以「测试绿」
   代替。
7. **证据口径。** `EVID-*` 必须为每条有序类型对（例如 `ViewId → RowKey`）各记一行：SPEC 断言、
   用例文件、快照 commit、实际错误码。六条互斥用例共享 E0308，但不得合并成一行。`EVID-VAL-18` 是
   语言层探针，继续有效，但不是 A0-02 的 gate 夹具；新 UI 用例另记 `EVID-*`，A0-02 的状态注释改为
   引用它。已有 `EVID-*` 不得就地改数字，变化要用新条目取代旧条目。
8. **「没有错误码就不计入」只适用于编译失败类判据。** A0-02 的「mount 只读一次」与 A0-03 的
   stale / 跨 runtime 夹具属于运行时候选证据，仍按 `EVID-*` 的运行规则计入（例如 `EVID-VAL-23`）。

## 依据

- `SPEC-GATE-A0-02`：「trybuild：`#[derive(Copy)] struct{String}` 必须 E0204；运行时 read 计数恰为 1；
  `String::with` 借读合法、拥有式 get 不编译」；当前状态 `OPEN`（仓库内 fixture 尚未建立）。
- `SPEC-GATE-A0-03`：「类型层 ViewId/RowKey/LayoutNodeId 互传必须编译失败」；当前 `PROBE` 的依据是
  `EVID-VAL-24` 的 6 条互斥 doctest。
- `EVID-VAL-24`：`main` 上 `zenui-core` 的 6 条 `compile_fail` doctest 被计入 compile-fail fixture。
- `EVID-VAL-18`：最小夹具在 pin 工具链上确实以 E0204 失败；这是语言层事实，不是 gate 夹具。
- `EVID-VAL-13`：`examples/macro_poc` 的 trybuild 3 条 compile-fail 在 H2（macOS）上逐字匹配，`.stderr`
  快照无平台漂移；平台范围仅限所记录的主机。
- `SRC crates/core/src/lib.rs:24-84`（`main`）：6 条 `compile_fail` fence 与注释中声明、但工具不校验的
  E0308。`SRC crates/core/src/lib.rs:29-105`（`f6ea74e`）：新增的 2 条（E0204 与拥有式 `get`）。
- `SRC examples/macro_poc/frontend/tests/compile_fail.rs:1-6`：现有 trybuild 入口；
  `SRC examples/macro_poc/frontend/tests/compile_fail/`：`.stderr` 快照形态。
- `SRC crates/core/Cargo.toml:9-11`：`[dependencies]` 为空及其边界说明。
- 复算命令：把背景表格三行分别写成独立 `t.rs`（正文放进 `//! ...` doctest），在 pin 工具链上执行
  `rustdoc --test <file>.rs`。**这是本 ADR 的证据缺口，如实记录**：按 `ADR-0012`，运行事实的唯一 ID
  是 `EVID-VAL-*`，而本轮不允许改证据层，因此该实验目前只有命令、没有证据 ID。实现 PR 必须为它补录
  `EVID-VAL-*`（含提交进仓库的夹具），并把本条依据改为引用该 ID；补录前不得引用本条作为 A0-02 /
  A0-03 的证据。
- `ADR-0011`：一个事实只定义一次。
- `ADR-0012`：`SRC path:line` 用于仓库内可直接复核的源码/清单事实；运行事实归 `EVID-VAL-*`。
- `ADR-0009` / `SPEC-ROUTE-001`：版本 pin 单一出处，其他文档不得复述版本号。

## 影响与撤销条件

**影响**

- `crates/core` 新增 dev-dependency、`tests/compile_fail.rs`、`tests/ui/` 与 `.stderr` 快照；CI 命令
  不变（`cargo test --workspace --locked`）。
- 诊断文本的任何变化都会让快照测试失败，需要显式 re-bless 并人工读 diff。这是设计意图：诊断变化必须
  被看见，而不是被宽松断言掩盖。
- 工具链升级成本上升：升级变更必须带快照 diff，且该 diff 要能解释（新 span、新措辞、错误码不变）。
- 迁移会改变 doctest 执行数。证据层必须**新增**条目取代旧条目，不得就地改写：
  `docs/evidence/validation.md:72`（`EVID-VAL-24` 的 6 条）与 `:407`（9 条 compile-fail 的构成）、
  `docs/spec/08-gates.md:48`（A0 汇总）、`:76`（A0-02 fixture 尚未建立）、`:77`（A0-03 引用 6 条
  doctest）、`docs/evidence/matrix.json` 的计数，都需要在迁移 PR 内由 gate/evidence owner 更新。本
  ADR 只定顺序与口径，不改这些文件。
- A0-02 / A0-03 的状态不由本 ADR 改变：A0-02 在 UI 用例落地后仍须满足运行时 read 计数；A0-03 的
  `PROBE` 依据随迁移切换到 UI 快照，切换失败则退回 `OPEN`。

**撤销条件（可判定）**

1. 若同一个 commit 上三个 CI job 中恰好一个失败、其余通过，且 `.stderr` diff 不只是 trybuild 归一化
   的路径与行尾差异（即出现不同的 span、措辞或错误码），该用例必须移出 gate 证据；随后新开 ADR 在
   「单平台 + 显式平台范围」与「直调 `rustc`」之间二选一。
2. 若 `SPEC-GATE-A0-02` 的判据改为不依赖错误码（例如只要求运行时行为），本条对 A0-02 的约束失效；
   对 A0-03 仍然有效。
3. 若 `cargo test --workspace --locked` 在离线环境无法解析 dev-dependency（先例：
   `docs/evidence/index.md` §2 记录 `--offline` 下 trybuild 不在索引里），必须重新评估承载方式，并把
   「UI 用例需要联网或 vendor」写成显式前提；当前没有任何 CI job 执行离线解析，因此这一条是人工
   前置检查，不假装自动触发。
