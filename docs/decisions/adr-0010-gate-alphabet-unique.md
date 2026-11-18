---
doc-id: ADR-0010
layer: decisions
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - docs/spec/08-gates.md
  - docs/archive/README.md §3–§4
  - archive/research/validation-and-roadmap.md §4（旧 Gate 草案）
  - archive/review/FINAL.md §9
replaces: []
---

# ADR-0010：Gate 字母表唯一化

## 背景

仓库曾同时存在两套 Gate：

- 研究文档的 `Gate A/B/C/D`：A = 交互模型与宏 API，B = layout/text/style，C = 垂直桌面窗口切片，
  D = 模型 viewport 与性能。
- 最终规格的 `A0/A1/B/C/D`：A0 = 纯 Rust core，A1 = 单 Windows Todo 窗，B = 基线，C = 三平台发布门，
  D = 3D 与性能定稿。

两套字母表有相同的 `A/B/C/D`，但 `Gate A` 与 `Gate C` 的含义完全不同。任何人引用"Gate C"都可能指
"垂直切片"或"三平台发布门"，导致验收标准无法对齐。必须裁定唯一字母表。

## 备选项

1. **保留两套，按上下文区分**：读者需要额外知识，历史记录继续混用。
2. **以 `spec/08-gates.md` 的 `A0/A1/B/C/D` 为唯一**（选中）；研究版归档失效。
3. **取消 Gate，改用连续里程碑**：失去可判定的发布门。
4. **重新命名研究版为 R-A/R-B/...**：可行，但归档不应被改写（ADR-0013），只能在新文档声明失效。

## 决定

1. **唯一字母表**：`SPEC-GATE-A0`、`A1`、`B`、`C`、`D`（`docs/spec/08-gates.md`）。
2. **研究版 Gate A/B/C/D 随归档失效**；新文档不得引用（`docs/archive/README.md` §3）。
3. **判据可判定**：每条 Gate 必须写"通过判据 + 需要证据 + 当前状态"；`blocked` 既不是 pass 也不是 fail。
4. **只加严或依证据放宽**：放宽必须有 ADR 指出哪条证据不支持原判据；不得为让 Gate 变绿修改失败测试。
5. **改名/拆分保留映射**：新增子 ID 时必须在 ADR 中列 old → new，历史记录不得断链。

## 依据

- `SPEC-GATE-A0-01`…`A0-11`、`A1-01`…`A1-09`、`B-01`…`B-10`、`C-01`…`C-07`、`D-01`…`D-05`：
  现行判据全集。
- `docs/archive/README.md` §3：旧 Gate 字母表的权威替代关系；§4：旧 A/B/C/D 已列过期。
- `CLAIM-003`：探针测试计数不构成 Gate 通过；`CLAIM-040`：A0 headless trace 的状态以台账为准。
- `EVID-MATRIX.coverage`：当前 42 条判据中 0 条 `VERIFIED`、1 条 `PROBE`、41 条 `OPEN`——这是
  "Gate 未完成"的机器可读事实，不是能力宣称。
- `SPEC-GATE-A0-04` 已有反例 `EVID-VAL-08`；`SPEC-GATE-B-01`…`-03` 已有源码缺陷
  （`evidence/validation.md` §7 SRC-01…03）——两者证明 Gate 状态必须与证据绑定。

## 影响与撤销条件

**影响**

- 所有 spec/product/decisions 文档只能使用 `SPEC-GATE-*`；引用研究版 Gate 视为过期引用。
- CI 分层（`docs/spec/08-gates.md` §7）以本字母表为唯一输入。
- 评审记录可以保留旧字母表，但必须位于 `archive/` 或明确标注 `historical`。

**撤销条件（可判定）**

只有在某个 Gate 需要按平台/后端拆分时，才允许新增**子 ID**（例如在 `SPEC-GATE-C-01` 下新增按平台记录的子条目），并且：

1. 旧 ID 保留并在新 ADR 中给出 old → new 映射；
2. 不恢复第二套 `A/B/C/D` 字母表；
3. `EVID-MATRIX` 的 coverage 能同时统计旧 ID 与新 ID，不产生重复计数。

否则本决定不变。
