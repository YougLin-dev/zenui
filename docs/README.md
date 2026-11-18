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

禁止使用的模糊措辞：“基本完成”“大部分支持”“接近生产”“应该可以”。需要表达覆盖度时，给出**分子/分母与来源**（例如：`2/42 条 Gate 判据处于 PROBE 级，来源 EVID-MATRIX.coverage`）。

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

条目写法：`### SPEC-RT-001 单一 UI 权威：一棵 View 树 + 一张响应式图`。在 `docs/` 内引用写作 `[SPEC-RT-001](spec/02-runtime.md)`（同目录内则用文件名），**链接只指向文件，条目用 ID 文本定位**——标题里带正文，GitHub 生成的锚点不是裸 ID，写 `#spec-rt-001` 会断。**禁止在别处复述该条内容超过一句**，只引用。

### 3.1 主题索引：先查这里，再进文档

| 主题 | 当前权威文档 | 负责人 | 证据到哪里查 |
|---|---|---|---|
| 能力边界、目标用户、非目标 | [spec/00-scope.md](spec/00-scope.md) | 框架产品经理 | `SPEC-SCOPE-*`；现无生产 runtime（`CLAIM-039`） |
| 技术路线、否决的路线、版本 pin | [spec/01-route.md](spec/01-route.md)、`decisions/ADR-0001…0007` | 框架产品经理 / 架构 | `SPEC-ROUTE-*`，pin 只在 `SPEC-ROUTE-001` |
| 运行时：View 树、Owner、MountPlan、事务、Store | [spec/02-runtime.md](spec/02-runtime.md) | 顶级 Rust 开发工程师 | `EVID-PROBE-02` 覆盖去重；事务原子性有反例（`EVID-VAL-08`，`SPEC-GATE-A0-04` 红） |
| `view!` 语法、props、组件、错误呈现 | [spec/03-macro.md](spec/03-macro.md) | API 人体工学师 | `EVID-PROBE-01`，仅 parse-only |
| StyleIR、utility 词表、优先级、诊断 | [spec/04-style.md](spec/04-style.md) | 顶级 Rust 开发工程师 | `EVID-PROBE-03`（utility family 覆盖为分子/分母，见条目） |
| 文本/字形布局、TextEdit、IME、选区 | [spec/05-text.md](spec/05-text.md) | 渲染与 GPU 抽象负责人 | `EVID-PROBE-04`，三处缺陷已证实 |
| 布局投影、绘制、合成、窗口、无障碍、3D | [spec/06-render.md](spec/06-render.md) | 渲染与 GPU 抽象负责人 | `EVID-PROBE-04`、`EVID-VAL-15/16` |
| 平台后端（Windows/macOS/Linux） | [spec/07-platform/index.md](spec/07-platform/index.md) | 各平台工程师 | 三平台均无真机证据，槽位全 `OPEN` |
| 验收 Gate A0–D | [spec/08-gates.md](spec/08-gates.md) | 测试与 CI/发布负责人 | 42 条判据中 0 `VERIFIED`、2 `PROBE`、40 `OPEN`（`EVID-MATRIX`） |
| 事实断言（某句到底算不算数） | [evidence/claim-register.md](evidence/claim-register.md) | 顶级研究员 | 100 条 `CLAIM-*`，逐条带状态与证据 |
| 真实执行记录与复跑命令 | [evidence/validation.md](evidence/validation.md)、[evidence/index.md](evidence/index.md) | 测试与 CI/发布负责人 | `EVID-VAL-01…22` |
| 探针能证明什么、不能证明什么 | [evidence/probes.md](evidence/probes.md) | 测试与 CI/发布负责人 | `EVID-PROBE-01…05`，每条含证伪边界 |
| 产品定位与版本路线 | [product/positioning.md](product/positioning.md)、[product/roadmap.md](product/roadmap.md) | 框架产品经理 | 路线 `M0–M4` ↔ Gate `A0/A1/B/C/D` |
| 历史结论与已撤销方案 | [archive/README.md](archive/README.md) | Crew 队长（架构） | 只读；`§3` 权威替代表、`§4` 已知过期物 |
| 文档本身怎么改 | 本节、[§4 文件头](#4-文件头契约每份文档必须有)、`templates/` | Crew 队长（架构） | `scripts/check_docs.py`（CI 强制） |
| 产物与文档各用什么语言 | [decisions/adr-0018-docs-language-scope.md](decisions/adr-0018-docs-language-scope.md) | 测试与 CI/发布负责人 | 无（语言无法机械校验，靠评审执行） |

新增主题时必须同时在 `spec/`/`evidence/` 落一条并更新本表；只改表不改文档、或只加文档不改表，都算未完成。

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

归档文件**不补 frontmatter**：原文逐字保留，失效声明与权威替代关系统一维护在 `archive/README.md` §3–§4（见 `ADR-0013`）。

`status` 四个取值的判定规则（按"冲突时谁说了算"区分，不是按完成度）：

| 状态 | 含义 | 与实现冲突时 | 能否作为验收依据 |
|---|---|---|---|
| `draft` | 草稿，仍在修订 | 以实现为准，回头改文档 | 不能 |
| `normative` | 现行规范，对实现有约束力 | 以文档为准，实现即为缺陷；改文档需走 `ADR-*` | 能 |
| `frozen` | 冻结定稿 | 只能新开 ADR supersede，不能就地修订 | 能，变更门槛最高 |
| `historical` | 历史归档 | 无权威，仅供追溯 | 不能 |

因此 `spec/` 全部为 `normative`：规范层的任务是"定义要求"，不是"证明已实现"；实现进度一律看 `CLAIM-*` 与 `SPEC-GATE-*`。

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
  templates/
    README.md                   # 四类模板与适用场景
    adr.md  spec-entry.md  evidence-entry.md  claim-row.md
  archive/
    README.md                   # 归档说明：为什么在此、如何引用
    research/*.md               # 原 archive/research/*
    review/*.md                 # 原 archive/review/*
```

**刻意不设的目录**（与初始规划相比）：`research/`（未定内容以 `draft` 的 spec 条目或 `OPEN` 的 `CLAIM-*` 落盘，避免出现第二权威）、`spikes/`（探针报告并入 `evidence/probes.md`，因为探针同时是证据）、`bench/`（当前没有可发布的性能基线，建空目录只会被误读为"已有基线"）、`schema/`（文件头契约在 §4，机器校验由 `scripts/check_docs.py` 执行，静态说明文件会与脚本分叉）。

## 6. CLAIM 台账：唯一事实来源

`evidence/claim-register.md` 是全部事实声明的**唯一真相表**，一行一个可判真假的断言：

| ID | 断言 | 状态 | 证据 | 关联 Gate | 备注 |
|---|---|---|---|---|---|
| CLAIM-0NN | 一句可判真假的断言 | PROBE | EVID-VAL-NN | A0 | 备注 |

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
7. 新增文档从 `templates/` 对应模板起手，并在 §3.1 主题索引登记；两者缺一视为未完成，`scripts/check_docs.py` 在 CI 上把关其余可机械检查的部分。

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
| 规范 | `spec/` 12 篇（含 `07-platform/` 4 篇） | `SPEC-SCOPE/ROUTE/RT/MACRO/STYLE/TEXT/RENDER/PLAT/GATE` | `normative`（2026-11-18 整体冻结为现行规范） |
| 证据 | `evidence/` 5 篇 | `EVID-VAL-01…22`、`EVID-PROBE-01…05`、`EVID-MATRIX` | `normative` |
| 事实台账 | `evidence/claim-register.md` | 100 条 `CLAIM-*` | `draft` |
| 决策 | `decisions/` 20 个文件 | `ADR-0001…0019` + `index.md` | `normative` |
| 产品 | `product/` 2 篇 | 定位、路线（M0–M4 ↔ A0/A1/B/C/D） | `draft` |
| 历史 | `archive/` 18 个文件 | `README.md` + 原 `research/` 8 篇 + `review/` 9 个 | 逐字只读（无 frontmatter，见 `ADR-0013`） |

收敛检查（本轮实际执行）：相对链接 **0 断链**（含 archive 内部）、`SPEC-*`/`ADR-*`/`CLAIM-*` 定义 **0 重复**、`spec/` 内 **0 处版本号复述**、**0 处** "已实现/已通过" 型未证据宣称。

> 快照只说明文档结构，不说明能力状态；能力状态一律以 `CLAIM-*` 台账为准。
