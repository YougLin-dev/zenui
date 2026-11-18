---
doc-id: ADR-0004
layer: decisions
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - archive/review/FINAL.md §0、§7.2、§8
  - archive/research/runtime-and-rendering.md §5
  - docs/spec/01-route.md
  - docs/spec/06-render.md
replaces: []
---

# ADR-0004：sparse-strips 为生产候选，Classic 仅 probe

## 背景

仓库里同时存在两条 Vello 线：一条是早期的 Classic 研究线，另一条是 sparse-strips 的 GPU/CPU 组合。
两条线对半透明合成的约定不同——Classic 的探针使用 straight-alpha source-over，sparse-strips 的
readback 是 premultiplied。历史上曾出现"把一条线的 alpha 结论套到另一条线"的错误。

必须先裁定：哪条线可以作为生产后端，哪些只能作为证据；两者能否共享默认？

## 备选项

1. **Classic 作为生产后端**：生态更成熟，但 alpha/合成约定与 sparse-strips 不同。
2. **sparse-strips 作为生产候选 + CPU reference/fallback**（选中）。
3. **只用 CPU**：确定性好，但没有 GPU 生产能力。
4. **两条线都作为生产后端**：按平台选择。
5. **引入第三个后端**：不在本轮裁决范围。

备选 1/4 会让公共 API 同时承载两套 alpha/format 合同；备选 3 放弃 GPU 目标；备选 5 没有证据。

## 决定

1. **sparse-strips 的 GPU 线是生产候选**；CPU 线是确定性 reference 与显式 fallback。
2. **Classic 仅作互操作 probe**，`MUST NOT` 成为第二生产后端或公共 ABI（`SPEC-ROUTE-002`、`-027`）。
3. **alpha 约定不跨后端/版本共享默认**：外部纹理与合成必须携带 `format/color_space/alpha/frame_epoch`
   等元数据（`SPEC-RENDER-009`、`-010`）。
4. **公共 API 不暴露** Vello `Scene`、`TextureId`、`wgpu::SurfaceTexture` 等后端 identity
   （`SPEC-RENDER-003`）。

## 依据

- `SPEC-ROUTE-001`：生产候选与 reference 的角色固定；`SPEC-ROUTE-002`：Classic 仅 probe；
  `SPEC-ROUTE-027`：Classic 作第二生产后端为 `WITHDRAWN`。
- `SPEC-RENDER-007`：后端角色固定；`SPEC-RENDER-010`：alpha 约定不得跨后端共享。
- `CLAIM-064`、`CLAIM-065`：sparse-strips headless 在 H1/H2 得到相同 readback；
  `CLAIM-066`：Classic 的 underlay/overlay 仅证明连通性；`CLAIM-067`：两条线 alpha 约定相反。
- `EVID-VAL-05`/`-15`（sparse-strips）与 `EVID-VAL-06`/`-16`（Classic）：都是 headless 单场景，
  不证明窗口、surface、device-loss 或平台矩阵。
- `CLAIM-073`：readback 成功不证明 retained paint cache 或窗口 present。

## 影响与撤销条件

**影响**

- 生产路径的 alpha/format 以 `SPEC-RENDER-010` 为准；Classic 探针的任何像素结论不得直接进入 golden。
- 外部纹理必须附带完整元数据；"只是换个 crate 名"的迁移按缺陷处理。
- Gate C 的 GPU 矩阵按实际后端分别记录，不能把两个 headless 数据点称为"矩阵通过"。

**撤销条件（必须同时满足）**

1. `SPEC-GATE-C-03` 在任一目标平台上证明 sparse-strips 无法产出与 CPU reference 在声明容差内一致的
   readback，且失败可归因于后端能力而不是本框架的 metadata/合成缺陷；
2. 另一个候选后端在同一平台、同一场景通过 `SPEC-GATE-C-03` 与 `SPEC-GATE-B-08`；
3. 替代方案仍满足 `SPEC-RENDER-003/008/009/010` 的边界与能力探测要求。

仅性能差异、生态偏好或"Classic 更简单"不构成撤销依据。
