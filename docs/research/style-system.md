# StyleIR：原生 Tailwind v4 风格语言，而不是浏览器 CSS VM

## 1. 运行时目标

```text
literal class (宏内/外部开发工具)
    ↓ 仅一次以固定主题/utility 版本解析
UtilityAst { variants, base_utility, value, modifier, important, span }
    ↓
CanonicalRule { property_mask, declaration, variant, specificity, ordering, source }
    ↓ 编译期消去无用/未知/冲突规则（遵守既定优先级）
StyleProgram（immutable static metadata + node-local dynamic bindings）
    ↓
状态/环境改变 → 命中延迟注册的小 bitset → patch 受影响 property mask
```

不在每一帧执行 `class.split(' ')`、不把字符串转换给 Taffy、不在节点上放任意 CSS AST 大对象。一个 `StyleProgram` 携带多个字段级声明，不是把一组 class 融合成不可重复的 “当前 style 大结构”。

与 Tailwind 官方 plain text candidate scanner/enumerator 不同，宏实际是 Rust 语法层边界，具备更准确的范围和更严的诊断能力。[官方检测与动态字符串约束](https://tailwindcss.com/docs/detecting-classes-in-source-files) 不承认字符串插值在编译器中的可解析性，所以我们明确限制语法而不是暗示任意动态 class 都能工作。

## 2. 核心 IR 形状

```rust
struct StyleRule {
    canonical: CanonicalRuleId,
    property_mask: PropertyMask, // 仅为实际上更新的字段；不是全局以来源覆盖
    declaration: Declaration,
    condition: StyleCondition,
    origin: StyleOrigin,         // theme/intrinsic/class/inline/framework
    important: bool,
    precedence: Precedence,      // (layer, variant_rank, utility_order, declaration_index, insertion_rank)
}

enum StyleCondition {
    Always,
    Node { hover: bool, focus_visible: bool, pressed: bool, checked: bool, /* set */ },
    Ancestor { group_id: GroupId, state: AncestorCondition },
    Theme(ThemeId),
    Root { width: Range<LogicalUnit>, height: Range<LogicalUnit>, interaction: MediaCapability },
    Container { id: ContainerId, query: ContainerQuery },
    Direction(TextDirection),
}

struct StyleResolution {
    fields: FieldMap<Field>,
    provenance: FieldMap<Precedence>, // debug: class/rule and why it won
    size_dependencies: ContainerDependencies,
    paint_dependencies: PaintDependencies,
}
```

实际布局 projection、Parley style、绘制记录是 lazy derived snapshots，非每帧解析 CSS。每个节点为当前 Style IR 存 revision 和实际使用条件，从 theme/environment revision 只反查依赖节点。

## 3. 优先级不能是“最后字符串覆盖一切”

层次从概念上如下（正式实现前用固定 v4 版本 CSS output 的 fixtures 决定确切细分）：

1. intrinsic/written defaults（框架定义，而不是浏览器的 preflight 混搭）。
2. `@theme`/fallback token values。theme 改变不等于任意 property always beats。
3. registered class utility declaration，按其 `layer → media/variant capability → important → utility property/selection order` 比较。
4. literal class 同一完全同类实例按源代码最后值可覆盖；仅此地应用字符串 later-wins。
5. direct inline prop。dynamic class binding 在宏生成的参与条件相同点保持显式 ordering，不自动一定 beats literal/inline。
6. framework/transition 因现有状态选择的低/高层不可跨级随意覆盖。`hover:bg-indigo` 比 plain bg 在 hover 命中时优先；与 breakpoint/state/dark 的 precedence 必须固定，不能“当前 condition matched 就最后一个 patch”。

我们无需 CSS selector-specificity 全套，但有预设 origin/cascade 结构和 utility index。**transition、动画与 variant 匹配之后的 resolved style 按同一规则比较；同名 transform/padding 简写展开到一致 property mask，而非按 class 名字 guessed override。**

对能力不足（例如 CSS variables 未提供对应的生命周期 resolver）会在构建/编译阶段失败，而不是悄悄变成未达标兜底样式。

## 4. 单位、color、tokens

- UI 布局单位：logical point，按 Tailwind 默认 spacing `0.25rem` 物理规格先转换为 point（通常 16 px rem basemap）；`px` 映射是 logical CSS pixel，不是 physical DPR pixel。桌面 `rem` 根值必须 explicit `RootTheme::font_size`，不是随平台 root scale 偷改。
- `pt-[calc(env(...))]` safe area/keyboard inset 先不承诺；支持的 `calc/min/max/clamp` 使用自有 typed `LengthExpr`，对 percent basis 求值只在 adapter 明确知道时进行。**不直接把 Taffy `calc` 的内部 opaque handle 当通用 CSS evaluator。**
- color 表示：局部使用 gamma-correct abstract profile；Vello `Rgba8Unorm` 不等于 sRGB color space 已解决的证明。默认支持 oklch/oklab/srgb 的标准化值、插值/opacity；sRGB physical texture histogram vs encoded output 的明确定义由 render crate 负责。
- P3/HDR 作为后端 surface capability 的后续功能，不把 `oklch` 只解析成 RGB 后等同于完整广色域支持。
- `em` 用于 typography/spacing inherited font size 时保留引用树、不得过早变 px；`vh/vw` 解释为 root window logical viewport，不是 monitor。
- 主题默认值采用固定 v4 palette/tokens 但内部转换为 native value registry。**不因为用 import tailwindcss 就自动接受 DOM preflight 对 button/input/h1 之类的默认改变。**

## 5. Tailwind CSS-first 文件与开发工具

目标是允许用户保留熟悉习惯：

```css
/* app.css; 不发送到浏览器，构建工具读取 */
@import "zen/theme";
@theme {
  --color-brand-500: oklch(0.62 0.18 260);
  --spacing: 0.25rem;
  --breakpoint-sidebar: 48rem;
}
@utility selected-ring {
  ring-width: 2px;
  ring-color: var(--color-brand-500);
}
```

### 优先级

| 层级 | 内容 |
|---|---|
| A 当前 fixture 范围 | 少量 static literal class、字段类型、字节 offset、hover/focus-visible/active 的条件映射。**不等于常见 variants、theme registry、优先级全部完成** |
| B 原生模板扩展（目标） | `@theme`, `@utility`, 自定义 pure-declaration props、@custom-variant 可投影至 environment/Ancestor states |
| C 工程化工具（目标） | `zenui-style build` 扫描 Rust 宏 tokens，生成 style manifest/IR 与 source maps；CLI watch/increments 编译，io fallible results 名称与压测 |
| D 明确的 edge | JS plugins、完全动态 template string、浏览器 arbitrary selector/@property 原样执行、full CSS cascade 和页面 DOM condition 不默认承诺 |

主宏不应为了 class 的字面量发现而再全盘扫描项目，也不应从 npm registry/CDN 下载“最新”Tailwind——构建必须可重现、可离线、固定编译辅助依赖。proc macro 读取 app.css 文件作为后阶段能力时，必须通过 named compiler/build integration（cargo、rerun-if-changed）保持 rebuild 一致；同一表达式在 editor 和 compiler resolver 版本不一致会显示版本错误。

## 6. Rust CSS 编译器候选的证据

`tailwind` crate 0.4.0 的纯 core（`default-features = false`）在 Windows Rust 1.99.1 本机成功编译，明确提供 `compile`、`Compiler::build`、AST、`DesignSystem` 等可做差分测试/语义基准的接口。本机的离线 probe 用 embedded theme/preflight/utilities assets 验证：

- `p-4` → `calc(var(--spacing) * 4)`；
- `bg-blue-500` → `var(--color-blue-500)`；
- arbitrary width/color 与 hover/ARIA 规则可生成；
- `@theme --color-brand-500` 自定义候选会输出相关 declaration；
- unknown candidate 只被忽略，**所以未知 class 错误需要我们的 AST/工具自行实现**。

使用这类第三方的依据：
[crate docs](https://docs.rs/tailwind/0.4.0/tailwind/)、[repository](https://github.com/sntran/tailwind.com)。其 CLI feature 的 QuickJS 在本机 MSVC 构建时遇到 `sys/time.h` 找不到，反证“rust crate = 所有平台零成本可用”不成立。不能把这未验证 port 当官方 Tailwind 100% 一致；选定 pin 后，对与上游官方实现生成固定 CSS 的声明比较、API 许可证与长期维护成本重新评估。

结论：**先做独立字面量→NativeDeclaration 与跟官方 fixtures 的差分语义，再逐渐引入 CSS 编译层；不是现在 clone 整个 CSS runtime。**

## 7. literal 与 dynamic class API

```rust
view! {
    <view
        class="flex items-center gap-2 rounded-md bg-zinc-900 active:bg-indigo-600"
        class:if-selected={move || selected.get()} // future explicit selected ClassId registry
        class:error={error_style}                 // ClassId, not an arbitrary runtime String
        style:width={width}                       // Width typed, not unit string guessing
    />
}
```

如果 `errors.bg_class()` 返回任意字符串，则编译系统不能证明 utility 已注册；应有静态变体映射、predeclared `ClassId` manifest 或 `style:background={Expression<Color>}` typed escape hatch。**不要给 `ClassString` 一个 runtime 全量解析的热路径来假装问题不存在。**

主题的动态切换从 token reference 触发 field patch；`class:if-*` 只替换 affected mask 的 condition resolution；不相关的 `title` 变化不应重新 style resolve/rebuild 整棵树。

## 8. parser/单元错误诊断例子

```text
error[ZENU200]: unsupported Tailwind candidate `peer-checked:bg-red-500`
  --> src/widgets/row.rs:18:30
   |
18 | class="peer-checked:bg-red-500"
   |                              ^
   = native tree has no CSS general sibling selector;
     use `class:checked={row_state}` with a parent relation or typed signal,
     don't add a hidden DOM;
help: select explicit native state in the owning controller.
```

```text
error[ZENU021]: unparsable unit `w-24pixels`
help: use `w-24`, `w-[96px]`, or typed `style:width={points(96.)}`
```

这些 span 保留来自 `syn::LitStr`，不把整棵宏代码糊成一个错误。style module 必须被 IDE-style inspector、trybuild、可手写 builder 等共同复用。
