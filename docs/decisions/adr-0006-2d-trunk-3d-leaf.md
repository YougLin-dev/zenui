---
doc-id: ADR-0006
layer: decisions
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - archive/review/FINAL.md §7.3、§8
  - archive/research/runtime-and-rendering.md §6
  - docs/spec/00-scope.md
  - docs/spec/06-render.md
replaces: []
---

# ADR-0006：2D 主干 + 3D 叶节点

## 背景

项目以桌面 UI 为主，但希望可选地嵌入 3D 模型视图。把 3D 做成第二套窗口/呈现路径会带来第二套 surface、
交换链、设备生命周期与输入路由；把它做成完整游戏引擎则违背"UI 框架"的定位。

必须裁定：3D 在架构中的位置，以及它与 2D 合成、资源生命周期的边界。

## 备选项

1. **完整 3D 引擎**：ECS、场景图、材质系统、独立窗口。
2. **3D 作为 Taffy 布局叶节点 + 独立离屏 color/depth + 普通纹理回到 2D**（选中）。
3. **无 3D**：只做 2D。
4. **同一个 pass 里 2D/3D 深度交错**：共享深度缓冲。

备选 1 会把游戏运行时塞进核心；备选 3 放弃已声明的可选能力；备选 4 在同一 pass 读写同一纹理，
既违反 GPU 规则，也让 2D/3D 的 z-order 无法定义。

## 决定

1. **2D 是主干，3D 是布局叶节点**（`SPEC-SCOPE-015`）。
2. **每个 View generation 自管离屏 color + `Depth32Float`**，解析为普通 `TextureView` 后回到 2D
   （`SPEC-RENDER-019`、`-021`）。
3. **禁止第二套窗口/交换链/呈现**（`SPEC-RENDER-020`）；3D 不得复用 Vello 内部 depth，也不得在同一
   pass 既写又采样同一颜色纹理。
4. **glTF 资产管线是 `SPEC`，不是已实现**（`SPEC-RENDER-022`）；第一阶段只承诺最小合法子集与深度遮挡。
5. **拾取是 view-local 射线**，不是扁平 hit map（`SPEC-RENDER-023`）；CPU 回退成本必须显式
   （`SPEC-RENDER-024`）。

## 依据

- `SPEC-SCOPE-015`：3D 是布局叶节点，不是引擎。
- `SPEC-RENDER-019`…`-024`：最小承诺、禁止第二窗口、generation、glTF 是 SPEC、拾取、CPU 回退。
- `CLAIM-070`：现有外部纹理是固定绿色 `Clear` 占位；`CLAIM-071`：depth 是 renderer 内部 view；
  `CLAIM-072`：无 glTF/vertex/index/camera/PBR；`CLAIM-078`：独立 3D depth 未实现。
- `EVID-VAL-15`：sparse-strips headless readback 成功，但 `CLAIM-073` 明确它不证明 retained/窗口；
  `EVID-VAL-15` 更不能证明 3D。

## 影响与撤销条件

**影响**

- 3D 的窗口/输入/焦点仍走同一 `WindowShell`，不得自建事件循环。
- 资源生命周期必须按 `View generation` 校验；迟到 completion 只能原子替换，不能写旧目标。
- 产品与市场材料不得把"绿色占位/纹理互通"写成"支持 glTF 或 PBR"。

**撤销条件（必须同时满足）**

1. `SPEC-GATE-D-01` 与 `-02` 连续失败，且 trace 证明失败原因是"叶节点 + 独立离屏"无法满足某个**已声明**
   的产品场景（不是新增需求）；
2. 独立窗口/呈现方案能给出更少的 device/surface 生命周期问题，并仍满足 `SPEC-RENDER-020` 之外的
   全部 A/B/C Gate；
3. 该方案不把 ECS/脚本运行时引入核心依赖（否则违反 `SPEC-SCOPE-014`）。

仅"3D 性能不够"或"游戏引擎更方便"不构成撤销依据。
