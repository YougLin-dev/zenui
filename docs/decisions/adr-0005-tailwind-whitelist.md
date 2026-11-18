---
doc-id: ADR-0005
layer: decisions
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - archive/review/FINAL.md §6、§8
  - archive/research/style-system.md
  - archive/research/tailwind-coverage.md
  - docs/spec/04-style.md
  - docs/spec/03-macro.md
replaces: []
---

# ADR-0005：Tailwind 白名单，不是 CSS 引擎

## 背景

用户希望保留 Tailwind 的书写习惯，但本项目不是浏览器：没有 DOM、没有 CSS 全局 selector、没有
JS plugin。若直接把 Tailwind 当 CSS 引擎运行，会引入无法闭合的兼容性工程，并让"支持多少"变成不可
计量的口头承诺。

必须裁定：样式层是"运行 Tailwind CSS"，还是"编译固定子集到原生声明"。

## 备选项

1. **运行时 CSS 引擎 / 浏览器内核**：完整 cascade、selector、at-rule。
2. **完整 Tailwind Rust port**：把所有 utility 与 variant 映射到原生字段。
3. **pinned registry 白名单 + typed 原生声明**（选中）：只接受固定上游版本生成的允许清单，未知输入诊断。
4. **无样式系统**：只提供原始属性，用户自建。

备选 1 会把 DOM 语义与浏览器行为偷偷带进核心；备选 2 无法给出可验证的完成边界；备选 4 放弃项目定位。

## 决定

1. **唯一编译链**：字面量 class → `StyleRule`/`StyleProgram` → 原生字段；**禁止**运行时 CSS selector
   解释器与任意 class 字符串（`SPEC-STYLE-001`、`SPEC-ROUTE-028`）。
2. **支持集由固定上游版本生成的 registry 决定**；N/D/R/X 词表固定，未知/X 类必须诊断
   （`SPEC-STYLE-033`、`-034`、`-040`、`SPEC-MACRO-013`、`-017`）。
3. **不做运行时拼接**：`format!("bg-{theme}")` 一类写法必须编译错误；动态 class 只接受编译期完整候选
   （`SPEC-STYLE-030`、`-032`）。
4. **覆盖率只能是分子/分母 + `EVID-MATRIX` 来源**，不得手写百分比（`SPEC-STYLE-041`、`-042`）。
5. **`@apply`、`@plugin`、任意 selector 不作为核心依赖**；是否支持由固定版本差分决定
   （`SPEC-STYLE-013` 对 important/layer 的冻结前禁用同理）。

## 依据

- `SPEC-SCOPE-020`：Tailwind 只承诺 pinned registry 生成的支持清单。
- `SPEC-STYLE-001`：单一编译链，拒绝运行时解释器；`SPEC-STYLE-033`：支持清单由固定上游生成；
  `SPEC-STYLE-040`…`-042`：N/D/R/X 与覆盖度记账。
- `SPEC-MACRO-013`/`-014`/`-017`：class 只接受编译期字面量；状态样式走登记条件；未知输入拒绝。
- `CLAIM-083`、`CLAIM-084`：现有探针只覆盖少量 literal class 与 3 条 style 断言。
- `CLAIM-087`：固定版本 manifest 与差分测试尚未生成；`CLAIM-088` 的 Rust oracle 只是研究探针（`PROBE`）。
- `CLAIM-085`：固定 commit 的 aria 命名变体只匹配 `"true"`，没有简单 false 变体——这是"不能凭直觉
   扩展 variant"的具体反例。
- `EVID-VAL-12`/`-13`：H1/H2 各 7 + 2 条测试，只覆盖少量 utility 与诊断，不证明覆盖率。

## 影响与撤销条件

**影响**

- 新增一个 utility/variant 必须同时给出：registry 来源、N/D/R/X 判定、失败模式、golden 或诊断快照。
- 未知 class 不得静默忽略；`class` 内的子 token span 归宏（`SPEC-MACRO-019`），但 style 层要给结构化错误。
- 产品文档只能用"已实现且有已验 fixture"的分子/分母，不能写"大部分支持"。

**撤销条件（按类族逐条判定）**

某一个当前被标为 `X` 或未登记的类族，只有在同时满足以下条件时才能提升为 `N`/`D`/`R`：

1. 固定上游 commit 能生成该类族的 candidate manifest；
2. 本框架存在原生等价物，且失败模式可在 `spec/04-style.md` 中写清；
3. 有 golden 或差分测试证明 token → declaration 的映射（不是只解析字符串）。

整体撤销"白名单路线"需要证明：运行时 CSS 引擎能在不引入 DOM/脚本的前提下满足 `SPEC-STYLE-010`
的唯一排序规则，并给出可复现的兼容性分母。仅"用户想要更多类"不构成撤销依据。
