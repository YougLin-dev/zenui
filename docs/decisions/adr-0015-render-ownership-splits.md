---
doc-id: ADR-0015
layer: decisions
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - docs/spec/06-render.md（SPEC-RENDER-011、SPEC-RENDER-016）
  - docs/spec/05-text.md（SPEC-TEXT-015）
  - docs/spec/02-runtime.md
  - docs/spec/04-style.md
replaces: []
---

# ADR-0015：渲染线三处归属切分

## 背景

整理渲染/文本规范时出现三处"同一事实被两个文件同时定义"的风险：

1. **隐藏 / 禁用 / inert**：既涉及"何时 dispose、何时允许保留"的生命周期，也涉及"是否参与 layout/paint/
   hit/a11y"的渲染后果；
2. **颜色与 format**：既涉及 token/declaration 的声明形态（样式层），也涉及线性化、premultiply、blend 与
   surface capability（渲染层）；
3. **caret / selection**：坐标空间与公开单位同时被文本层、运行时与编辑器适配层引用。

若两处都写成规范条文，后续任一变化都会出现两个权威。队长已裁决按"语义归属"切分，并在 spec 中写入
权威归属注记（`SPEC-RENDER-016`、`SPEC-RENDER-011`、`SPEC-TEXT-015`）。本 ADR 记录该切分及其边界。

## 备选项

1. **全部写在渲染/文本文件**：读者集中，但生命周期与声明形态的事实会在 RT/Style 里再定义一次。
2. **按语义归属切分，另一侧只保留后果并引用 ID**（选中）。
3. **全部上移到 RT/Style**：渲染/文本文件失去可执行的后果描述，实现者仍需跨文件拼接。
4. **不写权威归属，靠交叉引用**：冲突时无法判断哪一方胜出。

## 决定

1. **生命周期归 `SPEC-RT`**：`Hidden`、`visibility:hidden`、`Disabled`、`Inert`、clip 的"何时 dispose、
   何时允许保留"由 `SPEC-RT` 定义（`SPEC-RT-024`、`SPEC-RT-026`）；
   `SPEC-RENDER-016` 只规定渲染侧后果：是否参与 layout、是否绘制、是否命中、是否进入语义树。
2. **声明形态归 `SPEC-STYLE`**：颜色 token、alpha 表示、`StyleIR` 字段等"声明长什么样"由
   `SPEC-STYLE` 定义（`SPEC-STYLE-002`、`-004`、`-052`、`-055`）；
   `SPEC-RENDER-011` 只规定管线工作空间：线性化、premultiply、blend、surface format 与 capability 校验。
3. **坐标单位归 `SPEC-TEXT`**：caret/selection 的坐标空间与公开单位（grapheme/char，不是 glyph）由
   `SPEC-TEXT-011`、`SPEC-TEXT-015` 唯一持有；`SPEC-RT` 与编辑器适配层只能引用，不得另立单位或换算规则。
4. **禁止复制**：上述三处在另一侧只能写"见 `SPEC-*`"，不得复述定义；发现两处定义即为文档缺陷。
5. **切分不改变行为**：本决定只解决"谁定义"，不改变任何已裁定的生命周期、颜色或 caret 语义。

## 依据

- `SPEC-RENDER-016` 的"权威归属（队长裁决）"：生命周期归 `SPEC-RT`，本条只保留渲染后果。
- `SPEC-RENDER-011` 的"权威归属（队长裁决）"：token/declaration 形态归 `SPEC-STYLE`，本条只保留管线工作空间。
- `SPEC-TEXT-015` 的"权威归属（队长裁决）"：坐标空间与公开单位由 `SPEC-TEXT` 唯一持有。
- `SPEC-RT-004`：三类失效必须解耦，不得互相代偿——这是"生命周期归 RT"的结构依据。
- `SPEC-STYLE-004`：一个字段只有一个权威；`SPEC-STYLE-052`：颜色必须携带色空间；
  `SPEC-STYLE-055`：颜色空间与 surface 能力归渲染层——两处边界正好对应本条第 2 项。
- `SPEC-TEXT-011`：五个坐标集必须分离；`CLAIM-079`：IME/caret/selection 的真实转换尚未验证，因此
  单位必须由文本层唯一持有，不能由平台适配器各自解释。
- `CLAIM-067`：Classic 与 sparse-strips 的 alpha 约定不同；`CLAIM-073`：readback 不证明窗口/缓存——
  这两条说明渲染侧必须保留管线后果，不能把声明形态与合成语义合并。

## 影响与撤销条件

**影响**

- 修改隐藏/禁用语义时，先在 `SPEC-RT` 改生命周期，再检查 `SPEC-RENDER-016` 的后果是否仍成立；
  反之亦然。
- 修改颜色 token 或 alpha 表示时，先改 `SPEC-STYLE`，`SPEC-RENDER-011` 只验证管线能力。
- 任何 caret/selection 单位换算只能引用 `SPEC-TEXT-011/015`；AccessKit char range 与 IME 偏移必须经同一 adapter。
- 文档交叉检查时，这三处是重点：出现第二份定义即阻断合并。

**撤销条件（可判定）**

1. `SPEC-GATE-B-08`（四路同源几何）或 C Gate 的 AT trace 显示同一语义在两个 owner 下产生**不同结果**
   （例如 RT 的 dispose 规则与 render 的 hit/a11y 后果冲突，或 text 的 caret 单位与 AT char range 不一致），
   且无法通过引用修复；
2. 或某次实现证明该切分造成"无人拥有"的可观测缺口（两个文件都不定义，导致同一输入在不同平台行为不同），
   并有最小复现；
3. 重开时必须指定唯一 owner，并保留旧 ID 的映射；不得恢复"两处都定义"的状态。

仅"文件看起来分散"或"读者要跨文件"不构成撤销依据。
