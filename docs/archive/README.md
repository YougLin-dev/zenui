---
doc-id: ARCHIVE-README
layer: archive
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources: []
replaces: []
---

# 归档区：历史材料，只读

## 1. 这里是什么

`docs/archive/research/` 与 `docs/archive/review/` 是 2026-11-18 整理前的研究与评审原文，**逐字保留**。它们记录了当时的推理、争论与被否掉的方案，有追溯价值，但**没有任何权威性**。

## 2. 三条只读规则

1. **不修改原文。** 归档不应被编辑，包括"顺手修正"过时版本号、撤销已撤销的说法。需要修正的是 `spec/`、`evidence/`、`decisions/`，不是历史。
2. **不直接引用其结论。** 需要引用时，引用 `CLAIM-*` 或 `SPEC-*`；原文只在 `sources:` 字段里作为来源痕迹出现（`archive/research/xxx.md §N`）。
3. **其中的任何版本 pin、Gate 字母表、完成度描述一律失效。**

## 3. 权威替代关系

| 归档中的内容 | 现行权威 |
|---|---|
| `research/validation-and-roadmap.md` §4 的 Gate A/B/C/D | [spec/08-gates.md](../spec/08-gates.md)（`SPEC-GATE-*`） |
| 全部版本 pin（`research/README.md`、`CONCLUSION.md`、`stack-decisions.md`、`review/FINAL.md` §0） | [spec/01-route.md](../spec/01-route.md)（`SPEC-ROUTE-001`，全仓唯一） |
| `review/FINAL.md` 的规格正文 | `spec/00-scope.md` … `spec/08-gates.md` |
| `review/VALIDATION.md`、`research/validation-and-roadmap.md` §1 | [evidence/validation.md](../evidence/validation.md)（`EVID-VAL-*`） |
| 各文档里的"已通过 / 未完成"判断 | [evidence/claim-register.md](../evidence/claim-register.md)（`CLAIM-*`） |
| `research/README.md`、`review/README.md` 的入口地位 | [docs/README.md](../README.md) |

## 4. 已知过期物（superseded-by）

下列内容在归档中仍可读到，但**已被明确推翻**。若要复查旧行为，请以右列为准：

| 归档位置 | 过期的说法 | 现行 |
|---|---|---|
| `research/CONCLUSION.md` §Tailwind、`research/style-system.md` | `class:if-*` / `class:if-selected` 作为现行条件语法 | `SPEC-ROUTE-022`（`WITHDRAWN`），条件样式见 [spec/03-macro.md](../spec/03-macro.md) `SPEC-MACRO-014` |
| `review/verification-contract.json` 的优先级规则 | 激活后的 condition 可胜过 ordinary inline | `SPEC-ROUTE-023`（`WITHDRAWN`），ordinary inline 恒胜，见 [spec/04-style.md](../spec/04-style.md) |
| `review/validation-matrix-raw.json` 的 golden 期望 | 旧 hover/inline 优先级黄金用例 | 同上；该 JSON 的 `_status` 已声明其归档身份，**不得当测试证据** |
| `research/*`、`review/round*.md` 的 `impl View` / builder mount | 早期 mount 形态 | `SPEC-ROUTE-024`，现行 `MountResult<MountPlan>` 见 [spec/02-runtime.md](../spec/02-runtime.md) |
| 各文档中"Copy 结构体承载 String"的模型行 | 早期数据模型 | `SPEC-ROUTE-025`（实际触发 E0204） |
| 各文档中"过滤即 display:none / 默认保留 store" | 早期生命周期 | `SPEC-ROUTE-026`，见 [spec/02-runtime.md](../spec/02-runtime.md) |
| `research/runtime-and-rendering.md` §4 对 intrinsic 能力的乐观措辞 | "完整 bidi 宽度支持" | `CLAIM` 台账中该项为 `WITHDRAWN`/`OPEN`，缺陷见 [spec/05-text.md](../spec/05-text.md) |
| `research/validation-and-roadmap.md` §4 的 Gate 字母表 | A/B/C/D | `SPEC-GATE-A0/A1/B/C/D` |

## 5. 为什么要保留而不是删除

评审过程本身是研究资产：它解释了"为什么不是 Flutter 三树""为什么不直接跑 Tailwind CSS"。这些取舍在 `decisions/` 里以 ADR 形式保留了结论，归档保留了论证过程与反方意见。删掉归档会让后续维护者重新踩一遍同样的坑。
