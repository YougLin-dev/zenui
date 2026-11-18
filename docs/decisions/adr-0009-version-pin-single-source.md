---
doc-id: ADR-0009
layer: decisions
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - docs/README.md §3、§7
  - docs/spec/01-route.md
  - docs/archive/README.md §3
  - archive/research/stack-decisions.md
replaces: []
---

# ADR-0009：版本 pin 单点定义

## 背景

整理前的仓库在至少四处重复声明同一组依赖 pin：`research/README.md`、`research/CONCLUSION.md`、
`research/stack-decisions.md` 与 `review/FINAL.md` §0。同一份事实出现多个副本，导致升级时无法确定哪一处
是权威，也让"某个 pin 是否仍有效"无法判定。

治理契约要求"一个事实只在一处定义"。必须裁定：版本 pin 的唯一出处、升级流程，以及归档中的旧 pin
如何处理。

## 备选项

1. **每份文档各自维护 pin**：读者方便，但必然漂移。
2. **只信 `Cargo.lock`**：机器可读，但没有"为什么选它/允许替换什么"的语义。
3. **单一 pin 文件 + 其他文档只引用 ID**（选中）。
4. **把 pin 写进每个 crate 的 README**：同样会漂移。

## 决定

1. **`spec/01-route.md` 的 `SPEC-ROUTE-001` 是全仓唯一 pin 出处**；其他任何文档只能引用
   `SPEC-ROUTE-001`，不得复述版本号（`docs/README.md` §7 第 4 条）。
2. **窗口/无障碍三元组合单独标记为待验证基线**（`SPEC-ROUTE-003`），不得因为"pin 已选"就被写成已验证。
3. **升级任一 pin 必须有 ADR**，并在同一变更内更新 `SPEC-ROUTE-001` 与 lock；不得只改 `Cargo.toml`
   （`SPEC-ROUTE-004`）。
4. **归档中的 pin 一律失效**：归档保持原文，不"顺手修正"版本号；需要引用时以 `SPEC-ROUTE-001` 为准
   （`docs/archive/README.md` §2–§3）。
5. **发现第二处版本号即缺陷**，按文档治理问题处理，而不是"多写一份方便读者"。

## 依据

- `SPEC-ROUTE-001`：生产候选 pin 唯一表；`SPEC-ROUTE-003`：三元组合从未一起编译/运行；
  `SPEC-ROUTE-004`：升级纪律。
- `SPEC-SCOPE-021`：能力宣称必须能落到证据；未验证的 pin 组合不得写成"稳定"。
- `CLAIM-005`、`CLAIM-012`、`CLAIM-082`：H1 主机、MSRV 与窗口三元组合的验证边界。
- `docs/archive/README.md` §3：归档 pin 的权威替代关系。
- `CLAIM-039`/`CLAIM-053`：runtime 尚未实现，pin 只是依赖选择，不是能力证明。

## 影响与撤销条件

**影响**

- 所有 `spec/`、`product/`、`decisions/` 文档在提到依赖时只写 `SPEC-ROUTE-001` 或具体 `SPEC-ROUTE-00x`。
- 评审/归档可保留旧版本号，但必须带 `status: historical` 或位于 `archive/`，且不得被新文档引用为现行。
- 升级 PR 缺 ADR 或未更新 pin 表，复审阻断。

**撤销条件（可判定）**

只有在出现**独立发布单元**（例如 renderer 以独立版本发布、需要自己的兼容矩阵）时，才允许新增一个
pin 文件；新增必须同时满足：

1. 有 ADR 说明为什么不能共用 `SPEC-ROUTE-001`；
2. 在 `decisions/index.md` 与本 ADR 的 `supersedes` 链中登记；
3. 新文件仍然只有一个版本号表，不回到"每份文档各自 pin"。

否则单点规则不变。归档不因任何理由被编辑。
