---
doc-id: SPEC-<区段>
layer: spec
status: normative
date: YYYY-MM-DD
owner: <角色名>
sources:
  - <来源：ADR / CLAIM / 外部规范>
replaces: []
---

# <区段标题>

## SPEC-<区段>-NNN <一条可判定的要求>

- **要求**：用 MUST / MUST NOT / SHOULD 写，一句话说清"必须是什么样"。
- **失败模式**：不满足时会出现什么**可观测**的错误（崩溃、错位、丢帧、IME 丢字……）。写不出失败模式，说明这条不是规范。
- **来源**：`ADR-NNNN` 或 `CLAIM-NNN`，不复述其内容。
- **证据状态**：本条**只写要求**。它有没有被实现、被谁验证，写在 `evidence/claim-register.md` 的对应 `CLAIM-*` 行里；spec 层不写 `VERIFIED`。

## 写作检查

- 每条只断言一件事，能被单独判定遵守或不遵守。
- 数值、版本、平台清单只在本区段定义一次；其他文件引用 ID，不复制。
- 条目编号不得复用；撤销的条目不删，标 `WITHDRAWN` 并写明撤销依据。
