---
doc-id: SPEC-STYLE
layer: spec
status: normative
date: 2026-11-18
owner: 顶级 Rust 开发工程师（10 年+）
sources:
  - archive/review/FINAL.md
  - archive/research/style-system.md
  - archive/research/tailwind-coverage.md
  - archive/review/VALIDATION.md
replaces: []
---

# SPEC-STYLE 样式：StyleIR、优先级、条件与支持等级

> 本文件定义**要建什么**，不声明已实现。事实状态归 `docs/evidence/` 的 `CLAIM-*`、`EVID-VAL-*` 与 `EVID-MATRIX`；本文件只定义语义、失败模式与等级词表。
>
> 版本 pin 的唯一出处是 [spec/01-route.md](01-route.md)（`SPEC-ROUTE`）。本文提到“固定上游版本”“pinned registry”时一律指 `SPEC-ROUTE-001` 的 pin，**本文不写任何版本号或 commit hash**。
>
> 状态词只使用 `SPEC / PROBE / VERIFIED / WITHDRAWN / OPEN`；spec 层不使用 `VERIFIED`。历史材料 `archive/research/style-system.md` 与 `FINAL` 冲突时以 `FINAL` 为准，被取代的表述收在 §7。

## 0. 责任边界

| 层 | 拥有 | 只接收 / 只读 | 禁止事项 |
|---|---|---|---|
| **样式编译器（zenui-style）** | `UtilityAst`、canonical declaration、`StyleProgram`、优先级排序、诊断 | 字面量 candidate（宏或 typed API）、固定上游 registry | 执行 CSS selector / 运行时字符串解析 / 访问网络 |
| **宏（SPEC-MACRO）** | `class` 字面量的采集与转述诊断 | style 编译器输出的 IR 与错误 | 自行解释 utility、自行决定优先级、运行时候选 |
| **布局 adapter（SPEC-TEXT / SPEC-RENDER）** | Taffy `Style` 投影、几何、`GeometrySnapshot` | 投影后的 layout 字段 | 回读 class、修改 StyleProgram、测量中改样式 |
| **文本（SPEC-TEXT）** | Parley 段落描述符、字体、行高、断行 | layout-text 字段 | 解析颜色/单位字符串、决定 paint 字段 |
| **渲染（SPEC-RENDER）** | paint 字段（color/fill/stroke/shadow/filter/opacity/transform 合成）、能力门 | 已解析的 typed paint 值 | 采信运行时候选、绕过 `RenderCapabilities` |
| **Store / 运行时（SPEC-RT）** | 状态来源、epoch、事务提交 | condition 状态位、token 变更事件 | 持有 StyleProgram、重排声明 |

**唯一链路**：

```text
固定上游 registry + 字面量 candidate / typed inline
  → UtilityAst{base, value, modifier, variants, important, span}
  → canonical declaration{property_mask, value, condition, origin, precedence, source}
  → 不可变 StyleProgram（节点/组件级）
  → condition / token / theme epoch 变化 → 只 patch 受影响字段（StylePatch）
```

**两条铁律**：样式 MUST 在编译期完成 candidate → declaration 的解析；运行时 MUST 只做“条件命中 → 字段 patch”，MUST NOT 重新解析 class 字符串。

## 1. StyleRule 的形状与分发

### SPEC-STYLE-001 单一编译链，拒绝运行时 CSS 解释器

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：utility candidate MUST 只经“字面量 → `UtilityAst` → canonical declaration → 不可变 `StyleProgram`”这一条链路。MUST NOT 在每帧执行 `class.split(' ')`、MUST NOT 在运行时运行 selector 解释器、MUST NOT 把字符串直接交给 Taffy。
- **失败模式**：运行时候选解释 → 每帧分配且未知 token 静默失效；性能与正确性同时不可证伪。
- **来源**：FINAL §6；`style-system.md` §1；`SPEC-ROUTE-028`。

### SPEC-STYLE-002 `StyleRule` 必须是字段级规则，不是节点级快照

- **状态**：SPEC
- **强度**：MUST
- **要求**：每个 declaration MUST 表示为一条字段级规则：

```rust
struct StyleRule {
    canonical: CanonicalRuleId,
    property_mask: PropertyMask,   // 只覆盖本规则真正改写的子字段
    declaration: Declaration,      // 已规范化的 typed 值
    condition: StyleCondition,
    origin: StyleOrigin,           // theme | intrinsic | class | inline | framework
    important: bool,
    precedence: Precedence,        // (layer, variant_rank, utility_order, declaration_index)
    source: SourceSpan,            // 诊断与 provenance，不参与排序
}
```

- 规则 MUST NOT 携带“整节点样式”快照，MUST NOT 要求先合并成一个大结构再分发。
- `source` / provenance MUST 只用于诊断与调试，MUST NOT 进入运行时排序键。
- **失败模式**：一条 class 覆盖整个 style 表 → 无法做字段级 patch，hover 一个颜色触发全量 rebuild；provenance 被当作排序依据 → 样式结果随日志字段变化。
- **来源**：FINAL §6；`style-system.md` §2。

### SPEC-STYLE-003 `property_mask` 必须是完备的子字段展开

- **状态**：SPEC
- **强度**：MUST
- **要求**：以下简写 MUST 展开为**完备**的子字段 mask，不得用整表覆盖近似：

| 族 | 必须展开到 |
|---|---|
| padding / margin | 四边 + 轴（`p`/`px`/`py`/`pt`/…）独立字段 |
| border | 宽度按边、颜色按边、style 按边 |
| border-radius | 四角独立 |
| transform | translate / scale / rotate / skew 分部件 |
| shadow / ring | 离屏层 + offset/alpha/blur 分项 |

- **MUST NOT** 出现“`.styles` 整表覆盖”或“按 class 名字猜覆盖关系”（`SPEC-SCOPE-011`）。
- **失败模式**：`border-l-4` 实际改写了四边；hover 只 patch 了 padding 的一侧，布局在悬停时轻微跳动。
- **来源**：FINAL §6；`tailwind-coverage.md` §3.1/§3.3。

### SPEC-STYLE-004 分发目标固定为三域，一个字段只有一个权威

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：每条 declaration MUST 明确投向且仅投向一个目标域：

| 目标域 | 内容 | 消费方 |
|---|---|---|
| **layout** | display、flex/grid、gap、padding/margin、size、position/inset、order、z（参与布局） | 布局 adapter → Taffy `Style` |
| **layout-text** | font family/size/weight、line-height、letter-spacing、wrap/break、align、direction、decoration 度量 | 文本层 → Parley 描述符 |
| **paint** | color、fill/stroke、background、border 颜色、radius 形状、opacity、transform 合成、shadow/ring、filter | 渲染层 → retained paint 字段 |

- 跨域共享的输入（例如字号同时影响布局与字形）MUST 以 **layout-text** 为权威，paint 只读派生值；MUST NOT 让两个域各自维护一份可独立修改的字号。
- **失败模式**：颜色改动进入 layout cache key → 换色触发全量重排；字号在两个域不同步 → 命中矩形与可见文字错位（`SPEC-TEXT-001`）。
- **来源**：FINAL §6；`style-system.md` §2；`SPEC-TEXT` §0 边界表。

### SPEC-STYLE-005 `StyleProgram` 不可变，状态变化只产字段级 `StylePatch`

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：`StyleProgram` MUST 是不可变的编译产物（静态 metadata + 节点局部动态绑定槽）。条件或 token 变化 MUST 只产生 `StylePatch`（mask 内新值）；MUST NOT 重建节点、MUST NOT 重新解析候选、MUST NOT 重建整棵树。
- **失败模式**：主题切换触发整树 rebuild → 局部状态（焦点、滚动、caret）丢失；hover 每帧重算全部 class。
- **来源**：FINAL §6（“condition/token epoch 仅触发该字段 StylePatch”）；`style-system.md` §1/§7。

### SPEC-STYLE-006 失效范围必须按依赖反查

- **状态**：SPEC
- **强度**：MUST
- **要求**：每个节点 MUST 记录其 StyleProgram 实际使用的 condition 与 token 依赖；theme / root size / text epoch / 平台偏好变化 MUST 只反查依赖节点并 patch 受影响 mask，MUST NOT 使无关节点失效。
- **失败模式**：改一个主题 token 导致全窗口重排；或相反，依赖未被记录 → 主题切换后部分控件保持旧色（脏传播漏边）。
- **来源**：FINAL §6；`style-system.md` §2 末段。

### SPEC-STYLE-007 字面量 class 的编译期展开

- **状态**：PROBE（仅极小词表与未知类诊断）
- **强度**：MUST / MUST NOT
- **要求**：`class` MUST 是编译期字面量，逐 token 交给样式编译器（宏侧 `SPEC-MACRO-013`）；未知 token MUST 编译错误，MUST NOT 静默忽略。编译产物 MUST 落到 `StyleProgram`，MUST NOT 以字符串形式进入运行时。
- **失败模式**：未知 class 被忽略 → “样式没生效”且无诊断；字面量被当运行时候选保存 → 每帧解析。
- **现状（PROBE）**：现有宏探针能解析少量 static literal 并锁定未知 class 的整串诊断；这不构成词表、variant 或优先级的完成。
- **来源**：FINAL §6；`style-system.md` §1/§7；`tailwind-coverage.md` §6。

## 2. 优先级：单一规则

### SPEC-STYLE-010 优先级比较只有一条规则

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：普通作者 CSS（无 `!important`）下，排序 MUST 按以下固定层次比较：

```text
1. origin / importance
     （author inline 与 class rules 在此层分离）
2. 仅在 class rules 内部：canonical variant order / utility order
3. 仅当 origin、importance、canonical order 都相同：source order
```

- **MUST NOT** 先选“当前最活跃的条件”再回头比较 origin；MUST NOT 以“最后一个 patch 胜”作为通用规则。
- **失败模式**：hover 命中时组件默认背景越过 author inline → 主题与作者意图冲突且结果随事件顺序变化。
- **来源**：FINAL §6 优先级小节；CSS Cascade 的 origin/importance 概念。

### SPEC-STYLE-011 ordinary inline 在 hover=false / true 均胜出

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：ordinary（非 `!important`）inline 声明 MUST 在 hover 不命中与命中两种情况下都胜过**同属性**的普通 class 与 hover 变体声明。CSS origin 优先于状态激活；MUST NOT 让 condition 的命中改变 origin 比较结果。
- **已撤销**：早期候选“激活后的 condition 可跨 origin 胜过 ordinary inline”= `WITHDRAWN`，依据 `SPEC-ROUTE-023` 与 FINAL §6 / §8。任何实现都 MUST NOT 以第二条优先级规则复活它。
- **失败模式**：组件传入 `style:background={brand}`，悬停时却被 class 的 `hover:bg-*` 改掉 → 同一控件在不同交互阶段的视觉归属不可预测。
- **来源**：FINAL §6 / §8；`SPEC-ROUTE-023`。

### SPEC-STYLE-012 condition 命中只决定 eligible，不是排序键

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：condition 是否命中 MUST 只决定该 declaration 是否参与比较（eligible）。多个条件同时命中（例如 hover + pressed）时，结果 MUST 由 canonical variant order 与 source order 决定，MUST NOT 由“谁后激活 / 谁最后 patch”决定。
- **失败模式**：同一元素 hover + active 时样式取决于事件到达顺序，鼠标小幅移动即抖动。
- **来源**：FINAL §6（“不是先选最活跃条件再回头看 origin”）。

### SPEC-STYLE-013 `!important`、layer 与 transition 状态在冻结前不得启用

- **状态**：OPEN
- **强度**：MUST NOT
- **要求**：`!important`、显式 layer、transition/animation 中间状态的完整排序 MUST 在固定上游版本的差分 fixture 冻结后才能启用。在此之前 MUST 明确诊断拒绝或登记为 `OPEN`，MUST NOT 按 source order 近似后声称支持。
- **失败模式**：`!important` 被静默当作普通声明 → 与上游生成的 CSS 输出不一致，用户迁移时样式反转。
- **来源**：FINAL §6；`tailwind-coverage.md` §3.4/§3.5；`OPEN-STYLE-02`。

### SPEC-STYLE-014 生成顺序来自 canonical utility + variant，不是书写顺序

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：同一元素的 class 声明顺序 MUST 来自 registry 的 canonical utility 顺序与 variant 顺序，MUST NOT 来自属性书写顺序、MUST NOT 来自 appearance 字段的内部顺序。只有在 origin / importance / canonical order **完全相同**时才比较字面量内的 source order（later-wins）。
- **失败模式**：把 `class="p-4 pt-2"` 改成 `class="pt-2 p-4"` 结果变化（或本应变化却不变化），与上游行为分叉。
- **来源**：FINAL §6（“Compiler order 是 canonical utility+variant，而不是在 attrs walk 时后写 wins”）。

## 3. 条件到原生状态的映射

### SPEC-STYLE-020 condition 是登记枚举，不是 selector 字符串

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：`StyleCondition` MUST 是样式编译器与运行时共享的登记枚举（形态见 §1）；MUST NOT 接受任意字符串 selector、MUST NOT 在运行时解释 `:hover > *` 一类表达式。

```rust
enum StyleCondition {
    Always,
    Node { hover, focus, focus_visible, pressed, disabled, checked, indeterminate, /* … */ },
    Ancestor { group: GroupId, state: AncestorState },
    Theme(ThemeId),
    Root { width: Range<LogicalPx>, height: Range<LogicalPx>, /* … */ },
    Container { id: ContainerId, query: ContainerQuery },
    Direction(TextDirection),
}
```

- **失败模式**：字符串条件被接受但永不命中（无样式、无报错）；或需要运行时解析导致帧内分配。
- **来源**：FINAL §6；`style-system.md` §2。

### SPEC-STYLE-021 状态条件的权威是控件 typed state

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：每个状态条件 MUST 由控件/树提供的 native state 派生，MUST NOT 由另一套并行 bool 驱动：

| 条件 | 唯一来源 |
|---|---|
| `hover` | 指针命中的 hover 状态（含 clip 约束） |
| `focus` | 控件持有键盘/程序焦点 |
| `focus-visible` | 焦点来源的 modality（键盘/非指针），见 `SPEC-STYLE-023` |
| `focus-within` | 子树内任一控件持有焦点 |
| `active` | native Pressed（按下瞬态），见 `SPEC-STYLE-022` |
| `disabled` / `enabled` | 控件语义 disabled 字段（不是样式 bool） |
| `checked` / `indeterminate` | 控件 typed state，与语义树同源 |
| `aria-pressed` / `aria-*` | 控件语义字段（与视觉同源派生） |

- **MUST NOT** 在应用层维护第二个 bool 只驱动样式。视觉、命中、语义树 MUST 读取同一状态。
- **失败模式**：样式显示“已按下”而 `aria-pressed` 为 false；或 hover 高亮出现在 clip 外不可命中的区域。
- **来源**：FINAL §6；`tailwind-coverage.md` §4。

### SPEC-STYLE-022 `active:` 永久是 native Pressed，不是 selected

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：`active:` MUST 对应“正在按下”的 Pressed 瞬态；MUST NOT 表示 selected、toggled、checked 或应用级选中。toggle/filter 的选中态 MUST 用 `aria-pressed:` 与语义字段同源派生。
- **已撤销**：`class:if-*` / `class:if-selected` 条件语法 = `WITHDRAWN`（`SPEC-ROUTE-022`）；MUST NOT 作为另一套条件语法复活（宏侧禁令见 `SPEC-MACRO-014`）。
- **失败模式**：filter 按钮的“选中”用 `active:bg-*` 实现 → 松开鼠标即恢复，语义状态与视觉不一致；或为了保持视觉给按钮另设独立 bool，与 `aria-pressed` 不同步。
- **来源**：FINAL §6 / §8；`SPEC-ROUTE-022`。

### SPEC-STYLE-023 `focus-visible` 必须区分 modality

- **状态**：SPEC
- **强度**：MUST
- **要求**：`focus-visible` MUST 由“焦点来源是否为键盘/非指针”决定，MUST NOT 用 `focus` 近似（文本输入等控件语义本身要求常驻焦点环的除外，须逐控件登记）。modality MUST 随焦点转移更新。
- **失败模式**：鼠标点击按钮出现焦点环（视觉噪声）；键盘 Tab 导航无焦点指示（无障碍缺陷，C Gate 必挂）。
- **来源**：FINAL §6；`tailwind-coverage.md` §4。

### SPEC-STYLE-024 theme / dark / root size / window breakpoint 的映射

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：
  - `dark:` / `light:` MUST 来自应用 Theme scope 与 token，MUST NOT 依赖浏览器 media query；
  - root font size（`rem` 基准）MUST 显式配置（`RootTheme::font_size`），MUST NOT 随平台 scale 偷改；
  - window breakpoint MUST 使用窗口**逻辑宽度**，MUST NOT 使用显示器尺寸、物理像素或 DPR；
  - 主题/偏好变化 MUST 走 epoch（`SPEC-STYLE-006`）。
- **失败模式**：窗口拖到不同 DPI 显示器后 breakpoint 跳变；dark 切换只改变部分节点（漏依赖）。
- **来源**：FINAL §6；`style-system.md` §4；`tailwind-coverage.md` §4。

### SPEC-STYLE-025 container query 必须在布局阶段反查

- **状态**：OPEN（依赖布局反查能力）
- **强度**：MUST NOT
- **要求**：`@container` 条件 MUST 由布局阶段的 accepted available size 反查得到；MUST NOT 在 measure 回调中读取容器尺寸后**当场修改**节点样式（会造成测量—样式循环）。无法获得可用尺寸时 MUST 视为未命中并记录，MUST NOT 猜测。
- **失败模式**：容器测量中改样式 → 尺寸振荡、无限重排；content-sized 容器上条件永不命中且无提示。
- **来源**：FINAL §6；`tailwind-coverage.md` §4（`@container` 行）；`OPEN-STYLE-03`。

### SPEC-STYLE-026 ancestor / group 条件必须具名

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：`group-*` / ancestor 条件 MUST 绑定具名 `GroupId`/`AncestorId` 与登记状态，MUST NOT 实现 CSS ancestor 文本搜索或选择器匹配。
- **失败模式**：嵌套 group 命中错误祖先；样式随无关祖先状态变化。
- **来源**：FINAL §6；`tailwind-coverage.md` §4。

### SPEC-STYLE-027 DOM 关系型条件默认拒绝

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：`peer-*`、`has-*`、任意 arbitrary variant（如 `[&:nth-child(2)]`）、`nth-child` 任意表达式 MUST 编译期诊断拒绝（`X`），或以具名 native 关系/显式父子状态替代。MUST NOT 静默接受为永不命中。
- **失败模式**：`peer-checked:bg-*` 编译通过但永不生效；用户以为样式写错，实际是能力缺口未报告。
- **来源**：FINAL §6；`tailwind-coverage.md` §4；`style-system.md` §8（诊断示例）。

### SPEC-STYLE-028 系统文本缩放作用于字体度量，不缩放布局单位

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：操作系统级"放大文本"（Windows 文本缩放、macOS/GTK 等价项）MUST 只乘到**字体度量**（paragraph descriptor 的 font size / line height 基准），MUST NOT 乘到 `rem`、间距、圆角、边框或容器尺寸。布局 MUST 在收到缩放后的度量后重新测量 intrinsic size，并根据结果重排。
- **失败模式**：把缩放乘进 `rem` → 间距与控件尺寸一起变大，信息密度崩坏、点击目标错位；或只放大字形不重新测量 → 文字溢出容器、截断。二者都不满足无障碍要求。
- **来源**：FINAL §6；`SPEC-PLAT-WIN-015`；`SPEC-TEXT-006`（intrinsic 与度量）。

### SPEC-STYLE-029 高对比度是 token 层条件，不是控件级补丁

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：高对比度 / forced-colors MUST 作为登记的 token 重映射条件，在 **token → declaration** 这一层整体替换前景、背景、边框与强调色；MUST NOT 由各控件内部判断系统开关后自行改色（否则每新增一个控件就漏一次）。重映射后的任意前景/背景组合 MUST 满足最小对比度，纯色之外（渐变、图片、半透明遮罩）MUST 有确定性回退。
- **失败模式**：开启高对比度后文字与背景同色（自适应控件漏改）；图片上的文字失去可读性；同一条规则在深色主题正确、在高对比度下失效。
- **来源**：FINAL §6；`SPEC-PLAT-WIN-015`；`SPEC-STYLE-024`（theme/dark 同属 token 条件一族）。

## 4. class 面：字面量、动态与 manifest

### SPEC-STYLE-030 动态 class 只接受编译期完整候选

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：动态 class MUST 限定为以下三种之一：
  1. 编译期完整候选的 `enum`（每个变体对应已编译的静态规则集）；
  2. 静态条件表达式（条件本身由登记的 variant 派生，分支候选完整）；
  3. `ClassId` manifest 中已登记的条目（`SPEC-STYLE-031`）。

  `class={expr}` 若不能证明上述任一形态 MUST 编译错误；MUST NOT 接受任意运行时 `String`。
- **失败模式**：运行时拼接的 class 被接受但无样式；或字符串里混入未登记候选，编译期检查全部失效。
- **来源**：FINAL §6；`SPEC-SCOPE-012`；`SPEC-ROUTE-028`；`archive/research/style-system.md` §7。
- **表面语法归属**：本条只定义**语义**（候选必须编译期完整）；动态 class 如何拼写归 [03-macro.md](03-macro.md) `SPEC-MACRO-013` 与 `OPEN-MACRO-09`，本文不规定语法。

### SPEC-STYLE-031 `ClassId` manifest 只能引用已生成的静态 StyleProgram

- **状态**：OPEN（manifest 未启用）
- **强度**：MUST NOT
- **要求**：若启用 `ClassId` manifest，其条目 MUST 只引用已生成的静态 `StyleProgram`；MUST NOT 引入未登记的 condition namespace，MUST NOT 在运行时解释 manifest 之外的候选。manifest 与源码 MUST 同源、同版本 pin（`SPEC-ROUTE-001`），构建 MUST 可离线复现。
- **失败模式**：manifest 与源码版本漂移 → 线上样式与开发机不一致；manifest 引入新的伪 condition。
- **来源**：FINAL §6；`SPEC-SCOPE-012`。

### SPEC-STYLE-032 禁止 `ClassString` 一类运时候选热路径

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：MUST NOT 提供“运行时全量解析 class 字符串”的 API 或内部热路径；MUST NOT 用静默 no-op 兜底未知候选。需要运行时变化时 MUST 使用 typed `style:*` 值（`SPEC-MACRO-015`）或登记的 ClassId。
- **失败模式**：为方便增加的 `ClassString` 入口成为事实上的 CSS VM，所有编译期保证被绕过。
- **来源**：FINAL §6；`SPEC-ROUTE-028`；`style-system.md` §7。

### SPEC-STYLE-033 支持清单必须由固定上游版本生成

- **状态**：OPEN（manifest 与差异测试未建立）
- **强度**：MUST NOT
- **要求**：支持清单 MUST 从**固定上游版本**（`SPEC-ROUTE-001`）的官方输出生成 manifest，并以差异测试校验；MUST NOT 在运行时下载、读取 npm/CDN 或“最新版”定义，MUST NOT 从网络拉取 utility 定义。第三方 Rust 实现只能作离线 oracle，MUST NOT 替代官方 fixtures（`tailwind-coverage.md` §6）。
- **失败模式**：构建结果随网络/依赖更新漂移；同一源码在不同机器解析出不同 declaration。
- **来源**：FINAL §6；`tailwind-coverage.md` §6；`style-system.md` §5/§6。

### SPEC-STYLE-034 未知输入必须诊断，span 契约归宏

- **状态**：SPEC
- **强度**：MUST / SHOULD
- **要求**：未知 class、未知 variant、未知 arbitrary property、不可解析单位 MUST 编译错误并给出可定位 span 与修复建议；MUST NOT 降级为 warning 或静默忽略。字面量内部子 span 的降级规则 MUST 与 `SPEC-MACRO-018/019` 一致（当前降级为整段字面量 + token/byte offset，不伪造更细高亮）。
- **失败模式**：`w-24pixels` 被忽略或按前缀猜测；错误高亮整段 `class`，用户需自行数字符。
- **来源**：FINAL §6；`style-system.md` §8；`tailwind-coverage.md` §6。

## 5. 支持等级词表与覆盖度

### SPEC-STYLE-040 等级词表固定为 N / D / R / X

- **状态**：SPEC
- **强度**：MUST
- **要求**：每个 utility family 与 variant MUST 只使用以下四个等级，定义如下（`SPEC-SCOPE-020` 引用的唯一出处即本条）：

| 等级 | 含义 | 允许的宣称 |
|---|---|---|
| `N` | 原生明确支持，按 documented native 契约 | 可在文档写“支持”，附条目级证据 |
| `D` | 有约束力差异，必须公开（默认值、测量、舍入） | 写“支持但有差异”，差异必须写清 |
| `R` | 需要 renderer / 自有状态 / 平台服务才能支持 | 只写“依赖能力”，MUST NOT 写“已支持” |
| `X` | 不作为内置语义，必须编译诊断 | 写“明确拒绝”，MUST NOT 静默 accept |

- 实现等级 MUST 是**逐条目**判定；MUST NOT 用 family 整体的“大部分”替代条目判定。
- **失败模式**：把 `R` 写成 `N` → 用户在不支持的 backend 上得到空图或 panic；把 `X` 写成 `D` → 编译通过但不生效。
- **来源**：FINAL §6；`tailwind-coverage.md` §1。

### SPEC-STYLE-041 覆盖度只用分子/分母，且必须指向 EVID-MATRIX

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：需要表达覆盖度时 MUST 写成 `通过 fixture 数 / manifest 中条目总数`，并指向 `EVID-MATRIX`（`docs/evidence/matrix.json`，规划）与生成该 manifest 的固定 pin。MUST NOT 发布百分比，MUST NOT 使用“基本支持 / 绝大多数 / 接近完整”一类措辞（治理契约 §2）。
- **未跑过的官方 fixture MUST 记为 `OPEN`**；只跑过最小自建探针的条目只能记 `PROBE`。`PROBE` 不得被折算成分子。
- **失败模式**：文档出现“支持 80% Tailwind”而无矩阵来源 → 按虚假宣称删除（`SPEC-SCOPE-020/021`）。
- **来源**：FINAL §6 末段；`tailwind-coverage.md` §1/§6；`SPEC-SCOPE-020`。

### SPEC-STYLE-042 本文不记录覆盖数值

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：本文件 MUST NOT 给出任何百分比、分子/分母数值或“已支持 family 数”。矩阵与逐项状态归 evidence 层；本文只定义等级语义、判定规则与失败模式。
- **失败模式**：spec 与 evidence 各自维护一套数字 → 两处不一致，且 spec 无法复跑。
- **来源**：治理契约 §1 铁律；`tailwind-coverage.md` §6 末段。

## 6. 单位、颜色与 token 的确定性

### SPEC-STYLE-050 长度单位：logical px 是布局唯一单位

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：
  - 布局 MUST 使用 logical CSS pixel；`px` 是 logical CSS pixel，**不是** physical pixel；
  - `rem` MUST 基于显式配置的 root font size；
  - `vh` / `vw` MUST 解释为窗口逻辑 viewport；
  - DPR MUST 只进入 physical targets 与 text epoch，MUST NOT 混入布局值。
- **失败模式**：高 DPI 下尺寸翻倍；窗口跨屏拖动后布局与字形错位。
- **来源**：FINAL §6；`style-system.md` §4；`tailwind-coverage.md` §2。

### SPEC-STYLE-051 长度表达式必须 typed，percent 无 basis 时诊断

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：`calc`/`min`/`max`/`clamp` MUST 由自有 typed `LengthExpr` 求值；percent MUST 只在 adapter 明确知道 basis 时求值，否则 MUST 诊断。MUST NOT 把 Taffy 内部 opaque calc handle 当作通用 CSS 求值器；MUST NOT 在运行时把单位字符串转成数值。
- **失败模式**：`w-1/2` 在未知父宽下静默取 0；`calc()` 被近似后与上游结果分叉且不可复现。
- **来源**：FINAL §6；`style-system.md` §4。

### SPEC-STYLE-052 颜色必须是规范化值并携带色空间

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：颜色 MUST 在编译期解析为规范化的 typed 值（oklch / oklab / srgb 等）并显式携带色空间与 alpha；MUST NOT 在运行时解析颜色字符串，MUST NOT 丢失色空间只留 RGB 三元组。
- **失败模式**：同一 `oklch` 值在不同 backend 上偏移；透明度被预乘两次。
- **来源**：FINAL §6；`style-system.md` §4；`SPEC-RENDER-010`。

### SPEC-STYLE-053 token / theme 变化走 epoch，不每帧查表

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：token 引用 MUST 携带 theme/epoch；变化 MUST 只 patch 依赖字段（`SPEC-STYLE-006`）。MUST NOT 每帧按名字查 token 表，MUST NOT 把主题切换实现为全量重新解析。
- **失败模式**：主题切换掉帧；或部分节点未刷新（脏边缺失）。
- **来源**：FINAL §6；`style-system.md` §4/§7。

### SPEC-STYLE-054 规范化必须确定性，禁止累积漂移

- **状态**：SPEC
- **强度**：MUST / SHOULD
- **要求**：长度、颜色、transform MUST 采用单一方向的规范化转换（token → 值），MUST NOT 对已解析值反复做 `± 常数` 累积运算。等价的声明 MUST 得到逐位相同的结果；golden 验收 SHOULD 比较精确 bits（如 `f32::to_bits()`/规范化整数）而非近似阈值。
- **失败模式**：`p-4` 在一个 patch 周期内被累加多次，元素缓慢膨胀；同一 class 在不同节点产生略微不同的尺寸，像素级差分测试随机失败。
- **来源**：FINAL §6（“单位 logical CSS pixels”“确定性”）；`tailwind-coverage.md` §6（差分测试）。

### SPEC-STYLE-055 颜色空间与 surface 能力归渲染层，样式不越权

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：样式 MUST 只保证规范化值与声明的色空间；P3/HDR、format、alpha mode 的物理映射 MUST 由 `RenderCapabilities` 与 surface 能力决定（`SPEC-RENDER-009/010/011`）。MUST NOT 在样式层宣称“广色域已支持”或“像素正确”。
- **失败模式**：样式文档宣称 P3 正确，实际在 sRGB surface 上过饱和。
- **来源**：FINAL §6 末段 / §7.2。

## 7. 已撤销的样式表述（WITHDRAWN）

| ID | 曾提出的表述 | 状态 | 撤销依据 |
|---|---|---|---|
| WITHDRAWN-STYLE-01 | 激活后的 condition 可跨 origin 胜过 ordinary inline | `WITHDRAWN` | FINAL §6 / §8；`SPEC-ROUTE-023`；见 `SPEC-STYLE-011` |
| WITHDRAWN-STYLE-02 | `class:if-*` / `class:if-selected` / `class:selected` 条件语法 | `WITHDRAWN` | FINAL §6 / §8；`SPEC-ROUTE-022`；见 `SPEC-STYLE-022` |
| WITHDRAWN-STYLE-03 | 运行时 CSS selector 解释器 / 任意 class 字符串 | `WITHDRAWN` | FINAL §6；`SPEC-ROUTE-028`；沿用 `docs/archive/research/CONCLUSION.md` 旧 Tailwind 段落，只作历史 |
| WITHDRAWN-STYLE-04 | 按 attrs 书写顺序或 appearance 字段顺序决定胜出 | `WITHDRAWN` | FINAL §6：canonical utility+variant order；见 `SPEC-STYLE-014` |
| WITHDRAWN-STYLE-05 | 复制浏览器/Tailwind preflight 对 `button`/`input`/`h1` 的 DOM 默认样式 | `WITHDRAWN` | FINAL §6：原生控件按 widget 语义单独规范；`SPEC-SCOPE-010` |
| WITHDRAWN-STYLE-06 | 用 `selected` 状态冒充 `active:`（native Pressed） | `WITHDRAWN` | FINAL §6 / §8；见 `SPEC-STYLE-022` |

引用纪律：任何新文档/新 API MUST NOT 把上表左列写成现行方案；提及时写“已撤销，见 `WITHDRAWN-STYLE-0x`”。

## 8. OPEN：样式侧未决项

| ID | 未决项 | 影响 | 解除条件 |
|---|---|---|---|
| OPEN-STYLE-01 | support manifest 与官方 fixture 集合均未生成 | `SPEC-STYLE-040`–`-042` 只能登记语义，无法给出任何覆盖数据 | 由固定 pin 生成 manifest + 差异 harness，输出 `EVID-MATRIX` |
| OPEN-STYLE-02 | `!important` / layer / transition 的完整排序未冻结 | `SPEC-STYLE-013` | 固定版本差分 fixture 覆盖 padding 轴、border 简写、颜色 alpha、slash leading |
| OPEN-STYLE-03 | container query 的 accepted-size 反查未实现 | `SPEC-STYLE-025` | 布局阶段提供反查 API + 防循环 fixture |
| OPEN-STYLE-04 | `@theme` / `@utility` / `@apply` / `@custom-variant` 的白名单与 canonical order 等价性未定 | 自定义扩展能力 | 逐项 ADR + 与官方输出等价的差分测试 |
| OPEN-STYLE-05 | transition / animation 调度（property 组、旧值捕获、reduced motion）未设计 | 动效类 utility | 与渲染层联调，明确 patch 触发与 offscreen invalidation |
| OPEN-STYLE-06 | 广色域 / HDR 的 token → physical pixel 映射未验 | 颜色类宣称 | 与 `SPEC-RENDER-011` 联动，给出固定 token 对照 |
| OPEN-STYLE-07 | 编译错误是否引入稳定 `ZENU-*` 编号 | 诊断可搜索性 | 与 `OPEN-MACRO-05` 一起冻结编号空间 |
| OPEN-STYLE-08 | 动态 class 的 enum API 形状（变体如何映射到静态规则集） | `SPEC-STYLE-030` 的可用性 | 先写 2 个典型用例（filter 三态、错误态）的 fixture |

## 9. 与其他规范的关系

- 版本 pin 与上游 oracle：只引用 [SPEC-ROUTE-001](01-route.md)，本文不重复。
- 能力边界与宣称纪律：`SPEC-SCOPE-010/011/012/020/021`。
- 宏侧 `class`/`style:*`/`aria:*` 的语法与诊断：[spec/03-macro.md](03-macro.md)（`SPEC-MACRO-013`–`-017`、`-018/019`）。
- 状态来源、epoch 与事务：[spec/02-runtime.md](02-runtime.md)（`SPEC-RT-006`、`-030`、`-058`）。
- layout / layout-text 的目标字段：[spec/05-text.md](05-text.md)（`SPEC-TEXT-001`–`-005`）。
- paint 字段、颜色空间与能力门：[spec/06-render.md](06-render.md)（`SPEC-RENDER-009`–`-011`）。
- Gate 归属：B Gate 负责 StyleIR 基线（inline vs class、hover 两种状态、N/D/R/X manifest）；见 [spec/08-gates.md](08-gates.md)（`SPEC-GATE-B`）。
