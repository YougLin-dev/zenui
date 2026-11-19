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
| [ADR-0020](adr-0020-write-during-compute-as-phase-violation.md) | `WriteDuringCompute` 不是独立变体：由 compute 相位的 `PhaseViolation` 表达，最外层兑现为 `Fatal::PhaseViolation` | `SPEC-RT-038`；`SPEC-GATE-A0-05` | normative |
| [ADR-0021](adr-0021-trybuild-pins-error-codes.md) | 错误码判据只由 trybuild `.stderr` 快照钉住；`compile_fail` doctest 只证明「不编译」，不得作 gate 证据 | `SPEC-GATE-A0-02`、`-03`；`SPEC-ROUTE-001`、`-004` | normative |
| [ADR-0022](adr-0022-counters-are-observation.md) | 计数器是观测而非第二权威：删除记账 API、`Tx::rows` 经 `RowView`/缓存取行、缓存键含 order revision、`QueryCache` 移出公共面 | `SPEC-RT-043`、`-044`、`-047`；`SPEC-GATE-A0-07`、`-08` | normative |
| [ADR-0023](adr-0023-row-set-projection-scope.md) | A0 的共享行集投影由缓存快照承担；可订阅的 `Memo<RowSet>` 随 `For` 在 A1 落地且只从 revision 对派生；粗粒度 membership 掩码以「结构写同时推进 order revision」为前提 | `SPEC-GATE-A0-07`；`SPEC-MACRO-011`；`SPEC-RT-043`；`ADR-0022` | normative |
| [ADR-0024](adr-0024-gate-close-predicate.md) | Gate `closed` 是证据层脚本计算的结构谓词（子句映射 + `notApplicable` + 证据五要素），不新增状态词；删除恒为 0 的 `criteriaVerified` | `SPEC-GATE-A0-01`…`-11`；`SPEC-GATE-*` 全部 | normative |
| [ADR-0025](adr-0025-a0-not-applicable-handover.md) | A0 七条无执行对象子句逐条移交 A1：click、多节点环、native/IME/resource、memo/per-row 边、For/订阅、input/toolbar、subscriber bucket | `SPEC-GATE-A0-03`、`-05`、`-06`、`-07`、`-09`、`-10`、`-11`；`SPEC-GATE-A1-01`…`-08` | normative |
| [ADR-0026](adr-0026-adr-0025-evidence-gap-errata.md) | 勘误 ADR-0025「相邻证据缺口」：三处缺口已由 #22 的 generation / prev / caches 夹具关闭；不改变任何决定 | `SPEC-GATE-A0-09`、`-10`、`-11` | normative |
| [ADR-0027](adr-0027-m0-exit-policy.md) | M0 出口 = A0 全部判据 `closed`（ADR-0024）+ 逐判据执行型证据下限 + 出口时点 workspace/CI 运行；`PROBE` 本身不构成通过；出口前必须完成逐义务见证链复核 | `SPEC-GATE-A0-01`…`-11` | normative |
| [ADR-0028](adr-0028-command-generation-handover.md) | A0 没有命令派发对象：`SPEC-RT-026` 的 command generation 以 `notApplicable` 移交 A1-07；M0 出口不以该子句为前提 | `SPEC-RT-026`；`SPEC-GATE-A1-07` | normative |
| [ADR-0029](adr-0029-m0-exit-record.md) | M0 出口记录：候选 `cbf7012a` 上 E1/E2/E3 成立；11 条 A0 判据闭合、58 条义务逐条点名、E3a/trace/E3b 已记录 | `SPEC-GATE-A0-01`…`-11` | normative |

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
| 2026-11-18 P2 compute 期写裁决（队长） | 0020 | 删除同名变体后的语义映射与名称边界 |
| 2026-11-18 P3 验收驳回·判据承载（队长） | 0021 | `compile_fail` 不校验错误码（实测），错误码只由 trybuild 快照钉住 |
| 2026-11-18 P3 验收驳回·计数权威（队长） | 0022 | 计数器与测试夹具不得活在运行时之外的平行实现里 |
| 2026-11-18 P3.2 收口·行集投影边界（队长） | 0023 | `Memo<RowSet>` 归 A1；粗粒度掩码的前提与撤销条件 |
| 2026-11-19 M0 收口·关闭谓词（队长） | 0024 | `closed` 由证据层计算；`criteriaVerified` 恒为 0，不再作论据 |
| 2026-11-19 M0 收口·无对象子句移交（队长） | 0025 | 七条 A0 子句的对象在 A0 不存在，逐条移交 A1 |
| 2026-11-19 M0 收口·ADR-0025 勘误（Release） | 0026 | 三处「有对象、缺夹具」由 #22 夹具关闭；ADR 正文不改 |
| 2026-11-19 M0 出口政策（产品负责人） | 0027 | A0 全判据闭合 + 执行型证据下限 + 出口时点集成状态；出口前见证链复核 |
| 2026-11-19 A0 command generation 移交（产品负责人） | 0028 | A0 无命令派发对象；移交 A1-07，出口不以该子句为前提 |
| 2026-11-19 M0 出口记录（产品负责人） | 0029 | 候选 `cbf7012a` 上 E1/E2/E3 成立；出口只解除 M1 宏 lowering 阻塞 |
| `docs/README.md` 四层治理契约 | 0011 | 文档层的唯一入口与状态词表 |

## 5. 维护规则

- 新增 ADR 必须有**已发生**的裁决；不要把"建议"或"计划"写成 ADR。计划属于 `product/roadmap.md` 或 `spec/` 的 `SPEC` 条目。
- 修改**决定**必须新开 ADR 并写 `supersedes`；旧文件只更新 `status`/`superseded-by`，正文不改。
- 每季度或每次 Gate 结论变化时，检查"撤销条件"是否已触发；触发的 ADR 必须在 `index.md` 标注。
- 本目录的文件不参与 `git commit` 之外的工具链；没有脚本依赖其格式。
