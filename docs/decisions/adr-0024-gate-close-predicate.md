---
doc-id: ADR-0024
layer: decisions
status: normative
date: 2026-11-19
owner: Crew 队长（架构）
sources:
  - docs/spec/08-gates.md
  - docs/README.md
  - docs/decisions/adr-0010-gate-alphabet-unique.md
  - docs/decisions/adr-0012-evidence-status-and-prefixes.md
  - docs/evidence/index.md
  - docs/evidence/matrix.json
  - scripts/check_docs.py
replaces: []
---

# ADR-0024：Gate 的关闭谓词由证据层计算，不新增状态词

## 背景

`08-gates.md` 的判据只回答三件事：通过判据、需要的证据、现在到哪一步。头两件是文本，第三件是一个
状态词（`SPEC` / `PROBE` / `OPEN` / `WITHDRAWN`）。「Gate 什么时候算完」目前只存在于手写汇总里，而
手写汇总已经漂移：A0 汇总行（`:48`）仍写着「`A0-02` 与 `A0-07`…`-11` 仍 `OPEN`」，但同一张表的逐行
状态（`:76`、`:81-85`）已经全部是 `PROBE`；`docs/evidence/validation.md:68` 也还停在旧结论。
`check_docs.py` 读取的是逐行最后一个单元格，所以**逐行状态才是事实来源**，汇总行是描述。这不是要
追究谁写错，而是说明：靠人维护的「还剩几条 OPEN」必然漂移，关闭判断需要谓词。

`matrix.json` 的 `coverage.criteriaVerified` 是另一个结构性误读点：`08-gates.md:19` 规定 spec 层不得
声明 `VERIFIED`，`docs/README.md` §2 与 `ADR-0012` 决定 2 重复同一条禁令。于是「42 条判据中
0 `VERIFIED`」不是测量结果，而是词汇规则的产物；`docs/README.md:84`、`docs/evidence/index.md:138`
把它写成覆盖率陈述，`ADR-0010:50-51` 更把它当作「Gate 未完成」的机器可读事实（该处计数
`1 PROBE / 41 OPEN` 也已过期，现为 `12 PROBE / 30 OPEN`，其中 A0 占 11 条、`SPEC-GATE-B-05` 占 1 条）。
这些引用都必须停止。

本条只定义「关闭」这个**结构谓词**；「`PROBE` 级证据是否足以出口 M0」是产品负责人的政策判断，需其
另行裁决，本条不回答、也不引用任何未落纸的口头结论。无执行对象子句的逐条移交由 `ADR-0025` 提供；
`08-gates.md` §9 要求放宽留 ADR 说明「哪条证据不支持原判据」。

## 备选项

1. **新增状态词 `CLOSED`**：否决。状态词表在 `docs/README.md` §2（`08-gates.md:19` 同步）固定为
   五个，`closed` 与它们不正交：一条 `PROBE` 判据可以已关闭，也可以未关闭；把它塞进状态列会逼所有
   工具学会一条并不存在的阶梯。
2. **在 `matrix.json` 手填 `closed`**：否决。手填布尔会与树漂移，这正是 checker 存在的理由。
3. **保留 `criteriaVerified`，在文档里解释它恒为 0**：否决。解释不会阻止它继续被当作证据不足的论据。
4. **谓词由生成步骤从义务清单计算、checker 只读重算；删除 `criteriaVerified`，改为派生
   `criteriaClosed`**（选中）：可判定、不可手填，且不再制造「0 = 没证据」的假信号。

## 决定

1. **判据 `closed` 的定义。** `closed` 是布尔字段，不是状态词；当且仅当以下三条同时成立：
   - **义务齐全且可解析。** `criteria[].obligations[]` 按列分为 `source: criterion`（通过判据列）与
     `source: evidence`（需要的证据列）。每一列按全角分号 `；` 切段；该列的 obligations 按顺序拼接
     后，**去掉空白与 `；`、保留反引号与强调符号**，必须等于原列文本做同样处理后的结果。义务可以比
     分号段更细（例如把「native 树、id 映射、focus」拆成三段），但不得跳过任何字符，也不得只挑好
     做的几段。每条义务的 `ref` 必须可解析：要么是仓库测试清单里存在的 `#[test]` 函数名（由
     `cargo test -- --list` 或等价的源码清单生成），要么是 `docs/evidence/validation.md` 里逐字出现
     的命令。
   - **无对象义务已按 §9 标记，且目标已接受。** 在 A0 没有执行对象的义务标 `kind: notApplicable`、
     `adr: ADR-00xx`，并带 `acceptedBy: { criterion, text }`——目标是 A1 判据 ID，`text` 是该判据的
     MUST/证据列必须新增的义务原文。checker 必须确认 `text` 已出现在目标判据对应列的文本里（去空白
     比对）；没有 `acceptedBy` 或其文本尚未落进 A1 行时，该义务不算关闭。`ADR-0025` 是当前清单。
   - **证据够格。** 每条已映射义务的 `evidence[]` 至少引用一个 `exit == 0` 的 run；该 run 必须有
     host、command、可定位的 commit（run 级字段，或矩阵顶层 `commit` / `commitNote` 指向的 commit），
     以及摘要（`proves`，或 trace 类 run 的 `traceSha256`）。
     显式 expected-failure 的义务必须在 `obligations[].expectedFailure` 声明并指向对应 run。只靠
     `SRC` 引用的义务不满足本条。`runs[]` 的字段目前不齐（48 条中 11 条无 commit、4 条无 exit、
     21 条无 tests、7 条无 proves）；被 `closed` 依赖的 run 必须先补齐或替换。
   `closed` 不得为真于 `OPEN` / `SPEC` / `WITHDRAWN` 判据；即 `closed ⊆ PROBE`，反向不成立。
2. **Gate 的聚合。** Gate `closed` 当且仅当其下**全部判据** `closed`。Gate 的 `status`（汇总行的
   `OPEN`/`PROBE`/…）保持不变，不由谓词改写；两者并列，允许「gate `OPEN`、`closed: false`」与
   「gate `PROBE`、`closed: false`」。
3. **计算权归生成步骤，校验权归 checker。** `matrix.json` 增加 `criteria[]`：每条判据一行，含
   `id`、`status`（取自 `08-gates.md` 逐行状态单元格）、`obligations[]`、`evidence[]`、`closed`。
   `gates[].closed` 由 `criteria[]` 聚合。生成步骤可以是 `check_docs.py --write` 或独立脚本；无论哪种，
   **CI 只运行只读校验路径**，校验失败即失败，不得在 CI 里改写文件。`check_docs.py` 重算义务覆盖、
   `ref` / `adr` / `acceptedBy` 解析、证据门、`criteria[].closed` 与 `gates[].closed`，任一项不一致
   即失败。
4. **删除 `coverage.criteriaVerified`，改为派生 `coverage.criteriaClosed`。** 状态拆分仍是四个 spec
   状态词的**划分**：`criteriaSpec + criteriaProbe + criteriaOpen + criteriaWithdrawn = criteriaTotal`；
   `criteriaClosed` 单独报告，必须满足 `criteriaClosed ≤ criteriaProbe`，不得混进划分。入口文档的句子
   写成「12 `PROBE`（其中 N `closed`）、30 `OPEN`」这类形式，检查器分别解析两个数。spec 层的
   `VERIFIED` 仍须被显式断言为 0，但不再以「覆盖率」字段出现。`README.md:84` 与
   `docs/evidence/index.md:138` 的措辞随之改为 `closed` 口径。
5. **「0 VERIFIED」不再是论据。** 任何文档、评审结论或状态说明不得以「判据 0 `VERIFIED`」推断证据
   不足；判据是否可用只看 `closed` 与它引用的证据条目。`ADR-0010:50-51` 的那句计数示例属于历史快照，
   其用法由本条停止（计数本身也已过期）；ADR 正文不可就地修订（`index.md` §5），勘误按「影响」执行。
6. **本条不改任何判据的状态，也不裁决 M0 出口。** `PROBE` 是否足以出口 M0 须由产品负责人另行裁决；
   谓词只回答「这条判据的结构条件是否齐备」。

## 依据

- `08-gates.md:19`：「状态词只用 `SPEC` / `PROBE` / `OPEN` / `WITHDRAWN`。`VERIFIED` 只在
  `evidence/` 层声明」。
- `08-gates.md` §9（`:174-179`）：判据只能加严或依据证据放宽；放宽必须留 ADR 说明哪条证据不支持原
  判据；改名/拆分保留 old → new 映射。
- `08-gates.md` §0（`:21-25`）：0 tests 的 target 不算通过；缺 runner = blocked，不复制结果；判据
  必须能在干净机器上复跑。
- `08-gates.md:48` 与 `:76`、`:81-85`：汇总行与逐行状态的差异；`check_docs.py` 以逐行最后一个单元格
  为准。`coverage.criteriaProbe=12` = A0 的 11 条加 `SPEC-GATE-B-05`。
- `docs/README.md` §2（`:37-49`）：状态词表只用五个；`:43`、`:47` 的 `VERIFIED` 禁令。
  `docs/README.md:121`（§4）：`spec/` 全部为 `normative`，规范层定义要求而非证明已实现。
- `ADR-0012` 决定 2：`VERIFIED` 只允许出现在 evidence 层，spec 层永不出现。
- `docs/evidence/index.md:100`：一次复跑最少记录 commit、主机、命令、退出码、输出摘要。
- `SRC docs/evidence/matrix.json:962`：`coverage.criteriaVerified` 当前为 0；`runs[]` 的字段是
  `host` / `command` / `exit` / `tests` / `proves` / `doesNotProve`，其中 commit、exit、tests、proves
  在部分 run 上缺失（11 / 4 / 21 / 7 条）。
- `SRC scripts/check_docs.py:465-523`：coverage 拆分与逐 gate 计数；`:498-508` 的 projected
  `VERIFIED` 比对；`:644-691` 与 `:77` 的派生数字与 `COVERAGE_LABEL`；`:196-240` 的 ID 解析
  （只扫 Markdown，JSON 引用不在其中）。
- `EVID-MATRIX`：`matrix.json` 是 `validation.md` 与 `08-gates.md` 的机器投影。
- `ADR-0010`：Gate 字母表唯一、blocked ≠ pass；本条不新增状态或字母。

## 影响与撤销条件

**影响**

- 不改变任何判据的当前状态。逐行状态是事实来源；A0 汇总行（`:48`）与 `validation.md:68` 的陈旧描述
  由 gate owner 在状态变更里同步，本条不改 spec/evidence。
- 证据层变更清单（由 Release Engineer 执行，本 ADR 不改）：
  1. `matrix.json`：新增 `criteria[]`（`obligations[]` / `evidence[]` / `closed`）与 `gates[].closed`；
     删除 `coverage.criteriaVerified`，新增 `coverage.criteriaClosed` 与四个状态词计数。
  2. `check_docs.py` 新增谓词校验：按列切分与拼接比对、`ref` 解析（测试清单/命令表）、`adr` 与
     `acceptedBy` 解析、证据门、`closed` 与 gate 聚合一致性。
  3. `check_matrix_coverage`（`:465-523`）：状态投影扩展为四个词、显式断言 `VERIFIED == 0`、增加
     `criteriaClosed ≤ criteriaProbe` 与逐 gate 计数一致性；删除 projected `VERIFIED` 字典项。
  4. `check_derived_numbers`（`:644-691`）与 `COVERAGE_LABEL`（`:77`）：期望元组含 closed，标签解析
     「N `PROBE`（其中 M `closed`）」形式，错误文案同步。
  5. **单独的 JSON 引用扫描**：不能复用 `defined_ids`（它同时收集定义，会把引用自证）；须从
     Markdown/frontmatter/文件名收集 ADR 定义，再校验 `obligations[].adr` 与 `acceptedBy.criterion`。
  6. 数据修复：被 `closed` 依赖的 run 补齐 commit / exit / tests / proves；否则该义务不算关闭。
  7. `README.md:84`、`docs/evidence/index.md:138` 改为 `closed` 口径。
  8. `ADR-0010:50-51` 的陈旧用法按 `index.md` §5 由 gate owner 新开勘误或 superseding ADR。
- `ADR-0025` 提供七条 `notApplicable` 的依据与 `acceptedBy` 目标；没有它的 ADR 引用，任何
  `notApplicable` 标记都非法。
- 谓词刻意不含证据强度阈值：`closed` 只要求记录完整、可解析、exit 0；「证据够不够支撑 M0」仍是
  产品判断。

**撤销条件（可判定）**

1. 若某条判据的义务既无法映射到任何夹具/命令，又不属于「A0 无执行对象」（无 `notApplicable` +
   ADR + `acceptedBy` 且目标文本已落地），该判据必须记 `closed: false`；写成 `closed: true` 视为谓词
   违规，checker 必须失败。
2. 若某个 `notApplicable` 的对象在 A0 出现（点击派发、多节点依赖 API、native/IME/resource、行集
   订阅者、`For`/订阅、input/toolbar、subscriber bucket 之一），该标记必须由新 ADR 撤销，义务回到
   A0 判据；在撤销前 `closed` 不得据此为真。
3. 若将来需要子句级的中间态（例如同一判据下 3/5 义务齐备），`closed` 仍是全或无；部分进度只能体现
   在义务清单里，不得把 `closed` 改成百分比或模糊值。
