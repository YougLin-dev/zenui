---
doc-id: ADR-INDEX
layer: decisions
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - archive/review/FINAL.md §0、§8
  - archive/research/CONCLUSION.md
  - archive/research/stack-decisions.md
  - docs/README.md
  - docs/spec/01-route.md
  - docs/spec/08-gates.md
replaces: []
---

# 决策层索引（ADR）

> `decisions/` 只回答一个问题：**为什么这样定，否掉了什么。**
> "要建什么"归 `spec/`，"跑过什么"归 `evidence/`。ADR 不是规范条文，不能替代 `SPEC-*`；实现者仍须以满足 `spec/` 为准。
>
> 本层只记录**已经发生的裁决**。整理者没有在此新增技术选择；每条 ADR 的来源、裁决者与日期都可追溯。
> 记录：顶级研究员（疑难攻坚）｜裁决：Crew 队长（架构）｜日期：2026-11-18

## 1. 读法

| 问题 | 看哪里 |
|---|---|
| 现在必须建什么？ | `spec/`（`SPEC-*`） |
| 实际跑过什么、结果如何？ | `evidence/`（`EVID-VAL-*`、`CLAIM-*`） |
| 为什么不是另一条路？ | 本目录（`ADR-*`） |
| 过去怎么想的？ | `archive/`（只读，无权威） |

三条引用纪律：

1. ADR 的"依据"必须引用 `CLAIM-*`、`EVID-*` 或 `SPEC-*`；**不得只写"评审认为"**。
2. 若 ADR 与 `spec/` 冲突，以 `spec/` 为准，并开一条新 ADR 说明为何规格变化；不得改历史 ADR 的"决定"。
3. 被推翻的 ADR 不删除：`status` 改为 `superseded`，在文件头写 `superseded-by: ADR-00xx`。

## 2. ADR 目录

| ADR | 决定了什么 | 影响的 `SPEC-*` | 状态 |
|---|---|---|---|
| [ADR-0001](adr-0001-single-view-tree.md) | 单一 View 树 + 单一响应式图 + 一次性 `MountPlan` 是唯一"状态 → 界面"路径 | `SPEC-ROUTE-010`；`SPEC-RT-001`、`-010`…`-012`；`SPEC-SCOPE-013` | normative |
| [ADR-0002](adr-0002-reject-vdom-elm.md) | 否决 VDOM / Elm 全量 `view()` 重建作为默认刷新模型 | `SPEC-ROUTE-020`（WITHDRAWN）；`SPEC-RT-010` | normative |
| [ADR-0003](adr-0003-reject-flutter-three-trees.md) | 否决 Flutter 式 Element / RenderObject 三树，布局与绘制从同一棵树投影 | `SPEC-ROUTE-021`（WITHDRAWN）；`SPEC-RENDER-001` | normative |
| [ADR-0004](adr-0004-vello-backend-roles.md) | sparse-strips 为生产候选；CPU 为 reference/fallback；Classic 仅 probe | `SPEC-ROUTE-001`、`-002`、`-027`；`SPEC-RENDER-007`、`-010` | normative |
| [ADR-0005](adr-0005-tailwind-whitelist.md) | Tailwind 采用 pinned registry 白名单，不做运行时 CSS 引擎 | `SPEC-SCOPE-020`；`SPEC-STYLE-001`、`-033`、`-034`、`-040`、`-042`；`SPEC-MACRO-013`、`-014`、`-017` | normative |
| [ADR-0006](adr-0006-2d-trunk-3d-leaf.md) | 2D 是主干，3D 是布局叶节点；3D 不得创建第二套窗口/呈现 | `SPEC-SCOPE-015`；`SPEC-RENDER-019`…`-024` | normative |
| [ADR-0007](adr-0007-ordinary-inline-wins.md) | ordinary author inline 在 hover=false/true 均胜同字段普通 class；condition 不是排序键 | `SPEC-STYLE-010`…`-014`；`SPEC-ROUTE-023`（WITHDRAWN） | normative |
| [ADR-0008](adr-0008-filtered-out-default-dispose.md) | filtered-out 默认 dispose UI owner、保留 row model；编辑状态恢复必须显式 opt-in | `SPEC-RT-024`、`-026`；`SPEC-ROUTE-026`（WITHDRAWN） | normative |
| [ADR-0009](adr-0009-version-pin-single-source.md) | 版本 pin 只在 `SPEC-ROUTE-001` 定义；升级必须走 ADR；归档内 pin 一律失效 | `SPEC-ROUTE-001`、`-003`、`-004`；`SPEC-SCOPE-021` | normative |
| [ADR-0010](adr-0010-gate-alphabet-unique.md) | Gate 字母表唯一化为 `A0/A1/B/C/D`；blocked ≠ pass；判据只能加严或依证据放宽 | `SPEC-GATE-A0-*`…`SPEC-GATE-D-*` | normative |
| [ADR-0011](adr-0011-docs-four-layers.md) | 文档四层治理：spec / evidence / decisions / archive；一个事实只定义一次 | `SPEC-SCOPE-021`；全部 `SPEC-*` 的引用纪律 | normative |
| [ADR-0012](adr-0012-evidence-status-and-prefixes.md) | evidence 层允许 `SPEC`/`WITHDRAWN` 作为断言状态；接受 `SRC path:line` 为第三种证据前缀 | `SPEC-SCOPE-021`；`CLAIM-*` 台账规则 | normative |
| [ADR-0013](adr-0013-archive-read-only.md) | 归档只读、不重写；版本 pin / Gate / 完成度描述一律失效；已知过期物单列 | `SPEC-ROUTE-001`、`-002`；`SPEC-STYLE-011`、`-012` | normative |
| [ADR-0014](adr-0014-baseline-defects-are-gate-criteria.md) | baseline / intrinsic / synthesis 缺陷写成 Gate 硬判据，不以"探针绿"关闭 | `SPEC-GATE-B-01`…`-03`；`SPEC-TEXT-001`…`-003`、`-006` | normative |
| [ADR-0015](adr-0015-render-ownership-splits.md) | 渲染线三处归属切分：生命周期归 `SPEC-RT`、声明形态归 `SPEC-STYLE`、坐标单位归 `SPEC-TEXT` | `SPEC-RENDER-011`、`-016`；`SPEC-TEXT-015`；`SPEC-RT-024`、`-026`；`SPEC-STYLE-002`、`-004`、`-052`、`-055` | normative |
| [ADR-0016](adr-0016-release-order.md) | 释放顺序：Owner 后序（子先父）+ 同 owner 逆注册 LIFO；dispose 幂等；cleanup 中禁止 mount/batch/flush | `SPEC-RT-021`、`-022`、`-052` | normative |
| [ADR-0017](adr-0017-p2-runtime-authority-and-transaction-shape.md) | P2 形状：解析权归权威、值暂存 + 发布屏障、`RuntimeId` 归属与脱敏、A0-06 用 core 内零依赖 `LayoutMap` | `SPEC-RT-002`、`-005`、`-006`、`-033`、`-038`、`-050`…`-053`；`SPEC-GATE-A0-03`、`-06` | normative |
| [ADR-0018](adr-0018-docs-language-scope.md) | 语言分工：分支/commit/PR/代码/标识符一律英文；`docs/` 正文沿用中文；一个文件一种语言；不就地翻译冻结层 | 无（约束文档与产物的语言，不改写 `SPEC-*` 要求） | normative |
| [ADR-0019](adr-0019-failure-classification-and-phase-typing.md) | 失败按检测点分类；写 API 阶段违规返回值错误并毒化事务；commit 阶段违规返回致命错误 | `SPEC-RT-038`、`-053`、`-057`；`SPEC-GATE-A0-04`、`-05`、`-06` | normative |

## 3. 每条的固定结构

每条 ADR 只包含五段，顺序固定：

1. **背景**：当时必须解决什么问题；哪些事实已确认。
2. **备选项**：真正被考虑过的路线，以及为什么没有选它。
3. **决定**：已经作出的裁决，逐条编号。
4. **依据**：`CLAIM-*` / `EVID-*` / `SPEC-*` 的具体条目；不使用"评审认为"作为依据。
5. **影响与撤销条件**：对实现与文档的约束；撤销条件必须是**可判定**的观测（命令、Gate 判据、固定 workload），不能是"以后再看"。

## 4. 决策来源

| 来源 | 覆盖的 ADR | 说明 |
|---|---|---|
| `archive/review/FINAL.md` §0、§8 主控裁决 | 0001–0008、0010 | 只作为历史来源；现行约束以 `spec/` 为准 |
| `archive/research/stack-decisions.md`、`CONCLUSION.md` | 0004、0005、0006、0009 | 版本与生态取舍的论证过程 |
| 2026-11-18 队长裁决 D-01…D-06 | 0012、0013、0014（0010 同步） | 证据状态词、归档只读、Gate 唯一化、baseline 硬判据 |
| 渲染线三处归属切分（队长裁决） | 0015 | 生命周期 / 声明形态 / caret 单位的唯一 owner |
| 2026-11-18 P2 形状评审（API 设计 → 队长采纳） | 0017 | 解析权、事务机制、`RuntimeId` 归属、A0-06 `LayoutMap` |
| 2026-11-18 P2 失败分类裁决（队长） | 0019 | 检测点规则、阶段违规的值通道与毒化 |
| `docs/README.md` 四层治理契约 | 0011 | 文档层的唯一入口与状态词表 |

## 5. 维护规则

- 新增 ADR 必须有**已发生**的裁决；不要把"建议"或"计划"写成 ADR。计划属于 `product/roadmap.md` 或 `spec/` 的 `SPEC` 条目。
- 修改**决定**必须新开 ADR 并写 `supersedes`；旧文件只更新 `status`/`superseded-by`，正文不改。
- 每季度或每次 Gate 结论变化时，检查"撤销条件"是否已触发；触发的 ADR 必须在 `index.md` 标注。
- 本目录的文件不参与 `git commit` 之外的工具链；没有脚本依赖其格式。
