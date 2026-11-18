---
doc-id: ADR-0011
layer: decisions
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - docs/README.md §1–§8
  - docs/spec/00-scope.md
  - docs/evidence/claim-register.md
replaces: []
---

# ADR-0011：文档四层治理

## 背景

整理前，`archive/research/` 与 `archive/review/` 各自重复声明技术路线、版本 pin 与支持边界；已被撤销的
`class:if-*`、condition 优先排序、`impl View` mount 与现行方案混排，只靠一句"以 FINAL 为准"兜底。
读者无法在短时间内判断：哪句话是要求、哪句话是事实、哪句话是历史。

必须裁定：文档的层、状态词、ID 与引用纪律。否则后续每新增一份文档都会制造第二权威。

## 备选项

1. **单一超大文档**：更新集中，但无法区分规范/证据/历史，且冲突不可见。
2. **四层（spec / evidence / decisions / archive）+ 状态词 + ID**（选中）。
3. **外部 wiki**：脱离 git 历史，无法与代码同批评审。
4. **只保留最终规格**：丢失"为什么否决"的论证，后续会重踩同样的坑。

## 决定

1. **四层模型**：`spec/`（要建什么）、`evidence/`（跑过什么）、`decisions/`（为什么这样定）、
   `archive/`（过去怎么想，只读）。
2. **一个事实只定义一次**：其他位置只能引用 ID（`CLAIM-*`、`EVID-*`、`SPEC-*`、`ADR-*`）。
3. **状态词固定五个**：`SPEC` / `PROBE` / `VERIFIED` / `WITHDRAWN` / `OPEN`；**`VERIFIED` 只允许出现在
   evidence 层**（`docs/README.md` §2）。
4. **每份文档必须有文件头**：`doc-id`、`layer`、`status`、`date`、`owner`、`sources`、`replaces`。
5. **归档只读**：原文不改；旧结论若仍有效，必须在新层重新落一条并标注来源（ADR-0013）。

## 依据

- `docs/README.md` §1–§8：四层模型、状态词、ID 前缀、文件头契约、写作规则、工作顺序。
- `SPEC-SCOPE-021`：能力宣称必须能落到证据；这与"spec 不写已验证"一致。
- `CLAIM-003`/`CLAIM-006`：13 条探针与 H1/H2 主机边界；`CLAIM-039`：生产 runtime 未实现。
- `EVID-VAL-19`：目标示例 `todolist/main.rs` 单独编译有 40 个错误，证明"设计稿 ≠ 可运行程序"必须
  在文档层显式区分。
- `CLAIM-001`…`CLAIM-100`：事实层的唯一台账，供 spec/decisions 引用。

## 影响与撤销条件

**影响**

- 新增文档前必须先读 `docs/README.md`；不得在 spec 里写"已验证"，不得在 evidence 里写"应该"。
- 任何"重复定义"都是缺陷：交叉检查发现同一断言在两处定义且无引用关系，按治理问题处理。
- 归档迁移只移动文件，不改正文；需要修正时改新层。

**撤销条件（可判定）**

只有在同时满足以下条件时，才可调整层数或状态词：

1. 交叉检查发现同一事实在两处定义且无法用"引用 vs 权威"解释（可列出具体文件与行号）；
2. 现有五词无法表达某类必要状态（必须给出至少两个真实条目作为反例）；
3. 新方案仍能保证"spec 不写 VERIFIED"与"归档只读"。

仅"层太多"或"写起来麻烦"不构成撤销依据。
