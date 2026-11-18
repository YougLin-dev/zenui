---
doc-id: SPEC-TEXT
layer: spec
status: draft
date: 2026-11-18
owner: 渲染与 GPU 抽象负责人
sources:
  - archive/review/FINAL.md
  - archive/review/text-layout-review.md
  - archive/review/VALIDATION.md
  - archive/research/runtime-and-rendering.md
  - archive/research/stack-decisions.md
replaces: []
---

# SPEC-TEXT 文本、字形与编辑

> 本文件定义**要建什么**，不声明已实现。事实状态归 `docs/evidence/` 的 `CLAIM-*` 与 `EVID-VAL-*`；本文件只给出探针路径与文件行号，便于 evidence 层归并，**不复述版本号**。
>
> 版本 pin 的唯一出处是 [spec/01-route.md](01-route.md)（`SPEC-ROUTE`）。本文提到的 crate 名不携带版本。
>
> 状态词只使用 `SPEC / PROBE / VERIFIED / WITHDRAWN / OPEN`；spec 层不使用 `VERIFIED`。

## 0. 责任边界：谁拥有什么，谁不能碰谁

| 层 | 拥有的东西 | 只接收 / 只读 | 禁止事项 |
|---|---|---|---|
| **布局（Taffy 投影）** | box 几何、私有 `NodeId` 映射、dirty 标记、padding/border/min-max 展开 | StyleIR 投影后的 `Style`、文本叶子的测量回调 | 解析字形、持有 `parley::Layout`、把 `NodeId` 暴露给业务层、手算 text leaf 的 `Size` |
| **文本（Parley shaping）** | shaping、bidi 重排、按宽度断行、字体 fallback、`run.synthesis()` 合成标记、`first_baseline()` | paragraph descriptor + 有效宽度 + 字体/尺度 epoch | 直接画像素、持有 surface、决定 box 尺寸 |
| **编辑器（zenui-text）** | selection、composition、revision/epoch、五个坐标集之间的转换 adapter | committed value/origin ticket | 直接写 Store、直接触发 UI 提交 |
| **渲染（zenui-render）** | 自有 retained paint records、paint page 缓存、SceneAdapter | **渲染无关**的 glyph run 记录（字体句柄、尺寸、normalized coords、glyph id/位置、颜色） | 读取 `parley::Layout` 内部结构、公开 Vello `Scene`/`TextureId`/`wgpu::SurfaceTexture` |
| **光栅化（Vello GPU + CPU reference）** | GPU/CPU 的像素输出 | 场景/绘制记录 | 决定布局与断行 |
| **呈现（wgpu surface / WindowShell）** | swapchain、present、DPI、设备生命周期 | 已成功提交的帧 | 做 shaping、布局或文本编辑 |

数据流（单向，不允许反向控制）：

```text
StyleIR typography ──projection──▶ ParagraphDescriptor(+epoch)
      │
      ▼
Taffy measure callback(LayoutInput/known/available) ──▶ LayoutOutput{size, baselines}
      │
      ▼
Parley shaping + break at accepted width ──▶ Layout{positioned glyph runs, first_baseline}
      │
      ▼
zenui-text 序列化为渲染无关 glyph run 记录
      │
      ▼
zenui-render retained paint record ──▶ Vello GPU 光栅 / Vello CPU reference
```

**铁律**：Taffy 的测量回调 MUST 产出一个临时 `LayoutOutput`；MUST NOT 在测量中修改任何 retained `ViewId` 或触发提交。文本层 MUST NOT 读取布局树，渲染层 MUST NOT 读取文本层内部布局对象。

## 1. 测量与布局桥

### SPEC-TEXT-001 Taffy leaf 回调是文本测量的唯一入口

- **状态**：PROBE
- **强度**：MUST
- **要求**：所有文本叶子的测量经 `compute_layout_with_measure` 的回调完成，回调签名为 `(LayoutInput, NodeId, Option<&mut NodeContext>, &Style) -> LayoutOutput`；普通 leaf 的 padding/border/min-max/replaced 由 `compute_leaf_layout` 传播，不得在回调里手算最终 `Size`。
- **失败模式**：绕过 `compute_leaf_layout` 直接返回文本尺寸时，padding/border 被吞掉，导致绘制矩形、命中矩形与无障碍矩形三者不一致。
- **来源**：`examples/native_poc/src/layout.rs:95-105`；FINAL §7.1。

### SPEC-TEXT-002 四种测量模式必须分开

- **状态**：SPEC
- **强度**：MUST NOT / MUST
- **要求**：min-content、max-content、definite known dimensions、available space 是四种不同测量模式，MUST NOT 合并成单个 `Option<f32>`；`known_dimensions` 与 `available_space` 分别决定早退与断行。
- **失败模式**：用同一个 `Option<f32>` 近似时，flex shrink 与 grid intrinsic 在窄宽度下高度算错；单像素偏差可复现为文字被裁或容器被撑开。
- **来源**：FINAL §7.1；VALIDATION §6；`text-layout-review.md` P1。

### SPEC-TEXT-003 baseline 必须来自最终 accepted layout

- **状态**：OPEN（要求明确，实现未通过）
- **强度**：MUST
- **要求**：`LayoutOutput.baselines` MUST 由**最终 accepted text layout** 的 `first_baseline()` 加 content top 与外层 padding/border 计算得出；known-size、命中 cache、早退路径 MUST 使用同一 epoch，MUST NOT 从 stale measurement 补 baseline。
- **失败模式**：baseline 丢失时 flex/grid 中的文本基线对齐错误，单行文本在 `align-items: baseline` 下垂直错位，行内多字体混排的首行下移。
- **现状（OPEN）**：探针在 `examples/native_poc/src/layout.rs:27` 构造了 `Baselines::from_first(...)`，但同文件 `:104` 的回调只把 `.size` 交给 `compute_leaf_layout`，因此叶子最终输出 `Baselines::NONE`。现有测量测试为绿，不代表 baseline 正确。
- **来源**：`text-layout-review.md`；VALIDATION §6；详见本文 §3 `OPEN-TEXT-01`。

### SPEC-TEXT-004 文本/字体/尺度变化必须显式失效

- **状态**：PROBE
- **强度**：MUST
- **要求**：文本内容、字体 alias、字号、DPI/scale、可用宽度模式变化时，MUST 通过 `set_node_context` 或显式 `mark_dirty` 使布局失效；MUST NOT 依赖 `NodeContext` 内部可变状态被“自动发现”。每个 style/text/font epoch 变化都要进入失效路径。
- **失败模式**：原地修改段落内容后 Taffy 仍用旧缓存，出现新旧文字重叠、命中区域与可见文字错位。
- **来源**：`examples/native_poc/src/layout.rs:65-88`（探针断言 `set_node_context` 后 dirty 且尺寸变化）。

## 2. shaping、缓存与字体

### SPEC-TEXT-005 shaping 缓存 key 必须覆盖输入全集

- **状态**：SPEC
- **强度**：MUST
- **要求**：字形/断行缓存 key 至少包含：具体 font face（含 variation 轴）、weight、synthesis 标志、font size、normalized coords 的精确 bits、accepted width、断行模式、DPI/scale epoch、shaping engine 版本。`NormalizedCoord → i16` MUST 使用显式 `to_bits()` 类转换，不得隐式截断。
- **失败模式**：只按字号做 key 时，切换 weight 或 scale 后复用旧字形，出现“加粗无效”“换 DPI 后文字发虚/错位”。
- **来源**：VALIDATION §2；FINAL §7.1。

### SPEC-TEXT-006 Font synthesis 必须转发到光栅层

- **状态**：OPEN（实现未接入）
- **强度**：MUST
- **要求**：Parley `run.synthesis()` 产生的 embolden/skew 语义 MUST 转发给 glifo 的字形光栅；cache key 必须包含真实 face + weight + synthesis 标志 + scale。
- **失败模式**：只有 `font_size`/`normalized_coords` 时，缺少真 bold face 的文本渲染成常规字重，视觉与度量都不符合声明样式。
- **现状（OPEN）**：`examples/native_poc/src/text.rs:34-63` 的 `draw_gpu` 只传 `font_size` 与 `normalized_coords`，全仓 `grep -rn "synthesis"` 无命中。
- **来源**：`text-layout-review.md`；VALIDATION §6；本文 `OPEN-TEXT-03`。

### SPEC-TEXT-007 mixed-bidi intrinsic 宽度不得默认正确

- **状态**：OPEN
- **强度**：MUST
- **要求**：混合 bidi / 多脚本文本的 min/max-content 必须视为**未决能力**：保留 canary 与显式 fallback（例如退化为保守宽度或标 `Unsupported`），并在 B Gate 用固定字体与 ICU artifact 做差分。MUST NOT 以“`calculate_content_widths()` 已调用”作为完整 bidi 宽度支持的证据。
- **失败模式**：RTL 与 LTR 混排行在 shrink 容器中出现裁切或意外的宽高跳变；同一字符串在 CPU reference 与 GPU 路径的换行点不同。
- **来源**：FINAL §7.1；VALIDATION §3/§6；`runtime-and-rendering.md` §4。

### SPEC-TEXT-008 CJK/ICU 字典资产必须随发布物提供

- **状态**：PROBE（行为已验，打包未验）
- **强度**：MUST
- **要求**：CJK/日文的 ICU4X segmentation dictionary models MUST 存在于发布 artifact 中并被实际加载；缺失时 MUST 给出明确诊断，而不是静默退化为按字符切分。
- **失败模式**：只按英文设计文本时，中文断行按单字切分；首次运行出现 `ICU4X data error: No segmentation model for Chinese/Japanese`。
- **来源**：`examples/native_poc/src/lib.rs:45-61`（`complex_script_dictionary_actually_changes_word_boundaries`）；VALIDATION §1。

### SPEC-TEXT-009 渲染层只接收渲染无关的 glyph run 记录

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：文本层向渲染层交付的记录只包含：字体句柄/face 引用、字号、normalized coords（bits）、glyph id 与位置、颜色/画笔；MUST NOT 把 `parley::Layout`、glifo 内部 atlas 句柄或 Vello 类型穿过边界。渲染层不得回读 shaping 结构。
- **失败模式**：渲染层直接借用 `parley::Layout` 生命周期时，文本编辑或重排会使绘制记录悬垂；升级 Parley 会连锁破坏渲染层。
- **来源**：FINAL §7.1/§7.2；`runtime-and-rendering.md` §5。

### SPEC-TEXT-010 字体 fallback 与 emoji 必须可复现

- **状态**：OPEN
- **强度**：SHOULD
- **要求**：字体 fallback 顺序与结果 MUST 在固定字体 artifact 下可复现并记录；emoji 的 shaping/光栅（彩色或单色）必须明确支持状态，不得默认“能显示”即通过。
- **失败模式**：同一文档在开发机与发布机上 fallback 到不同字体，行宽与断行不同；emoji 显示为豆腐块或颜色错误。
- **来源**：FINAL §7.1；VALIDATION §3；`CONCLUSION.md` §通过验证。

## 3. 文本编辑、IME 与坐标

### SPEC-TEXT-011 五个坐标集必须分离

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：UTF-8 bytes（Rust 字符串）、UTF-16/native OS offsets、Unicode grapheme、Parley glyph position、AccessKit char range 是五个不同坐标集，各自为独立 newtype；任何选取/插入/删除 MUST 经编辑器 widget 提供的同一个 adapter 转换。MUST NOT 把 glyph index 当 selection，也不得把 byte offset 直接交给 AT。
- **失败模式**：中文/emoji 文本中删除一个字符却删掉半个码点或两个 grapheme；AccessKit 报出的选区偏移与可见插入符错位。
- **来源**：FINAL §5；`runtime-and-rendering.md` §1。

### SPEC-TEXT-012 TextEdit 是双相位单一 reducer

- **状态**：SPEC
- **强度**：MUST
- **要求**：会话态 `session{editor_id, editor_generation, editor_revision, value, selection, composition}` 与提交态 `committed{epoch, value, origin}` 分离但共用一个 reducer；widget 持有 selection/composition，因此不能只存 `String`。本地 committed input 经 origin ticket 回写 Store；回到 origin editor 时只 ack epoch，不发起 ExternalReplace。
- **失败模式**：只用 `String` 建模时，输入法组合态、选区与外部替换互相覆盖；同一输入触发两次 replace，光标跳回行首。
- **来源**：FINAL §5。

### SPEC-TEXT-013 外部替换与本地更新走同一路径

- **状态**：SPEC
- **强度**：MUST
- **要求**：外部 set/load/undo 通过 `ExternalReplace{value, origin, revision, SelectionPolicy::{PreserveClamped, SetRange, CollapseEnd}}` 与本地更新进入同一 reducer；相同 epoch 只有最后 committed 值生效，过期 ticket 丢弃。`value={draft}` MUST 只经专属 `apply_store_revision` 进入编辑器。
- **失败模式**：外部替换绕过 reducer 时，编辑器与 Store 形成两份互相不知情的权威，保存后内容回退。
- **来源**：FINAL §5。

### SPEC-TEXT-014 IME composition 与 Enter 的消费顺序

- **状态**：SPEC
- **强度**：MUST
- **要求**：候选窗口激活时，Enter MUST 先被输入法/编辑器消费，不得触发表单提交；该次确认结束后的下一次普通 Enter 才 submit 一次。composition 期间的外部替换默认走显式“先提交本地 composition 再替换”，并提供显式 cancel。底层 `set_text()` 一类会取消 composition 的调用 MUST NOT 被字段更新随意复用。
- **失败模式**：中文输入选词回车时误提交表单；组合串被外部更新打断后丢失已输入内容。
- **来源**：FINAL §4/§5；A1 验收文字。

### SPEC-TEXT-015 caret/selection 的公开单位是 grapheme/char，不是 glyph

- **状态**：SPEC
- **强度**：MUST
- **要求**：公开 API、AT action 与序列化选择范围使用 grapheme 或 char 语义；glyph position 只存在于文本层内部，用于绘制。caret 的几何 MUST 从同一 `GeometrySnapshot` 与同一 accepted layout 计算。
- **失败模式**：连字或组合字符中 caret 落在 glyph 中间；点击选择时选到半个字。
- **来源**：FINAL §5/§7.1。
- **权威归属（队长裁决）**：caret/selection 的**坐标空间与公开单位**由本条（`SPEC-TEXT`）唯一持有；`SPEC-RT` 与编辑器适配层只能引用本条，不得另立一套单位或坐标换算规则。

## 4. OPEN 缺陷台账（不得被“已通过”覆盖）

| ID | 缺陷 | 证据位置 | 为什么现在不能算通过 |
|---|---|---|---|
| **OPEN-TEXT-01** | baseline 丢失：`measure()` 构造了 baselines，但回调只传 `.size` | `examples/native_poc/src/layout.rs:27` vs `:104` | 现有测试只断言高度/宽度，没有断言 `Baselines`；叶子输出为 `Baselines::NONE` |
| **OPEN-TEXT-02** | intrinsic 未证明：min/max-content 与 known-size 早退未分开验证 | `layout.rs:100-105`；VALIDATION §6 | 探针只跑 `MaxContent` 与少量宽度，`calculate_content_widths()` 早退与 `break_all_lines(Some(0))/None` 近似不能证明收缩/wrap/bidi |
| **OPEN-TEXT-03** | Font synthesis 未转发 | `examples/native_poc/src/text.rs:34-63`；全仓无 `synthesis` 引用 | 假 bold/oblique 与对应 cache key 完全缺失 |
| **OPEN-TEXT-04** | mixed-bidi intrinsic 无保证 | FINAL §7.1 | 公开文档未充分保证，必须保留 canary/fallback 并做差分 |
| **OPEN-TEXT-05** | emoji 与确定性 fallback 资产未验 | VALIDATION §3 | 没有固定字体 artifact 与跨机复现记录 |
| **OPEN-TEXT-06** | IME/caret/selection/AT 坐标转换未验 | FINAL §5；VALIDATION §3 | 没有真实 IME trace 与 AccessKit char range 对照 |
| **OPEN-TEXT-07** | 字形 atlas cache 关闭 | `text.rs` 的 `.atlas_cache(false)` | 官方标注实验性；正确性优先，性能与缓存命中断言均未建立 |

**收敛规则**：上述条目只有在 evidence 层出现可复跑的 `EVID-VAL-*`（命令 + 输出 + 主机）后才能从 `OPEN` 改为 `VERIFIED`；仅“代码改了一行”或“别的测试是绿的”不构成关闭依据。

## 5. 与其他规范的关系

- 版本 pin：只引用 [SPEC-ROUTE](01-route.md)，本文不重复。
- 字体/文本相关的样式投影与不支持诊断：见 `SPEC-STYLE`（[spec/04-style.md](04-style.md)）。
- 文本的绘制记录、合成与 3D 叠加：见 [spec/06-render.md](06-render.md)（`SPEC-RENDER-*`）。
- baseline/intrinsic/synthesis 的 Gate 归属：B Gate（[spec/08-gates.md](08-gates.md)，`SPEC-GATE-*`）。
