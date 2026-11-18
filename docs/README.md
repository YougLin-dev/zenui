---
doc-id: DOCS-HOME
layer: governance
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources: []
replaces: [research/README.md, review/README.md]
---

# Zenui 文档总览与治理契约

> 本文件是**全部文档的唯一入口**，也是文档整理的**治理契约**。新增或改写任何文档前，先满足这里的规则。
>
> 日期：2026-11-18 ｜ 维护：Crew 队长（架构） ｜ 层级：`spec/governance` ｜ 状态：normative

## 0. 为什么要有这份契约

截至本轮整理前，仓库存在两套并列的叙述（`archive/research/` 与 `archive/review/`），它们各自重复声明技术路线、版本 pin 与支持边界，并保留了已被撤销的候选方案。读者无法在 5 分钟内判断：哪句话是要求、哪句话是事实、哪句话是历史。本契约用**分层 + 状态词 + 可追溯 ID** 消除这种歧义，规则只有一条：

> **一个事实只在一处定义；其他地方只能引用它的 ID。**

## 1. 四层文档模型

| 层 | 目录 | 回答的问题 | 权威性 | 能否作为事实引用 |
|---|---|---|---|---|
| **spec** 规范 | `docs/spec/` | 要建什么、必须满足什么 | 唯一规范权威 | 只能当“要求/目标”，不能当“已实现” |
| **evidence** 证据 | `docs/evidence/` | 实际跑过什么、输出是什么 | 事实权威 | 可以，但必须附命令或可复跑路径 |
| **decisions** 裁决 | `docs/decisions/` | 为什么这样定、否掉了什么 | 决策权威 | 可以，用于解释取舍 |
| **archive** 历史 | `docs/archive/` | 过去怎么想的 | 无权威，只读 | 不可以 |

两条铁律：

1. **spec 里不写“已验证”，evidence 里不写“应该”。** 任何“已完成 / 已通过”的表述必须能指向 `EVID-*` 条目。
2. **archive 只读。** 归档文档保持原文与 git 历史，不再修订；其中的旧结论若仍有效，必须在新层重新落一条并标注来源。

## 2. 状态词表（全文统一，只用这五个）

| 状态 | 含义 | 允许出现在 |
|---|---|---|
| `SPEC` | 目标要求，尚无实现 | spec；evidence（作为一条“尚未被证据支持的事实断言”） |
| `PROBE` | 已有探针/夹具的最小验证，不等于产品完成 | spec、evidence |
| `VERIFIED` | 有可复跑命令与输出记录 | **仅** evidence |
| `WITHDRAWN` | 曾提出、已被裁决撤销（必须写明撤销依据） | decisions、archive；evidence（作为“该断言已被推翻”的事实记录） |
| `OPEN` | 未决，需要有结论 | 任意层 |

`CLAIM-*` 台账是**事实断言表**而非规范条文：一条断言可以处于 `SPEC`（还没证据）或 `WITHDRAWN`（已被推翻），这描述的是**断言的可信度**，不是要求在 spec 层书写。方向性规则只有一条：**spec 层永不出现 `VERIFIED`**。

禁止使用的模糊措辞：“基本完成”“大部分支持”“接近生产”“应该可以”。需要表达覆盖度时，给出**分子/分母与来源**（例如：`3/42 个 utility family 有 probe 级验证，来源 EVID-PROBE-02`）。

## 3. 文档 ID 与命名

| 前缀 | 范围 | 文件 |
|---|---|---|
| `SPEC-SCOPE` | 能力边界、非目标、目标用户 | `spec/00-scope.md` |
| `SPEC-ROUTE` | 唯一技术路线与不采纳的路线 | `spec/01-route.md` |
| `SPEC-RT` | View 树、Owner、响应式图、MountPlan、事务、Store | `spec/02-runtime.md` |
| `SPEC-MACRO` | `view!` 语法、props、组件、错误呈现 | `spec/03-macro.md` |
| `SPEC-STYLE` | StyleIR、utility 词表、优先级、诊断 | `spec/04-style.md` |
| `SPEC-TEXT` | 段落/字形布局、TextEdit、IME、选区 | `spec/05-text.md` |
| `SPEC-RENDER` | 布局投影、绘制、合成、窗口、无障碍、可选 3D | `spec/06-render.md` |
| `SPEC-PLAT` | 平台后端与平台差异矩阵 | `spec/07-platform/*.md` |
| `SPEC-GATE` | 验收 Gate A0–D | `spec/08-gates.md` |
| `EVID-VAL` | 真实执行记录 | `evidence/validation.md` |
| `EVID-PROBE` | 探针能力与边界 | `evidence/probes.md` |
| `EVID-MATRIX` | 机器可读矩阵 | `evidence/matrix.json` |
| `CLAIM-*` | 单一事实声明台账 | `evidence/claim-register.md` |
| `ADR-*` | 架构决策记录 | `decisions/` |

条目写法：`### SPEC-RT-001 单一 UI 权威：一棵 View 树 + 一张响应式图`。在 `docs/` 内引用写作 `[SPEC-RT-001](spec/02-runtime.md#spec-rt-001)`（同目录内则用文件名），锚点为小写 ID。**禁止在别处复述该条内容超过一句**，只引用。

## 4. 文件头契约（每份文档必须有）

```yaml
---
doc-id: SPEC-RT
layer: spec            # spec | evidence | decisions | archive | product | governance
status: draft          # draft | normative | frozen | historical
date: 2026-11-18
owner: <角色名>
sources:               # 本文件吸收的来源（archive 路径或外部链接）
  - archive/research/runtime-and-rendering.md
replaces: []           # 本文件取代的旧文档
---
```

历史文档在归档时补 `status: historical` 与 `superseded-by:`，正文**不做任何改写**。

## 5. 目标目录树

```text
docs/
  README.md                     # 本文件：入口 + 治理
  product/
    positioning.md              # 目标用户、能力边界、稳定性承诺、不做清单
    roadmap.md                  # 版本路线、里程碑、对外承诺强度
  spec/
    00-scope.md  01-route.md  02-runtime.md  03-macro.md
    04-style.md  05-text.md   06-render.md   08-gates.md
    07-platform/{index,windows,macos,linux}.md
  evidence/
    index.md                    # 证据索引与复跑说明
    claim-register.md           # CLAIM-* 台账（唯一事实来源）
    validation.md               # EVID-VAL-*
    probes.md                   # EVID-PROBE-*
    matrix.json                 # EVID-MATRIX
  decisions/
    index.md                    # ADR 目录
    adr-0001-*.md ...
  archive/
    README.md                   # 归档说明：为什么在此、如何引用
    research/*.md               # 原 archive/research/*
    review/*.md                 # 原 archive/review/*
```

## 6. CLAIM 台账：唯一事实来源

`evidence/claim-register.md` 是全部事实声明的**唯一真相表**，一行一个可判真假的断言：

| ID | 断言 | 状态 | 证据 | 关联 Gate | 备注 |
|---|---|---|---|---|---|
| CLAIM-001 | rstml 能把目标节点形状解析为目标 AST | PROBE | EVID-VAL-02 | A0 | 仅 parse-only |

规则：

- 断言必须是**可判真假的单句**，不写“架构优雅”这类不可验证内容。
- `状态` 只能取五个词之一。
- `证据` 必须指向 `EVID-*` 条目、`SRC path:line`（可复核的源码事实）或外部官方文档 URL；没有证据就只能写 `SPEC` 或 `OPEN`。三种前缀的优先级：执行记录 `EVID-VAL-*` > 探针边界 `EVID-PROBE-*` > 源码事实 `SRC`。`SRC` 只能证明"代码此刻是这样"，不能证明运行行为。
- 被推翻的断言**不删除**，改状态为 `WITHDRAWN` 并在备注写明推翻依据。

## 7. 写作规则

1. 正文中文，术语保留英文（`retained tree`、`shaping`、`damage` 等）。
2. 规范条目用 **MUST / MUST NOT / SHOULD** 表达强度，不用“建议尽量”。
3. 每条规范都要有**失败模式**：不满足时会出现什么可观测的错误。
4. 版本 pin 只写在 `spec/01-route.md` 一处，其他文件引用 `SPEC-ROUTE-00X`。
5. 不写评审过程与投票，评审过程归 `archive/`；`decisions/` 只留结论与依据。
6. 不承诺未验证的平台/能力；没有真机结果的一律 `SPEC` 或 `OPEN`。

## 8. 本轮整理的工作顺序

1. 冻结入口与契约（本文件）。
2. 各领域从 archive 层抽取规范，写入 `spec/`，逐条标注来源与状态。
3. 事实类内容收拢进 `evidence/`，形成 `CLAIM-*` 台账。
4. 关键取舍沉淀为 `ADR-*`。
5. 原 `research/` 与 `review/` 两个目录整体移入 `docs/archive/`，只读保留。
6. 队长统一收敛：交叉引用检查、重复内容删除、`git commit`。

## 9. 整理快照（2026-11-18）

| 层 | 文件 | 规模 | 状态 |
|---|---|---|---|
| 规范 | `spec/` 12 篇（含 `07-platform/` 4 篇） | `SPEC-SCOPE/ROUTE/RT/MACRO/STYLE/TEXT/RENDER/PLAT/GATE` | `draft`，待冻结 |
| 证据 | `evidence/` 5 篇 | `EVID-VAL-01…19`、`EVID-PROBE-01…05`、`EVID-MATRIX` | `normative` |
| 事实台账 | `evidence/claim-register.md` | 100 条 `CLAIM-*` | `draft` |
| 决策 | `decisions/` 15 个文件 | `ADR-0001…0014` | `normative` |
| 产品 | `product/` 2 篇 | 定位、路线（M0–M4 ↔ A0/A1/B/C/D） | `draft` |
| 历史 | `archive/` 17 个文件 | 原 `research/` 7 篇 + `review/` 10 个 | `historical`，只读 |

收敛检查（本轮实际执行）：相对链接 **0 断链**（含 archive 内部）、`SPEC-*`/`ADR-*`/`CLAIM-*` 定义 **0 重复**、`spec/` 内 **0 处版本号复述**、**0 处** "已实现/已通过" 型未证据宣称。

> 快照只说明文档结构，不说明能力状态；能力状态一律以 `CLAIM-*` 台账为准。
