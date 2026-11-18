---
doc-id: ADR-0016
layer: decisions
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - archive/review/FINAL.md §1.3、§3.2
  - docs/spec/02-runtime.md
replaces: []
---

# ADR-0016：释放顺序是子先父后的后序 + 同 owner 逆注册 LIFO

## 背景

FINAL §1.3/§3.2 规定 OwnerScope 负责订阅、异步 ticket 与 cleanup 的注销，但没有规定**顺序**。顺序不写死会产生三种都能通过"单测绿"的实现：

- 父作用域先释放，子 cleanup 随后读取已失效的父槽；
- 同 owner 内多个 cleanup 按注册顺序执行，导致后建立的资源先于它依赖的资源被释放；
- 释放期间某个 cleanup 触发 mount，使正在收缩的树反向增长。

这三种都是卸载路径上的偶发错误，恰恰最难在开发期复现。`SPEC-RT-021` 已按本裁决落地，此前一直标 `OPEN-RT-07`。

## 备选项

1. **前序（父先子后）**：实现最直观。
2. **后序 + 注册顺序**：先子后父，但同 owner 内按创建顺序。
3. **后序 + 逆注册（LIFO）**（选中）。
4. **完全交给 Rust `Drop` 的自然顺序**：不在框架层规定。

备选 1 会让子 cleanup 读已释放句柄；备选 2 的"先建的先拆"与依赖方向相反——后建立的资源通常依赖先建立的资源，因此必须先拆后者；备选 4 把顺序交给字段声明顺序，重构字段就会改变语义，且 `Drop` 无法表达"释放期间禁止 mount"。

## 决定

1. Owner 树释放 MUST 按**后序**：先子树，后父作用域（`SPEC-RT-021.1`）。
2. 同一 owner 内多个 cleanup MUST 按**注册逆序**（LIFO）执行（`SPEC-RT-021.2`）。
3. cleanup 中 MUST NOT 发起 mount、batch 或 flush；期间产生的资源归同一 owner 或被拒绝（`SPEC-RT-021.3`）。
4. `dispose` MUST 幂等，重复调用无副作用（`SPEC-RT-021.4`）。
5. View 删除 MUST NOT 在 cleanup 中直接改树，只入事务 journal（`SPEC-RT-021.5`、`SPEC-RT-052`）。

## 依据

- 后序：子作用域可以合法持有父作用域内的句柄（父对子的可见性），反向由 `SPEC-RT-022` 禁止。因此唯一安全顺序是子先释放。
- LIFO：同一 owner 内的资源按创建顺序建立依赖，逆序释放保证"依赖者先于被依赖者"消失。
- 幂等与重入禁令：释放是事务性的，任何在释放中改变树的行为都会产生"边释放边增长"，只能靠禁令而非事后检查。

## 影响与撤销条件

- 影响：Owner arena 需要记录注册序号；cleanup 需要"释放中"标志位；A0 必须提供 mount/unmount churn fixture 与"cleanup 中 mount 被拒"的用例。
- 撤销条件（可判定）：若 A0 的 churn fixture 出现**合法的跨 sibling 依赖**，使 LIFO 在兄弟仍在引用时释放资源，则改为显式依赖拓扑序，并新开 ADR 取代本条。该情形同时意味着 `SPEC-RT-022`（所有权无环）需要放宽，应一并评审。
