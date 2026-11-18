---
doc-id: ADR-0013
layer: decisions
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - docs/archive/README.md §2–§5
  - docs/README.md §1
  - docs/spec/01-route.md
  - docs/spec/04-style.md
replaces: []
---

# ADR-0013：归档只读，旧 pin / Gate / 完成度一律失效

## 背景

`archive/research/` 与 `archive/review/` 保存了整理前的研究与评审原文，其中既有仍有效的论证，也有被推翻的
结论（`class:if-*`、condition 优先、`impl View`、Copy+String 模型行、旧 Gate 字母表、多处版本 pin）。
最直接的诱惑是"顺手把归档里的旧版本号改掉"或"把旧结论删掉"，但这会破坏原文哈希与可追溯性，也无法
再回答"当时为什么这样想"。

必须裁定：归档是否可编辑，以及归档中的旧事实如何失效。

## 备选项

1. **重写归档**：把旧结论改成现行说法，保持"看起来一致"。
2. **只读归档 + 权威替代表 + 已知过期物清单**（选中）。
3. **删除过期文件**：丢失论证过程与反方意见。
4. **保留原文但不声明失效**：读者会把旧 pin/Gate 当现行。

## 决定

1. **归档原文逐字保留**，包括过时版本号、被撤销的语法与旧 Gate 字母表；不"顺手修正"（D-04）。
2. **归档中的任何版本 pin、Gate 字母表、完成度描述一律失效**；引用时以 `spec/01-route.md`、
   `spec/08-gates.md`、`evidence/claim-register.md` 为准。
3. **`docs/archive/README.md` §3 维护权威替代表**；§4 维护"已知过期物（superseded-by）"清单。
4. **两份历史 JSON 不修改**：`verification-contract.json` 与 `validation-matrix-raw.json` 保持原文；
   其旧 hover/inline golden 不得当测试证据（D-05）。
5. **归档不是事实来源**：需要引用旧结论时，先在新层落 `CLAIM-*`/`SPEC-*`，并在 `sources:` 标注
   `archive/... §N`。

## 依据

- `docs/archive/README.md` §2：三条只读规则；§3：权威替代关系（旧 Gate → `SPEC-GATE-*`，旧 pin →
  `SPEC-ROUTE-001`，旧 VALIDATION → `evidence/validation.md`）；§4：已知过期物清单。
- `SPEC-ROUTE-001`/`-002`：版本 pin 与 Classic 角色的唯一权威。
- `SPEC-STYLE-011`/`-012`：ordinary inline 恒胜、condition 非排序键——与归档中 D4/D8 的旧规则相反。
- `CLAIM-089`、`CLAIM-090`、`CLAIM-091`、`CLAIM-092`、`CLAIM-095`、`CLAIM-096`：六条 `WITHDRAWN`
  断言，均以现行 spec 为推翻依据。
- `CLAIM-003`/`CLAIM-006`：探针测试计数与 H1/H2 主机边界（计数以台账为准）；归档中的"已通过"不能跨主机复用。

## 影响与撤销条件

**影响**

- 归档迁移是纯移动操作；`git diff` 对归档文件应为空（除路径变化）。
- 读者在归档中看到旧 pin/Gate 时，必须查 `archive/README.md` §3/§4，而不是直接引用。
- 若归档中的某个论证仍有效，必须在新层重新落条；归档本身不获得权威。

**撤销条件（可判定）**

1. **发现归档文件被修改**（内容哈希与 git 历史不符，或出现现行版本号/新结论）→ 立即回滚该文件，并按
   治理事故处理；这是硬性回滚条件，不是"讨论后决定"。
2. 若某个归档事实在新层没有替代、且实现必须引用它：允许新增一条 `CLAIM-*` 并标注 `sources: archive/...`，
   但**不得编辑归档原文**；若无法写出可判真假的断言，则该事实保持 `OPEN`。
3. 若归档 README 的替代表与实际 spec ID 不一致（链接断链或 ID 不存在），先修 README 的替代表，不改归档。

任何"为了文档一致而重写历史"的提案都不被接受。
