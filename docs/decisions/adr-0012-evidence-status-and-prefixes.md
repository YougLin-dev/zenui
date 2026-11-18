---
doc-id: ADR-0012
layer: decisions
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - docs/README.md §2、§6
  - docs/evidence/validation.md §0–§2
  - docs/evidence/index.md §4
  - docs/evidence/claim-register.md §0、§3
replaces: []
---

# ADR-0012：evidence 层的状态词与证据前缀

## 背景

治理契约最初规定：`SPEC` 只允许出现在 spec 层，`WITHDRAWN` 只允许出现在 decisions/archive。但同一契约
又要求 evidence 层的 `CLAIM-*` 台账使用五个状态词，并且"没有证据就只能写 `SPEC` 或 `OPEN`"、被推翻的
断言要标 `WITHDRAWN` 保留。两条规则直接冲突（审计 `C-01`）。

同时，`CLAIM-*` 大量依赖"源码第几行是事实"这类可复核证据。契约 §6 只允许 `EVID-*` 或 URL，没有给
源码静态事实留位置（审计 `C-18`）。`evidence/validation.md` §7 已经自行使用 `SRC-01…04` 记录四条源码
事实，`evidence/index.md` 也声明 `probes.md` 尚未创建且无 owner。必须裁定这三件事。

## 备选项

1. **严格分层**：evidence 层禁止 `SPEC`/`WITHDRAWN`，台账只记录有证据的断言；被推翻的断言移到 decisions。
2. **允许 evidence 层使用五词**（选中）：`SPEC` 表示"尚无证据"，`WITHDRAWN` 表示"已被推翻"；spec 层仍
   禁止 `VERIFIED`。
3. **新增第六个状态词**（例如 `REFUTED`）：需要改全部文档与工具，收益不足。
4. **台账取消状态，只存证据指针**：无法表达"这条事实还不可信"。

## 决定

1. **`SPEC` / `WITHDRAWN` 允许出现在 evidence 层**，含义分别是"该断言尚无证据"与"该断言已被推翻"；
   它们是**断言状态**，不是规范条文（D-01）。
2. **方向性禁令不变**：`VERIFIED` 只允许出现在 evidence 层；spec 层永不出现 `VERIFIED`
   （`docs/README.md` §2）。
3. **`SRC path:line` 作为与 `EVID-*` 并列的第三种证据前缀**，用于可在仓库内直接复核的源码/清单事实；
   `EVID-VAL-*` 仍是运行事实的唯一 ID（D-02）。
4. **`probes.md` 与 `EVID-PROBE-*` 归 CI 负责人**；在文件建立前，任何 `EVID-PROBE-*` 引用都视为 dangling，
   必须改引 `EVID-VAL-*` 或 `SRC`。
5. **编号稳定**：`EVID-VAL-01…19` 已由 `evidence/validation.md` §2 定义；新增只追加，不改旧 ID 的语义。

## 依据

- `docs/README.md` §2（本轮修订）：状态词表明确 `SPEC`/`WITHDRAWN` 在 evidence 作为断言状态；
  `VERIFIED` 仅 evidence。
- `docs/README.md` §6：`CLAIM-*` 台账必须使用五词、被推翻的断言保留并标 `WITHDRAWN`。
- `docs/evidence/validation.md` §2：`EVID-VAL-01…19` 的 ID 索引；§7：`SRC-01…04` 源码核对。
- `docs/evidence/index.md` §4：引用规则（断言只在 `claim-register.md` 落 `CLAIM-*`，spec/product 只引用）；
  §1：`probes.md` 尚未创建。
- `CLAIM-001`：rstml parse-only 是 `PROBE`，而非 `VERIFIED`；`CLAIM-025`/`-027`/`-028`：仓库外夹具为
  `PROBE`，不能升级为产品证据。
- `SPEC-SCOPE-021`：能力宣称必须能落到证据——与"evidence 可标 SPEC"并不冲突：标 `SPEC` 就是在承认
  还没有证据。

## 影响与撤销条件

**影响**

- `claim-register.md` 中的 10 条 `SPEC` 与 12 条 `WITHDRAWN` 现在合法；spec 引用它们时须写"引用状态"。
- 任何源码事实优先写 `SRC path:line`；若源码移动导致行号失效，先修引用，不得把旧断言升级为 `VERIFIED`。
- `evidence/index.md` 的"claim-register 尚未创建"与"probes 无 owner"两处描述已过期，由 CI 负责人更新；
  本 ADR 不修改他人文件。

**撤销条件（可判定）**

1. 若同一断言在 spec 与 evidence 出现**相反状态**，且无法用"spec 是要求、evidence 是可信度"解释，
   则本决定需要重审；
2. 若 `SRC path:line` 在干净检出上无法定位（文件不存在或行号漂移且无映射），则该类证据必须降级为
   `OPEN`，并重新讨论是否引入 `EVID-SRC` 文件；
3. 若 `probes.md` 建立后其 `EVID-PROBE-*` 与 `EVID-VAL-*` 编号发生冲突，必须以 `validation.md` 的
   运行记录为准并开新 ADR。

仅"严格分层看起来更干净"不构成撤销依据——它会让被推翻的断言重新变成不可见的幽灵。
