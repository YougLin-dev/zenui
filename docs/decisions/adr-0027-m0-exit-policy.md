---
doc-id: ADR-0027
layer: decisions
status: normative
date: 2026-11-19
owner: 框架产品经理（技术型，10 年+）
sources:
  - docs/spec/08-gates.md
  - docs/decisions/adr-0024-gate-close-predicate.md
  - docs/decisions/adr-0025-a0-not-applicable-handover.md
  - docs/product/roadmap.md
  - docs/evidence/index.md
  - docs/evidence/matrix.json
  - docs/README.md
replaces: []
---

# ADR-0027：M0 出口政策——A0 全判据结构闭合，且每条判据有执行型证据

## 背景

`ADR-0024` 把「Gate 什么时候算完」定义成证据层可计算的 `closed` 谓词，并在决定 6 明确把「`PROBE` 级证据是否足以出口 M0」留给产品负责人；`ADR-0025` 处理了 A0 中没有执行对象的子句。两者合起来回答了「一条判据的结构条件是否齐备」，但没有回答「M0 这个里程碑能不能出、按什么判」。

`docs/product/roadmap.md:45` 写的出口是「`SPEC-GATE-A0` 全项通过」。「全项通过」在现行文档里没有可判定的定义：它可能被读成「11 条判据的 MUST 文本都成立」「全部达到 `PROBE`」「全部 `closed`」，也可能被读成「全部 `VERIFIED`」。最后一种读法必须排除：`docs/README.md` §2 规定 `VERIFIED` 只出现在 evidence 层，`08-gates.md:19` 同步；`ADR-0024` 决定 5 已停止把「0 `VERIFIED`」当作证据不足的论据。因此出口政策不能建立在状态词升级上，只能建立在 `closed` 与证据质量上。

三个已核实的前提（撰写时，2026-11-19）：

1. 「0 `VERIFIED`」是词汇规则的产物，不是测量结果；A0 的 11 条判据当前全部是 `PROBE`，另一条 `PROBE` 是 `SPEC-GATE-B-05`，不属于 M0。
2. 谓词已在证据层落地：`matrix.json` 有 `criteria[]` 与 `gates[].closed`，E1 可计算；本条只写条件，不写当前满足情况。结构结果不等于断言成立——逐义务见证链复核仍是出口前置。
3. A0 的方法列是「编译期 + 运行时 trace」（`08-gates.md:48`），且 11 条判据的 MUST 列每一条都含运行时可观测量（trace、提交次数、计数、回滚逐字段比对、身份保留、churn 回基线等）。编译期证据是必要条件的一部分，但不能单独支撑任何一条 A0 判据。

不裁定的后果：M1 的准入（`roadmap.md:37`：M0 未闭合前不得开始 M1 的宏 lowering 扩展）会继续靠口头判断，而 `PROBE` 与 `closed` 的差别会被当成「差一点」。这正是本条要消除的模糊。

## 备选项

1. **只要求 E1（全部判据 `closed: true`），不加证据质量下限**：否决。`ADR-0024` 决定 1 的证据门不限制 run 的 `kind`，`compile-fail` 与 `lint` 类运行也能让一条义务「有证据」；一条以运行时行为为 MUST 的判据可以只靠类型层快照关闭。M0 的交付物是「checked core」的运行闭环，不是能编译的骨架。
2. **要求 A0 全部升级为 `VERIFIED`**：否决。spec 层永不出现 `VERIFIED`（`docs/README.md` §2、`08-gates.md:19`、`ADR-0012` 决定 2），把它作为出口条件等于要求一个词汇表不允许的状态；且「0 `VERIFIED`」不携带证据信息（`ADR-0024` 决定 5）。
3. **按比例出口（如 9/11）或按 gate 汇总行判断**：否决。`ADR-0024` 决定 2 规定 gate `closed` 是全称；汇总行的 `status` 是描述而非事实来源（`ADR-0024` 背景）。部分出口会把未关闭判据的风险带进 M1。
4. **产品签发一次性豁免（waiver）**：否决。`08-gates.md` §9 只规定「放宽必须留 ADR」；具体机制是 `ADR-0024` 决定 1 的 `notApplicable` + `acceptedBy` 与 `ADR-0025` 的逐条移交。产品豁免会变成五词状态表之外的第六个状态，而且没有可复核的撤销条件。
5. **手工维护「必须运行时验证」的判据子集**：否决。A0 的 11 条判据全部含运行时 MUST，枚举与全称等价；而 §9 允许加严新增判据，手维护清单会漏掉新增项。全称规则不会漂移。
6. **E1 + 逐判据执行型证据下限 + 出口时点集成状态（选中）**：三条条件全部可判定，全部从 `matrix.json`、候选/记录 commit 的 git 关系与候选 commit 上的 spec/workflow 文本机械计算，不引入新状态词。

## 决定

1. **定义域与时点。** M0 出口只裁决**出口时点** `docs/spec/08-gates.md` §2 中定义的全部 A0 判据（撰写时为 `SPEC-GATE-A0-01`…`-11`）。判据集合以**候选 commit** 上的 spec 文本为准，不以本条撰写时的常量 11 为准。若 §9 的加严、拆分或改名改变了集合，必须先有新 ADR 给出 old → new 映射与新的分母。被合法撤销（`WITHDRAWN`）的判据只有满足两个条件时才不计入分母：同一 ADR 明确将其移出 A0 出口集合，并同时把它从 `08-gates.md` §2 删除（使其不再进入 `criteria[]` 与 `gates[].closed` 的聚合），附 old → new 映射与依据；只把状态改成 `WITHDRAWN` 而仍留在 §2 的判据仍计入集合，且因 `closed` 不得为真而阻止出口。`SPEC-GATE-B-05` 等其它 `PROBE` 项不属于 M0。出口不新增、不改写任何状态词。

2. **E1：结构闭合。** 对定义域中的每一条判据：`criteria[].closed: true`；且 `gates[]` 中 `SPEC-GATE-A0` 的 `closed: true`（聚合对象即定义域）。`closed` 的定义、义务拼接口径、证据门与聚合完全由 `ADR-0024` 决定，本条不重复、不放宽、不另行解释。任一条 `closed: false`，出口不成立。E2/E3 是 `closed` 之外追加的**出口条件**，只加严出口，不参与也不修改 `closed` 的计算。`closed` 只检查义务是否有结构化证据，不检查 `ref` 是否真的断言了义务文本；因此 E1 的成立以决定 6 的见证链复核为前提，未经复核的 `closed: true` 不足以支持出口。

3. **E2：逐判据、逐义务的执行型证据下限。** 对定义域中的每一条判据，按义务逐条检查，不使用判据级存在量词：
   - **测试名义务**：判据中每一条 `ref` 为测试名、未标 `kind: notApplicable` 的义务，都必须在候选 commit 上有至少一条运行点名它；
   - **命令义务**：判据中每一条 `ref` 为命令的未标 `notApplicable` 义务，同样逐条适用；整包测试运行不能顶替判据自己声明的入口命令或 trace；
   - **类型层义务豁免**：某条测试名义务若其**全部**现存证据运行都是类型层运行（`kind == "compile-fail"`，或命令包含 `--test compile_fail`、`--test trybuild`、`rustdoc --test`），该义务免于逐条点名；豁免不传递给同判据的其它义务。
   每条义务的见证运行 `R` 必须同时满足：
   - `R.exit == 0`；`R.class` ∈ {`A`, `D`}（只读 `runs[].class`，定义见 `docs/evidence/validation.md` §0；`B` 转述与 `C` 仓库外夹具不算）；`R.kind` ∈ {`test`, `runtime`}；
   - `R` 不是类型层专用运行：`R.kind` 不是 `compile-fail`，且 `R.command` 不含 `--test compile_fail`、`--test trybuild` 或 `rustdoc --test` 子串（典型反例是 `EVID-VAL-41`：它记为 `kind: "test"`、命令为 `cargo test -p zenui-core --test compile_fail --locked`，实际只执行 trybuild runner）；
   - `R` 有 run 级 40 位十六进制 `commit`；不接受矩阵顶层 `commit` / `commitNote` 回退；
   - `R.kind == "test"` 时 `tests` 对象存在、`tests.failed == 0`、`tests.passed >= 1`；`R.kind == "runtime"` 时有 `traceSha256` 或 `proves`；
   - **`R` 点名义务**：义务的 `ref` 是测试名时，`R.names` 必须包含一个与它按 `::` 分段精确相等的条目。`names` 是该运行**已执行测试的全名清单**（如 `module::tests::fn_name`），匹配时取每项最后一个 `::` 段与 `ref` 精确相等，**不做子串匹配**；`names` 不是样例，也不是筛选后的子集。`ref` 是命令时，`R.command` 与该命令逐字相等；
   - **`R.commit == <候选 commit>`**：E2 只接受在候选 commit 上执行的运行（比 `ADR-0024` 的历史证据门更严，但不改变 `closed`）。
   判据级 `evidence[]` 是义务证据的并集，不能替代逐义务点名；未被点名的义务即视为未满足 E2。**若某条判据的全部义务都是 `notApplicable`、或只剩类型层/预期失败义务**，E2 对该判据不成立、出口阻塞；该判据只能由 superseding ADR 按决定 1 移出集合。E2 只证明「运行点名了该 `ref`」，不证明「该 `ref` 断言了该义务」；后者只能由决定 6 的见证链复核回答。

4. **E3：出口时点的集成状态。** 出口记录必须指定**候选 commit**（被认证的代码树）与**记录 commit**（新增该出口 ADR 的提交）。在候选 commit 上必须存在两类已记录运行，二者都满足 `run.commit == 候选 commit`：
   - **E3a（本机）**：`class == "A"`、`command == "cargo test --workspace --locked"`、`exit == 0`、`tests` 对象存在、`tests.failed == 0`、`tests.passed > 0`；
   - **E3b（CI）**：`class == "D"`、同一命令、`exit == 0`、`tests` 对象存在、`tests.failed == 0`、`tests.passed > 0`，且 run 级 `platforms[]` 按 OS 家族（ubuntu / macos / windows）覆盖候选 commit 上 `.github/workflows/workspace.yml` 矩阵的全部 OS 家族。允许 `*-latest` 与具体镜像名互认；读 run 级 `platforms[]`，不读 `tests.platforms` 整数。
   候选 commit 必须是记录 commit 的祖先，`git diff --name-only <候选> <记录>` 只允许 `docs/decisions/`、`docs/evidence/`、`docs/README.md`、`docs/product/` 下的变更（`docs/spec/` 不得在候选之后改动）；**记录 commit 本身不得作为候选**（E3 运行必须先于记录完成）。若候选与记录之间出现其它变更，取记录之前最后一个满足该条件的 commit 为候选，并重跑 E3、重算 E1/E2。
   **CI 平台下限**：ubuntu、macos、windows 三个 OS 家族是冻结下限；把矩阵缩小到不足三个家族必须新开 superseding ADR，不能只靠「重新裁决」。

5. **`PROBE` 与出口的明确规则（不留「视情况」）。** 判据状态一律取 `criteria[].status`（即候选 commit 上 `08-gates.md` 判据表逐行最后一个单元格）；gate 汇总行的 `status` 与 `gates[].status` 不参与判定。本条中的「M0 未闭合」指 E1–E3 未全部成立，不等同于 `gates[].closed == false`（后者只是 E1 的机器字段）。
   - `PROBE` 且 `closed: true`：在结构条件上**通过**，不需要也不允许再要求 `VERIFIED`；
   - `PROBE` 且 `closed: false`：**不通过**；「已有探针」「只差一条子句」「证据看起来够」都不是出口理由；
   - `OPEN` / `SPEC` / `WITHDRAWN`：**不通过**（`closed ⊆ PROBE`，`ADR-0024` 决定 1）；
   - 不存在部分出口：定义域中任意一条判据不满足 E1 或 E2，或 E3a/E3b 任一不成立，M0 即未闭合；不按比例、不按 gate 汇总行、不按「其余都绿了」四舍五入；
   - 产品不签发放行：唯一的放宽路径是 `08-gates.md` §9 + `ADR-0024` 的 `notApplicable` + `acceptedBy`，且目标文本已落进 A1 行（`ADR-0025`）。

6. **出口裁决的记录形状与效力。** 出口成立时，必须新开一条 ADR（decisions 层）作为出口记录；`product/roadmap.md` 只做引用更新，不承担裁决（其 frontmatter 为 `draft`，按 `docs/README.md` §4 不能作为验收依据）。**判定快照**：定义域与 `criteria[].status` 取自**候选 commit** 的 `docs/spec/08-gates.md`；E1/E2 读取**记录 commit** 的 `docs/evidence/matrix.json`（记录必须写明这两个 blob 的 SHA）。出口记录内容至少包含：候选 commit 与记录 commit；E1 的 `criteriaClosed` 计数与逐 gate 聚合；E2 的逐判据见证链（判据 ID → 被选中的未标 `notApplicable` 的义务 → 运行 ID → 该运行如何在 `names`/命令中点名该义务）；E3a/E3b 的运行 ID；未映射义务清单（必须为空）。**未映射义务**是静态可解析性概念：`ref` 不可解析、且未标 `notApplicable` 的义务（与 `ADR-0024` 的失败模式同一口径）；它与 E2 的「运行点名义务」是两件事，前者覆盖全部义务，后者只覆盖每条判据被选中的那一条。**见证链复核是出口前置**：出口记录必须附一份逐义务复核结论（义务原文 ↔ `ref` 的断言内容 ↔ 运行），由复核者签署；任何「夹具存在但不证明该义务」的条目使该义务视为未映射、对应判据 `closed: false`，不得用兄弟义务、另一条判据或另一条运行顶替。复核未完成时不得记录出口；E2 的点名不等于断言成立。本 ADR 只定义条件，不声称任何条件在撰写时已满足。`roadmap.md:45` 的第二句「嵌套 batch 的中间值泄漏反例转绿」由 `SPEC-GATE-A0-04` 的 E1+E2 满足：其 E2 运行必须是该夹具在候选 commit 上的一次执行（历史记录 `EVID-VAL-23` 只证明夹具存在，不能代替候选 commit 上的运行），不得用 `EVID-VAL-08`（`class: C` 仓库外夹具）代替。出口只解除 `roadmap.md:37` 对 M1 宏 lowering 扩展的阻塞，是 M1 的**必要非充分**条件；M1 仍受 `SPEC-ROUTE-003` 前置、`SPEC-GATE-A1` 出口与对外承诺条款约束。出口记录不得把结论表述为「A0 `VERIFIED`」。

7. **出口判定的重开（撤销）条件。** 出现下列任一事实，出口判定必须重算；重算后任一条件不成立即撤销。a–d 可由 matrix.json / git / 候选 spec 机械判定；d 的重跑与 e 是必须落盘的事件：
   - a. 定义域内任一判据的 `closed` 由 `true` 变为 `false`；
   - b. A0 判据集合变化（新增、删除、改名、拆分）：必须有 superseding ADR 给出 old → new 映射与新分母（`08-gates.md` §9）；集合未变而任一判据状态变化，按决定 5 重算 `closed`，触发 a 时即撤销；
   - c. 出口依赖的任一运行失效：run 级 `commit` 被重写或不再是记录 commit 的祖先（E2/E3 所依赖的运行还必须仍等于候选 commit）、运行被删除或改记为失败、主机/命令/摘要字段被撤回、`ref` 不再被该运行点名。人工复核发现运行并未真正断言所映射义务（假映射）时，由新决策记录撤销；
   - d. E3a 或 E3b 不再可复现：同 commit、同命令的新运行 `exit != 0` 或 `tests.failed > 0`；或复核发现 E3b 未覆盖候选 commit 上矩阵的全部三个 OS 家族；
   - e. `ADR-0024` 或 `ADR-0025` 被 supersede、撤销，或 `ADR-0025` 的**任一**撤销条件（1–4）由新 ADR 记录触发，导致对应判据 `closed` 变假。
   撤销不删除历史：不得就地改写出口记录；按 `docs/decisions/index.md` §5 新开 ADR 记录撤销与 new 记录指向，旧记录正文不改；撤销后依赖 M0 的 M1 工作（宏 lowering 扩展）停止，直到 E1–E3 重新成立。撤销由产品负责人记录，`closed` 的重算由证据层给出，谓词争议由架构裁决。

## 依据

- `08-gates.md:19`：spec 层状态词为 `SPEC` / `PROBE` / `OPEN` / `WITHDRAWN`，`VERIFIED` 只在 evidence 层声明；五词表见 `docs/README.md` §2。
- `08-gates.md:21-26` §0：0 tests 的 target 不算通过；缺 runner = blocked；判据必须能在干净机器上复跑。
- `08-gates.md:48`：Gate 总表把 A0 的方法写成「编译期 + 运行时 trace」。
- `08-gates.md:73-85`：A0 的 11 条 MUST 列各自包含运行时可观测量（trace、提交次数、计数、回滚逐字段比对、身份保留、churn 回基线等）。
- `08-gates.md:174-179` §9：判据只能加严或依证据放宽；放宽必须留 ADR；改名/拆分保留 old → new。
- `docs/README.md` §2：五个状态词；`VERIFIED` 仅 evidence 层；禁止「基本完成」类措辞。§4：`normative` 文档与实现冲突时以文档为准。
- `ADR-0024` 决定 1、2、3、4、5、6：`closed` 的定义与证据门、gate 全称聚合、`criteria[]` 由证据层计算、`criteriaClosed` 的派生口径、0 `VERIFIED` 停止作为论据、谓词不裁决 M0 出口；本条只消费它，不重复定义。
- `ADR-0025` 决定 1–7 与撤销条件 1–4：`notApplicable` 的合法性来自 `acceptedBy` 目标文本落地；部分对象不得整体移交。
- `roadmap.md:31`、`:37`、`:45-46`：M0 的交付与「全项通过」措辞、M1 的前置顺序、M0 无对外承诺；`:52-53` M1 的独立前置与出口。
- `docs/evidence/index.md` §2.3（`:100`）：一次复跑最少记录 commit、主机、命令、退出码、摘要；E2/E3 的字段要求是它的机器化。
- `docs/evidence/validation.md` §0：`class` 的 A/B/C/D 定义（A 本轮实测、B 上轮转述、C 仓库外夹具、D CI 托管 runner）。
- `SRC docs/evidence/matrix.json`：`runs[]` 的 `class` / `host` / `kind` / `exit` / `tests` / `commit` / `traceSha256` / `names` / `platforms` 字段；`kind` 现有取值为 `test` / `runtime` / `compile-fail` / `lint` / `external-fixture` / `gpu-probe`；`EVID-VAL-41` 是 `kind: "test"` 的 trybuild runner。`names` 与 `platforms` 是 schema 允许的字段，但撰写时只有 2 条 run 带 `names`；出口记录前必须为被 E2 依赖的运行补齐全名清单。
- `SRC .github/workflows/workspace.yml`：CI 矩阵当前为 `ubuntu-latest` / `macos-latest` / `windows-latest`，测试步骤为 `cargo test --workspace --locked`。
- `SRC scripts/check_docs.py` 的 `check_run_commits`：只校验 run 的 commit 是 **HEAD** 的祖先，不针对候选 commit；E2 的 `run.commit == 候选` 与 E3 的候选-记录祖先/目录检查必须另行执行。`check_matrix_coverage` 与 `check_matrix_criteria`：coverage 与逐 gate 计数从 spec 状态列重算，`criteria[].closed` 由义务与证据重算。
- `ADR-0012` 决定 2：`VERIFIED` 只允许出现在 evidence 层。

## 影响与撤销条件

**影响**

- 产品侧：M0 出口从「看起来差不多了」变成三条可重算条件；出口裁决本身成为一条独立记录，不再由汇总行或口头结论承担。
- **阻塞前置**：E1–E3 依赖的 `criteria[]`、`gates[].closed` 与 checker 已在 main；在候选 commit 的运行与决定 6 的逐义务见证链复核完成前，不得记录出口；本条不宣称任何条件已满足。E1 的 `closed: true` 还必须在出口前通过复核；结构谓词本身不能证明 `ref` 断言了义务。
- 证据层（发布负责人）：`criteria[]` 落地时需保留义务的 `kind`、义务级 `evidence[]` 与运行的 `names`（**已执行测试的全名清单**，不是样例），使 E2 的逐义务点名可计算。E2 与 E3 都要求运行在候选 commit 上：测试名义务由 E3a 的 workspace 运行覆盖（其 `names` 必须列出全部被点名的测试），A0-01 的 trace 等命令义务按 `EVID-VAL-45` 的形状在候选 commit 重录；E3b 按现有 `EVID-VAL-47` 的形状在候选 commit 新增。
- 文档：`roadmap.md` 的 M0 出口行改为引用本条（不重复条件）；`decisions/index.md` §2 与 §4 登记本条；`docs/README.md` §9 的文件计数随新增文件更新。本 ADR 不改 `08-gates.md` 的任何状态列，不改 `matrix.json`。
- 若 `ADR-0025` 的「相邻证据缺口」复核后由既有夹具覆盖，那只改变证据层的映射，不改变本条：E1 仍按 `ADR-0024` 重算。
- 本条不裁决 M0 当前是否已出口；在候选 commit 的运行与见证链复核完成前，任何「已出口」表述都违反本条。

**撤销条件（可判定）**

1. `docs/product/roadmap.md:45` 的 M0 出口措辞或 `docs/spec/08-gates.md` §2 的 A0 范围/判据集合被修改，且新定义与 E1–E3 不可同时成立——此时必须新开 ADR supersede 本条，不得就地修改。
2. `ADR-0024` 被 supersede，或 `closed` 的定义/证据门发生变更，使 E1 与本条引用的口径不再一致——须新开 ADR 说明新的对应关系。
3. `matrix.json` 的 `runs[]` 字段或 `kind` 取值发生变更，使 E2 的执行型证据集合无法按本条判定——须新开 ADR 重新枚举，而不是放宽判定。
4. 本条实施后发现 E1–E3 在合法场景下互相冲突（例如某条 A0 判据按 §9 加严后无法产生执行型证据），须新开 ADR 说明冲突，不得由评审口头豁免。
