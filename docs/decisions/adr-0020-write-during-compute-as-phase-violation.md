---
doc-id: ADR-0020
layer: decisions
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - docs/spec/02-runtime.md
  - docs/spec/08-gates.md
  - docs/decisions/adr-0017-p2-runtime-authority-and-transaction-shape.md
  - docs/decisions/adr-0019-failure-classification-and-phase-typing.md
replaces: []
---

# ADR-0020：`WriteDuringCompute` 由 compute 相位的 `PhaseViolation` 表达

## 背景

P2 的实现把 `Fatal::WriteDuringCompute` 删掉了：写 API 的阶段违规统一走 `ValueError::Phase(PhaseViolation)`（ADR-0019 决定 3），最外层再把同一个 `PhaseViolation` 兑现为 `Fatal::PhaseViolation`。保留一个同名的 `Fatal` 变体会造出第二条通往同一后果的路径。

这个删除引出一个必须落纸的问题：`SPEC-RT-038` 与 `SPEC-GATE-A0-05` 的括号列表里仍然写着 `WriteDuringCompute`。如果只在代码里删掉变体而不在决策层记录映射，下一轮会有人按名字去找一个不存在的东西，或者反过来「补回」那个变体。`SPEC-RT-038` 的失败模式也明确要求这类错误可执行、可匹配，所以映射必须写清楚，而不是靠代码考古。

## 备选项

1. **保留 `Fatal::WriteDuringCompute` 与值通道并存**：同一个后果有两条路径，两个名字会在后续演化中各自漂移；违反 ADR-0019 决定 3 的单一通道。
2. **保留类型别名或构造函数** `PhaseViolation::write_during_compute(node)`：类型别名无法约束 phase（`PhaseViolation.phase` 是动态值）；构造函数又把名字带回代码面，等于保留一个半变体。
3. **只保留致命变体、写 API 直接返回 `Fatal`**：写 API 出现两种错误类型，每个调用点都要两层 match；违反 ADR-0019 决定 3。
4. **用 `PhaseViolation { phase: Topo | Binding, attempted }` 表达**（选中）：一个 payload、两个通道，名字只留在文档域。

## 决定

1. **`WriteDuringCompute` 不是错误变体。** compute 阶段（`Topo` / `Binding`）的写由 `ValueError::Phase(PhaseViolation)` 表达；最外层 exit 用同一个 `PhaseViolation` 产出 `Fatal::PhaseViolation`。两个通道包装同一个结构，禁止各定义一份字段。
2. **信息没有丢。** `Topo` / `Binding` 只出现在帧内计算期间（`SPEC-RT-050` 的固定阶段顺序），没有其他情形能在这两个相位发起写；因此 `phase ∈ {Topo, Binding}` 与 `attempted` 的组合**唯一**标识「compute 期间写」，不需要独立变体承载。
3. **不保留同名 `Fatal`。** ADR-0019 决定 3 已经规定写 API 的阶段违规走值通道；同名致命变体会成为第二条路径，并在两处独立演化。
4. **名字的存续范围。** `WriteDuringCompute` 只作为 `SPEC-RT-038` / `SPEC-GATE-A0-05` 的**领域错误名**与测试名存在；在 `crates/` 里搜索这个名字不得再期待一个变体。若诊断或分类需要稳定标签，必须从 `phase ∈ {Topo, Binding}` 推导，不得另立枚举分支。
5. **与 ADR-0017 / ADR-0019 的关系。** 不就地修订两者；本条是 ADR-0019 决定 3–5 的直接推论，并关闭「是否保留同名 `Fatal`」这个未决点。

## 依据

- `SPEC-RT-038`：Memo / binding / measure / layout 阶段的写 MUST 被判定为领域错误并以 typed 错误返回；MUST NOT 让借用文本泄漏，MUST NOT panic。类型叫什么由实现决定，但必须是可模式匹配的。
- `SPEC-RT-050`：阶段顺序固定，`Topo` / `Binding` 只在计算阶段出现，`Commit` 只允许在 depth == 1 发生；这是「相位对唯一标识 compute 写」的前提。
- `SPEC-RT-055`：失败后必须恢复；毒化在最外层 exit 清除，下一帧可提交。
- `SPEC-RT-057`：错误进入统一 sink，MUST NOT 静默替换旧值、MUST NOT 把 `Result` 塞进文本节点。
- `SPEC-GATE-A0-05`：要求 `WriteDuringCompute`、cycle 是可观测且可恢复的领域错误；本条说明前者的可观测形态是 `ValueError::Phase` 的相位对，而非独立变体。

## 影响与撤销条件

影响：

- A0-05 的夹具按值通道断言：写调用得到 `ValueError::Phase`、`phase` 属于 `{Topo, Binding}`、帧的 `commits` 不前进、下一帧提交成功；不得按变体名匹配。
- 代码里 `grep WriteDuringCompute` 只应命中文档与测试名，不应命中 `crates/**` 的枚举定义。
- 诊断输出若要区分「compute 期写」，从 `(phase, attempted)` 推导；不新增同义枚举分支。

撤销条件（可判定）：

- 若新增一个在 compute 之外也会出现的非法写相位，`{Topo, Binding}` 不再唯一标识 compute 写；届时新开 ADR，为 `PhaseViolation` 增加 `cause` 字段或恢复独立变体。
- 若出现一个夹具要求「同一相位对上区分 compute 写与另一种非法写」，同样触发上一条。
