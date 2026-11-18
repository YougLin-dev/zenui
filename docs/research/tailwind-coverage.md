# Tailwind CSS v4：原生支持词典与覆盖验收

## 1. 如何定义“绝大部分”

**不按 class 名字自我申报。** 分四个维度记录：

- U: utility/property/value syntax。`class` 的字面量与 fixed variant 语法是否解析正确，长度、颜色、负值、任意值、modifier 能否规范化。
- V: 变体及选择关系。原生有没有对应 node state/ancestor/environment，不可只靠 CSS parser 生成的选择器成立。
- P: 属性级 target semantics。Taffy 模式尽量一致；native 默认二进制单位/scroll/transform backend 利钩明确定义差。
- T: toolchain。fixed theme、custom utility、source spans、unknown class 在 compile/IDE 中的可诊断性。

对于用户常见、原生有明确意义的 utilities，达到 **广泛、可计量支持** 是目标；对于 CSS 浏览器独占的能力，故意拒绝而不是打折扣悄悄忽略。

状态定义：

- `N`：原生明确支持，按 documented native 契约；
- `D`：有约束力差异，必须公开（例如默认值、文字测量）；不将差异伪装成“完全相同”。
- `R`：需要 renderer/own state or platform service，可以在第二阶段实现，但不声称免费获得。
- `X`：不作为框架的内置 native 语义；必须编译诊断而不是静默 accept。

本文是**设计规范**，没有逐项实测的项目均标记为目标层级，不是当前代码已经全实现。矩阵要以 `tailwind-version.pin` 从官方 package 生成人群对应的 candidate manifests，并对 fixtures 逐项断言。官方基础参考：[function/directives](https://tailwindcss.com/docs/functions-and-directives)、[styling with utilities](https://tailwindcss.com/docs/styling-with-utility-classes)、[detecting classes](https://tailwindcss.com/docs/detecting-classes-in-source-files)、[theme](https://tailwindcss.com/docs/theme)。

## 2. 基础/default/native 差异

| 类别 | 桌面方式 | 判定 |
|---|---|---|
| 长度/百分比尺寸 | 逻辑 CSS px；`rem` 基于根字体；`vh/vw` 窗口逻辑大小；DPR 在物理 target 层处理 | N |
| Flex/Grid | Taffy Block/Flex/Grid mapped types；不宣称版本所有 CSS quirks 已吻合 | N/D，须有差分 fixtures |
| DOM-based default | `<view>` 默认 **block-like normal flow** 与 Tailwind 全义基线选择一次冻结；若 `flex` 则置 display Flex，不做隐式横向 | N，UI 基础规范 |
| `<row>/<column>` | intrinsic 语义明确置 Flex 方向；体用 physical top-left 坐标，RTL按 Taffy direction | N |
| box-sizing | 原生边框盒即 border-box，接受 utility为兼容声明，不要求两套 CSSbox semantic战争 | D，公开 |
| preflight | 只透明选取与自绘 nodes 切合的 reset；不把 HTML h1/button/a/form defaults 盲目复制 native widgets | D，widget styling 单独规范 |
| visibility | `hidden`/`invisible`：剔除布局或只剔除绘画/交互；ARIA 是否同时拥有语义依节点 style invariant | N，非 CSS display attribute 浅交代 |
| text inheritance | 文本样式从保留树的最近只声明内容 inherit parent 动态版本派生，与 border/padding 不无关继承 | N |
| overflow scrolling | 自有 ScrollState/hit/clipping/backbar/reveal；不只是 Taffy overflow enums | R |

## 3. Utility family 总表

### 3.1 Layout / Box

| 类族 | 示例 | 状态与理由 |
|---|---|---|
| position & inset | `absolute relative fixed sticky top-4 inset-x-2` | N/R：scrollbar popup 和 sticky 属于框架自己的坐标阶段，非 Taffy 自己吸附 |
| flex direction/wrap/grow/shrink/basis/order | `flex-row`, `flex-1`, `grow-0`, `basis-1/2`, `order-1` | N；映射要和 sibling/ancestor dirty，但非 component re-create |
| display & box decoration | `block flex grid contents table inline-block inline` | `flex/grid/block/hidden/contents` N；inline/align式类型有原生 abstraction后另行支持；表格系列 **X（初版）**，不代表 `<table>` 组件不可用 |
| alignment/justify/items/self/content | `items-center`, `justify-between`, `self-end`, `content-center` | N；`safe/unsafe` CSS 边缘对齐和 baseline定项 `D/R` |
| size min/max | `w-full`, `h-1/2`, `max-w-md`, `min-h-screen`, `aspect-video` | N；arbitrary math 用自有 Expr求值，无完整 CSS dynamic `calc` API promise |
| box ratio/cols/rows/intrinsic | `aspect-square`, `size-14`, `min-w-fit` | N/R；replaced item icon/image/measure 模式由 widget state声明不由 CSS0值猜 |
| flex/grid gap | `gap-4`, `space-x-2`, `divide-y` | N；divide lines 是 render descriptor 而不是只专依 CSS child selector |
| grid tracks | `grid-cols-3`, `grid-cols-[1fr_2fr]`, `auto-cols-max`, `grid-flow-dense` | N/D: Taffy minmax/core repeat基础验收，subgrid不在初版 |
| grid item span/sizing | `col-span-2`, `row-start-1`, `grid-cols-subgrid` | 普通 span N；subgrid X (not Taffy capability) |
| bidirectional & LTR | `rtl`, `ltr`, `text-start`, `start-4` | N：style direction + Parley bidi + Taffy layout共享，不假设每 widget 会独立 bidi context |
| z/order | `z-10`, `isolation-isolate`, `order-first` | N：生成 paint/hit compositing stack，不只是一串布局 info |

### 3.2 Typography

| 类族 | 示例 | 状态 |
|---|---|---|
| family/weights | `font-sans`, `font-mono`, `font-medium`, `font-thin` | N：Fontiquefamily/local fallback；命名 family found 字体实际可变 coverage 依据资源检查 |
| size/height | `text-sm`, `leading-6`, `leading-tight`, `text-xs/5` | N：typed font_size/line_height；group selectors 无关 |
| style/synthesis | `italic`, `not-italic`, `antialiased`, `subpixel-antialiased` | N/D: oblique/font variants supported genuine face/fallback；hinting anti 名不悄悄保证灰阶 or RGB 文本 |
| spacing | `tracking-wide`, `tracking-tighter` | N：Parley letterspacing supported；text shaping key重变和失效必须同步 |
| color/opacity | `text-zinc-900`, `text-white/60`, `text-[oklch(...)]` | N：文字 ColorInterpolation pipeline and color spaces |
| alignment | `text-left`, `text-start`, `text-center`, `text-justify` | N/D：Parley支持array +lastline；不做完整 CSS hyphenation |
| decoration | `underline`, `overline`, `line-through`, `decoration-wavy`, `underline-offset-2` | N：Parley or native decoration；下划线的 width/over offset 需真实 glyph metrics，非over paint |
| transform | `uppercase lowercase capitalize normal-case` | `uppercase/lowercase`由 locale-aware Unicode case mapping支持；`capitalize` 需限制CSS微小语言规则，D/R |
| white space/break | `whitespace-pre`, `text-wrap`, `break-words`, `break-all`, `break-keep` | N/D：Parley wrap/break模式，unknown/hyphenation 明确拒绝 |
| ellipsis / line clamp | `truncate`, `line-clamp-3`, `text-ellipsis` | R：single-line可简单 inline truncation；多line paragraph 不可加省略号后还重排整个 para，排版与a11y要求全文档不变短语不明确欺骗 |
| indent | `indent-4` | N：inline/text paragraph style key |
| writing mode | `vertical-rl`, `sideways-lr` | X：Taffy horizontal-tb约束不能假装 Parley bidi已解决 |
| font variant features / numeric | `tabular-nums`, `slashed-zero`, `ordinal`, `diagonal-fractions`, `font-stretch-*` | N：OpenTypefeature/variations normalization + glyphkey |
| placeholder/selection | `placeholder:text-zinc-500`, `selection:bg-blue-200` | R/N：native TextEditor/selection controls states，非浏览器 shadow pseudo-element |

### 3.3 Paint / Appearance

| 类族 | 示例 | 状态 |
|---|---|---|
| colors/alpha | `bg-indigo-500`, `border-zinc-800/70`, `text-pink-300` | N：palette固定和 arbitrary value evaluator |
| borders/rounding | `border`, `border-2`, `border-l-4`, `rounded-lg`, `rounded-tl-full`, `outline` | N：boxstyle fields， `outline-style: solid` 没有外部 native 控件全量 preflight |
| gradient stops | `bg-linear-to-r`, `from-indigo-500`, `via-pink-400`, `to-cyan-400` | N：peniko/common gradient；angle/color interpolation 的 exact variants require 源参照 |
| box ring/shadow | `shadow-md`, `ring-2`, `ring-offset-2`, `inset-shadow-...` | N/D：paint 独立命令、alpha+blur backend支持检测，不能只抄 CSS shadow string |
| opacity | `opacity-50`, `hover:opacity-100` | N：composite layer；涉及 geometry can变更 GPU resources，无需 layout |
| masks/backdrop/filters | `blur-sm`, `backdrop-blur-md`, `drop-shadow-lg`, `grayscale`, `contrast-*` | R： Velo capabilities gating，filter 依赖 subtree offscreen surface |
| mix/isolation | `mix-blend-multiply`, `bg-blend-screen`, `isolate` | R：只支持明确的 compositing ops，destructive/非隔离 unsupported backend不报“已启用” |
| SVG / fills/strokes | `fill-current`, `stroke-2`, `stroke-current` | N：native path icon state and renderer type |
| CSS content pseudo | `before:content-['x']`, `after:...` | X：content 请写真正 text node/slot，不能悄悄干掉真正显示内容 |
| `cursor-*`, `pointer-events-*` | `cursor-pointer`, `cursor-grab`, `pointer-events-none` | N/R：native hit map andcs Cursor abstraction， `resize` 单独 cursor platform service |
| `appearance-*`, `will-change-*`, `forced-color-adjust-*` | `appearance-none`, `will-change-transform` | D/N：可以 witnessed纯 visualhints，不需要 DOM 控件 appearance；Ime/a11y 不应因此 disabled |
| float/clear/display-table | `float-*`, `clear-*`, table family | Float if supported by Taffy feature D；table 没常规 native flexwidget 支持故初版 X |

### 3.4 Arbitrary & special

- arbitrary properties `[color:red]`、`[mask-type:luminance]`：**默认 X**，不是 Tailwind compilation をー проблема但本框架只支持白名单声明集合；可以 `custom @utility` after typed registration。
- arbitrary value `[calc(100%-2rem)]`、`bg-[...]`、fraction/negative：`N` for supported property typed value grammar；CSS `_` 转空格、escaped chars、bracket nesting 正确保留 diagnostics。
- arbitrary variants `[&:nth-child(2)]`：X；除非封为 named positional variant、动态 typedsignal，不实现 arbitrary selector executor。
- `!` important、prefix、modifier `/50`、`text-red-600!`、`group-hover/name:`：N，priority/变体语义单独实现，不能最后拼接 keep stripping将其当 base utility。
- `bg-[url(...)]` / 外部资源：本地受控 asset um resolver、cache ownership + load generation；下载远程得交给应用显式授权，构建期自动请求外部网络不是 style macro正确做法。

### 3.5 Motion

- `duration-*`, `ease-*`, `transition-*`, `delay-*`：N/R，transition 实际 from old resolved value到新 value，按 property/group scheduler，不是 class0? style patch	animation fire every global state 改变。
- `@keyframes`, `animate-spin/pulse/ping/bounce`：N/R，loading indicator 和 opacity/transformation timeclock，property group interpolation 绑定 resources. `animation`与 GPU offscreen invalidation 需要互不影响节点bounds or hit rect。
- `motion-safe/reduce`：N，AccessibleRoot preferences从 native platform Query +应用程序 override，不要求 web Media Query。
- `transform`, `translate`, `scale`, `rotate`, `origin`, `skew`：N，composition-only pts；types 保留 transforms 不改 Taffy layout.constraints，hit path 用相同 transform。
- `scroll-snap`/`scroll-smooth`：R，scroll state anim/decel platform policy，Taffy 不会处理 smooth 程序时间点。

## 4. Variants & environment

| Variant | 状态与默认服务 |
|---|---|
| `hover/focus/focus-visible/focus-within/active/disabled/enabled` | N：native context/state bits，keyboard focus visible style 不只 follow pointer |
| `checked/selected/indeterminate/expanded/invalid/aria-*` | N：typed semantic states/action linking to AccessKit,checked 具体映射 native 控制 state，不是 css attribute text |
| `first/last/odd/even/only` | N：reconciled children positional metadata，`For` reorder 时正确 invalid |
| `dark/light` | N：Application Theme scope & tokens；不要求 CSS media browser |
| `sm/md/lg/xl` | N：window root logical width breakpoints；若在原生 container得写 `container @sm:*` 对应 nearest query target |
| `@container/@sm/@max-*` | N/R：container layout stage的 query style conditions 源自 accepted box，不可在 layout cleanup measure 期间读一脸中再突变节点 styles |
| `group-hover/name`, `group-focus-within`, named groups | N：explicit native AncestorId + condition depth，而不是模糊 CSS ancestor text search |
| `peer-* and peer-* has-*`/general CSS relational | **默认 X**；显式 parent-to-sibling reactive property adapter 后命名表示。不能让 CSS selector engine猜测 app intent |
| `has-checked/has-[...]` | R，仅支持 declared parent child state shape；任意 selector 与 :has 无穷表达式 X |
| `rtl/ltr` | N：LayoutEnvironment direction与 texta11y biDi 同步 |
| ` motion-safe`, `motion-reduce`, `contrast-more`, `forced-colors` | N/R：native preferences platform query,framework safe fallback |
| `print`, `screen`, `speech` | X：无 web printing/postscript DOM；printing 是独立 render exporter而不是UI 项目 responsive |
| `noscript`, `open`, `inert`, `popover` | D：popover native window/portal、dialog/inert自己的 focus scopes与IME；`noscript` X |
| `min-[600px]`, `max-[40rem]`, `pointer-fine/coarse`, `any-*` | N/R：capability environment revision with host query,not fake type |
| `portrait/landscape` | N：aspect window resolved size，不是 monitor orientation 将 monitor incorrectly默认window |
| arbitrary property `[content]` / cross fields | X or explicit utility declaration after native giving semantics |

## 5. 工具链与 CSS-first 扩展

| 功能 | 方向 |
|---|---|
| `tailwind.config.js` | X：不引入 JS配置引擎。CSS-first native configurable themes + strongtyped Rust extensions |
| `@theme` | N：固定变量 namespace and knownnative effect dependencies, size tokens not CSS var 查表 per frame |
| `@utility` | N：普通 allowed declarations 白名单映射，不支持返回任意CSS selector cascade 完整 CSS |
| `@custom-variant` | N/R：简单伪类/ARIA/媒体等于 native condition；任意嵌套 selector/browser media X |
| `@apply` | R：pair utility declarations 与 properordering analysis,unsupported property 出诊断 |
| `@source` | N：开发工具扫描应用/共享 crate 类字面量；binary assets/lockfile 忽略规则沿用源文档思想 |
| `@plugin`, `@config`, WASM dynamic module | **默认 X**；执行用户提供的本地 extension 是进程安全/重现性/许可证议题，未来明确 feature、不是下载并执行 |
| dynamic CSS var `var(--x)` | R：只在 native registeredTheme/runtime vars 纳入 epoch/direction;font registry 的时候有支持，unknown 不能 arbitrary drop |
| `@property` | X（首版）：typed native 替代，无需在 paint runtime 自造 full custom CSS property inheritance/grammar |
| `@keyframes` | R：bounded internal animation compiler 与 named keyframes；GPU continuous anim 安静时schedule 不动 |
| `@import` | N：本地文件 loader build system，支持 dependencywatch andplatform lock,dev not URL-download |

## 6. 黄金测试而非自行宣称

- 用 Tailwind 固定版本生成 `input` 后的 candidate -> declaration 序列。纯 Rust `tailwind0.4` 可用于对照，而**源代码 port 不能替代现有官方test fixtures/上游commit**。
- utility regular feature + variant order output type proof：border-width shorthand、padding axes、`grid-cols`、negative calc、color alpha、font size/slash leading、placeholder、dark+responsive.
- browser layout diff（Taffy核心关心与browser一致 rect）与渲染 comparison（Vello native matches CSS colors gradients）两项分别验收，不把模拟截屏当计算样式事实。
- Unknown class 产生明确错误，disabled/genuine state 必须行为正确，`class arbitrary dynamic string` 没硬承诺。
- `tailwind-version.pin` commit/source map/test manifest 由工具 output 确定支持百分比，手写文档只作解释。**无法计数的 `绝大多数` 用例不会从一句“目标”升级为完成。**
