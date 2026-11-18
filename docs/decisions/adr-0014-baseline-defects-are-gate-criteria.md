---
doc-id: ADR-0014
layer: decisions
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - docs/spec/08-gates.md §4
  - docs/spec/05-text.md
  - docs/evidence/validation.md §7
  - archive/review/text-layout-review.md
replaces: []
---

# ADR-0014：baseline / intrinsic / synthesis 缺陷写成 Gate 硬判据

## 背景

现有探针里已经确认三类适配器缺陷：

1. `Paragraph::measure()` 计算了 `first_baseline()`，但传给 Taffy 的 measure 闭包只返回 `.size`，
   baseline 被丢弃；
2. 叶子测量只使用 `known.width`，没有区分 known dimensions、available space 与 min/max-content；
3. glyph 绘制没有把 Parley run 的 Font synthesis 转发给光栅层，cache key 也不含合成参数。

这些缺陷不会被"4 个 native 测试通过"掩盖：测试只验证了测量被调用、文字变更会失效、CJK 像素非空。
如果把缺陷写成"已知问题"备注，实现者可以用"探针绿"关闭 B Gate。必须裁定：缺陷如何进入验收。

## 备选项

1. **记为 known issue，不阻塞 Gate**：最省事，但测试绿会制造完成假象。
2. **写成 Gate B 硬判据**（选中）：每条缺陷对应一条可判定的通过判据与证据要求。
3. **只在上游库修复**：Taffy/Parley 不负责本框架的 adapter 传参，无法替代。
4. **删除相关测试**：掩盖问题，违反"不得改失败测试"。

## 决定

1. **B-01（baseline）**：首 baseline 与 content top 必须来自最终 accepted text layout，并真正进入 Taffy
   leaf 输出；known-size 早退路径复用同 epoch（`SPEC-GATE-B-01`、`SPEC-TEXT-003`）。
2. **B-02（intrinsic）**：known dimensions、available space、min/max-content 分开建模；禁止用一个
   `Option<f32>` 冒充全部测量模式（`SPEC-GATE-B-02`、`SPEC-TEXT-002`）。
3. **B-03（synthesis）**：`run.synthesis()` 的 faux bold/oblique 必须导向 glifo；cache key 必须含
   face + weight + variation + scale（`SPEC-GATE-B-03`、`SPEC-TEXT-006`）。
4. **探针绿不关闭缺陷**：`EVID-VAL-03`/`-14` 只证明当前测试通过；在 fixed-font golden 通过前，
   `SPEC-TEXT-001`…`-003`、`-006` 的缺陷状态保持 `OPEN`（D-06）。
5. **不许改测试迎合实现**：若规格与上游 API 冲突，先记录证据并开 ADR，不得把失败测试改成通过。

## 依据

- `SPEC-GATE-B-01`、`-02`、`-03`：三条通过判据与"当前状态"列已写明现存缺陷。
- `SPEC-TEXT-001`：Taffy leaf 回调是文本测量唯一入口；`SPEC-TEXT-002`：四种测量模式分开；
  `SPEC-TEXT-003`：baseline 来自最终 accepted layout；`SPEC-TEXT-006`：Font synthesis 必须转发。
- `CLAIM-056`：measure 内部计算了 `first_baseline()`；`CLAIM-057`：回调只返回 `.size`；
  `CLAIM-058`：只使用 `known.width`；`CLAIM-060`：无 synthesis 转发。
- `EVID-VAL-03`/`-14`：native 4 条测试在 H1/H2 通过——但 `CLAIM-055` 明确其覆盖范围；
  `evidence/validation.md` §7 `SRC-01…03` 给出源码级缺陷位置。
- `CLAIM-059`：Parley 官方文档承认 `calculate_content_widths()` 对 mixed-direction text 可能不正确，
  因此 intrinsic 不能默认正确。

## 影响与撤销条件

**影响**

- B Gate 的绿色必须附带 fixed-font golden；"测试通过"不能作为关闭理由。
- 实现负责人在 `SPEC-TEXT-*` 修复后，新增 `EVID-VAL-*` 并更新 `08-gates.md` 的状态列；不得直接改
  `CLAIM-056`…`-060` 的结论而不补证据。
- 评审记录中的"E2 已发现"仅作来源；现行判据以 `SPEC-GATE-B-*` 为准。

**撤销条件（可判定）**

1. 只有当 `SPEC-GATE-B-01`/`-02`/`-03` 的 fixed-font golden 在 H2（或目标平台）实际通过，并登记新的
   `EVID-VAL-*`，才可把这些条目从"现存缺陷"改为"已修复"；
2. 若上游 Taffy/Parley API 变化使"measure 闭包返回完整 `LayoutOutput`"在技术上不可行，必须开新 ADR
   给出等价判据（例如通过 context 回传 baseline），并保留旧 ID 的映射；
3. 若发现 golden 本身错误（例如固定字体 artifact 变了），先修 artifact 并新开 `EVID-VAL`，不得改判据
   来适配错误 golden。

仅"测试已经绿了"或"实现太麻烦"不构成撤销依据。
