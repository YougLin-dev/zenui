---
doc-id: SPEC-MACRO
layer: spec
status: normative
date: 2026-11-18
owner: API 人体工学师
sources:
  - archive/review/FINAL.md
  - archive/research/native-macro.md
  - archive/review/round2-debate.md
  - archive/review/round3-review.md
  - archive/review/VALIDATION.md
replaces: []
---

# 宏规范：`view!`、`#[component]`、props 与诊断

> 本文只规定**宏表面语义**。它不是“宏已实现”的声明：当前仓库的 macro 探针只覆盖 `view`/`text`/`button`、`class` 与 `on:click|change|input`（见 §8）。本文件中 `SPEC` = 目标要求，`PROBE` = 有探针级最小验证，`OPEN` = 未决。

## 1. 责任边界

| 事项 | 归属 | 说明 |
|---|---|---|
| `view!` 的节点/属性归类、展开为 typed builder 或 descriptor 调用、宏侧诊断 | **本规范** | 宏是薄编译桥，不是运行时 |
| MountPlan 的提交、Owner/reactive graph、事务与回滚 | [02-runtime.md](02-runtime.md) | 宏只产生“描述 + 绑定槽”，不提交、不求值 |
| class token → StyleIR、variant 白名单、优先级 | [04-style.md](04-style.md) | 宏只把字面量交给 style 编译器，并转述其诊断 |
| TextEdit 的 epoch/origin/reducer | [05-text.md](05-text.md) | 宏只为 `value={signal}` 生成专属 adapter 调用 |
| 版本 pin（rstml/syn/quote 等） | [01-route.md](01-route.md) | **全仓唯一出处**，本文不复述版本号 |
| 宏验收 fixture 与命令 | [08-gates.md](08-gates.md)（A0/A1） | A0 不含宏 |
| 探针实际跑出的结果 | [../evidence/validation.md](../evidence/validation.md) `EVID-VAL-*` | 本文只引用 ID，不复制结果 |

**非目标**（不因为它写过 `view!` 就成立）：不做 VDOM，不做 HTML/DOM 标签，不做事件字符串，不做运行时 class 字符串解析，不做自动双向绑定，不自动把 `{expr}` 改写成绑定。

## 2. 四类位置：先分类，再谈类型

宏对每个 `{}` 的处理**由语法位置决定，不由表达式内容猜测**（不看是否含 `.get()`、是否 `FnOnce`、是否 `Copy`）。

| 位置 | 写法 | 唯一语义 |
|---|---|---|
| intrinsic 数据位（文字、普通 attr、style 值、`Show` 的 `when`） | `{expr}` | mount 期求值一次的值 |
| 同上 | `{move || expr}` | `BindingDescriptor`，读取处自动登记依赖 |
| 同上 | `{name}`（`name` 已绑定为 `move || ...` 闭包） | 与内联 `{move || ...}` **同类** |
| `on:*` | `{move \|E\| ...}` / `fn` 指针 | 控件 payload 事件；**不追踪、mount 不执行** |
| PascalCase 组件属性 | `{expr}` | 普通 Rust 实参，宏不改写 |
| 元素子位 | 文本字面量、`{move \|T\| view!{...}}` | children factory，按需执行 |

### SPEC-MACRO-001 位置分类不得退化为表达式猜测

- 强度：**MUST**
- 规范：宏 MUST 先按上表确定位置类别，再按该位置的类型契约接受表达式；MUST NOT 因为表达式“看起来像闭包/像 getter”而改变类别。
- 失败模式：分类退回猜测后，`{count.get()}` 会被当绑定处理（永不更新或重复订阅），`{move || ...}` 传给 `Signal<T>` prop 会被当 getter 求值（类型错误指向宏内部而非用户那一行）。两个症状都必须能在 A1 的 golden 中区分。

### SPEC-MACRO-002 `{expr}` 是一次性值，handle 原样传递

- 强度：**MUST**
- 规范：数据位的 `{expr}` MUST 在 mount 期求值**恰好一次**，MUST NOT 建立订阅。表达式类型是 `Signal<T>` / `Memo<T>` / Store/field handle 时，MUST 传 handle 本身，MUST NOT 调用 `.get()` / `.with()`。
- 失败模式：在应当更新的位置写 `{count.get()}` 会编译通过但停在首帧（无诊断、无更新）。宏 MUST NOT 事后把已求值的值“升级”为绑定（依赖已经丢失）；类型不匹配时 SHOULD 给出 help（例如“若希望随信号更新，写 `move || count.get()`”），但 MUST NOT 把笼统的 “missing move” 作为固定诊断覆盖真实类型错误。
- 状态：**SPEC**（探针只验证了静态文本与 `move ||` 内容两种形态）。

### SPEC-MACRO-003 `{move || expr}` 是绑定边

- 强度：**MUST**
- 规范：
  - 数据位的 `move ||` MUST 降级为 `BindingDescriptor`，MUST 装入当前 Owner 的 render binding，MUST NOT 在 mount 期执行函数体。
  - MUST 要求显式 `move`：绑定位置上的非 `move` 闭包 MUST 编译错误，MUST NOT 被静默当作一次性值。理由是绑定在 mount 之后才被调用，闭包必须拥有捕获值；把它降级为一次性值会造成“看起来写了绑定、实际永不更新”。
  - 函数体读取 MUST 在 tracked 上下文内登记；同一绑定内重复读同一字段 SHOULD 只产生一条订阅。
- 失败模式：漏写 `move` 时 MUST 得到指向该闭包的宏诊断（示意：`` `text` binding closure must be `move || String`; bindings are installed after mount and must own their captures ``），而不是裸 E0373 或整段 `view!` 报错。若绑定在 mount 执行，则依赖登记发生在 tracker 之外，症状为值永不更新。

### SPEC-MACRO-004 命名闭包与内联闭包同义

- 强度：**MUST**
- 规范：`{name}` 在数据位 MUST 与内联 `{move || ...}` 同类；MUST NOT 被读作 bool 当前值。`{selected}` 等价于 `{move || current.get() == value}`，不需要写 `selected()`。
- 失败模式：若把 `{name}` 当当前值，`Signal<Filter>` 场景会在首帧固定、点击不生效；错误信息应指向 `name` 这个标识符，而不是整个元素。
- 状态：**OPEN** 机制（如何区分“闭包变量”与“一次性值”：按签名 trait 还是显式 wrapper），见 OPEN-MACRO-01。

### SPEC-MACRO-005 `on:*` 事件：不追踪、mount 不执行、payload typed

- 强度：**MUST** / **MUST NOT**
- 规范：
  - `on:*` MUST 只接受 Rust 表达式；字符串字面量 MUST 编译错误。
  - MUST 编译为控件限定的 payload handler（`FnMut(E)`，或框架授权的 result-aware adapter）；MUST NOT 暴露 `on("click", ...)` 这类字符串事件 API。
  - mount MUST NOT 执行 handler；handler 体内的 `.get()` MUST NOT 建立依赖。
  - 每次控件激活 MUST 恰好调用一次；同一 handler MUST NOT 因父级其它字段变化被重复挂接。
  - handler 调用返回 `Result` 的 setter 时，MUST 由显式错误路径处理（用户处理或 result-aware adapter 送 sink）；宏 MUST NOT 用 `let _ =` 吞错误，也 MUST NOT 把 `Result` 静默丢弃以迎合无返回签名。
- 失败模式：`on:click="alert('no scripts')"` → `` event handlers must be Rust expressions, never script strings ``（已有 fixture）。payload 写错（如给 `checkbox` 写 `|v: String|`）→ 类型错误指向闭包参数，并 SHOULD 给出“该控件 payload 是 bool”的 help。mount 执行 handler 或把 body 读取当依赖，分别表现为首帧副作用与点击引发的重复计算。

### SPEC-MACRO-006 小写 = intrinsic，PascalCase/路径 = Rust 组件

- 强度：**MUST**
- 规范：小写标签 MUST 解析为 intrinsic registry 中的原生元素；PascalCase 或路径 MUST 解析为 Rust 函数组件，按 Rust 名字解析（IDE rename 可用），MUST NOT 有任何字符串 key 查找。未知小写标签 MUST 编译错误。
- 失败模式：`<div>`/`<span>` → 编译错误（示意：`` unknown intrinsic `div`; native intrinsics are not HTML DOM elements; use `view` ``）。未知组件名 MUST 以普通 rustc 未解析名字的错误出现，且 span MUST 落在标签名上，而不是整个 `view!`。
- intrinsic 清单本身的状态：**OPEN**（首批清单随 A1 冻结，见 OPEN-MACRO-02）。

### SPEC-MACRO-007 组件 props 是普通 Rust 实参

- 强度：**MUST**
- 规范：
  - PascalCase 组件的所有 `{}` MUST 作为普通实参按声明类型检查；宏 MUST NOT 自动 `.get()`、自动包 `move ||`、或做隐式 `Into` 转换。
  - handle 参数 MUST 直接传 handle：`current={filter}` 传 `Signal<Filter>` 本身，不是它当前的值。
  - Props 里的 owned `String`/`&'static str`/Copy enum MUST NOT 被要求 `Copy`；MUST NOT 隐藏 `.clone()`。
- 失败模式：`current={filter.get()}` 传给 `Signal<Filter>` 参数 → rustc 类型错误，span 落在属性值；宏 MUST NOT “帮忙”把它包装成 handle（那会隐藏订阅点）。若宏把 `{filter}` 包成闭包 → 参数类型不匹配，且语义从“传 handle”漂移为“传 getter”。
- 状态：**SPEC**（探针无组件/props 降级）。

### SPEC-MACRO-008 组件 mount 目标必须可失败

- 强度：**MUST**
- 规范：组件 MUST 仅在实例 mount 时执行一次；mount 结果 MUST 是可失败的 `MountResult<MountPlan>`（`Result<MountPlan, MountError>`）。函数体中的 `?` MUST 保持普通 Rust 语义。mount 失败 MUST 使该次 mount 整体不提交：无节点、无订阅、无默认替代文本，错误进入统一 sink。
  `-> MountPlan` 的短形式是否由 `#[component]` 自动包成 `Ok(plan)` 属 API 取舍，见 OPEN-MACRO-03；若保留，两者 MUST 语义等价。
- 失败模式：部分提交 → 半棵树/泄漏订阅；用 panic 或默认标题掩盖失败；宏生成的错误转换吞掉 `MountError`。三者都 MUST 被 A1 的 compile-fail/golden 覆盖。
- 状态：**SPEC**。只证明了普通 Rust 函数 `?` 的类型形状，不证明宏降级（见 §8）。

### SPEC-MACRO-009 children 是显式 factory，按需执行

- 强度：**MUST**
- 规范：子内容 MUST 以显式 `ChildrenFactory`/typed slot 传入；MUST 在父级分支活跃时按需执行；`Show` 为 false 时 MUST NOT 执行子 factory。多根规则 MUST 由类型控制，MUST NOT 用隐式 “任意 closure 当 children” 的猜测。
- 失败模式：父构造时提前执行 children → 隐藏分支也产生副作用与订阅；多根子内容被静默丢弃 → 用户看不到编译错误却少渲染节点。
- 状态：**SPEC**（slot 语法与 arity 规则 OPEN，见 OPEN-MACRO-04）。

### SPEC-MACRO-010 `<Show>` 是 mount/unmount

- 强度：**MUST**
- 规范：`when` MUST 接受绑定（`move || bool` 或等价的 BoolBinding）；分支切换 MUST 实现为 owner 的 mount/unmount；MUST NOT 编译成 `display:none`/`visibility:hidden` 之类的呈现策略替代卸载。`fallback` MUST 惰性，仅在 false 分支活跃时执行。
- 失败模式：用隐藏代替卸载 → 子控件仍参与布局/命中/语义树，且其订阅与定时器泄漏；切换后旧 generation 的迟到事件仍可能命中（必须由 runtime 拒绝，但宏不得制造这种结构）。

### SPEC-MACRO-011 `<For>` 必须显式 key，必须消费 `Memo<RowSet>`

- 强度：**MUST** / **MUST NOT**
- 规范：
  - `each` MUST 是共享的 `Memo<RowSet>`（或 runtime 声明的同构投影）；MUST NOT 接受 `Vec`、任意迭代器或每次求值都新建的集合。
  - `key` MUST 显式提供；MUST NOT 有 index fallback。key 形如 `|row: &T| row.id`（借用），child factory 形如 `move |row: T| view! { ... }`（接收投影行）。
  - 重排 MUST 保留 ViewId/Owner/局部状态；key 消失 MUST 只销毁 UI owner，模型 row 不归 For 所有。重复 key MUST 在 patch 前被运行时拒绝。
- 失败模式：缺 `key` → 编译错误，span 落在 `<For` 标签，示意：`` `<For>` requires an explicit `key`; index fallback would silently lose focus and edit state on reorder ``。`each` 传 `Vec`/iterator → 类型错误 + help 指向 `memo(...)`。把 `each` 写成 `move || rows()` 闭包 → 类型错误（`each` 收 Memo handle，不收 getter）。

### SPEC-MACRO-012 `<Index>` 只用于按位置

- 强度：**MUST** / **SHOULD**
- 规范：`Index` MUST 按位置保留行、把 item 暴露为 signal；长度缩短时 MUST 只释放尾部；MUST NOT 作为 `For` 的替代用于会重排的数据。
- 失败模式：对重排数据用 `Index` 编译通过，但 caret/焦点/局部状态跟错行（行为错误）。宏 SHOULD 在文档与诊断中提示该差异，而不是试图用类型完全阻止（两者签名相近）。

### SPEC-MACRO-013 `class` 只接受编译期字面量

- 强度：**MUST**
- 规范：`class` MUST 是字符串字面量并在编译期解析为 StyleIR；未知 token MUST 编译错误，MUST NOT 静默忽略。动态 class 字符串 MUST NOT 被接受；运行时拼接/拆分 MUST NOT 存在。
- 失败模式：`class={some_string}` → 编译错误 + help 指向 typed `style:*` 或（未来）登记的 ClassId manifest。静默忽略未知类会产生“样式没生效”的无声失败，必须禁止。
- **与 `SPEC-STYLE-030` 的边界**：本条只约束 `class=` 的**字符串形态**——它 MUST 是字面量。样式层允许的 enum / 静态条件 / `ClassId` manifest（`SPEC-STYLE-030`）MUST NOT 通过把表达式塞进 `class=` 字符串来实现；这类形态的表面语法（是独立属性还是登记枚举变体）未定，见 `OPEN-MACRO-09`。
- 归类细节（词表、variant、优先级）归 [04-style.md](04-style.md)，本文不复述。

### SPEC-MACRO-014 状态样式必须走登记的条件，禁止自造条件属性

- 强度：**MUST NOT**
- 规范：应用状态驱动的样式 MUST 通过 SPEC-STYLE 登记的 variant（例如 `aria-pressed:` 与 `aria:pressed` 同源派生）或 typed style 值；MUST NOT 引入 `class:` 前缀的临时/自定义条件属性，MUST NOT 提供 `ClassString` 之类的运行时候选。
- 失败模式：任何 `class:<任意名>` 属性 → 未知属性错误（span 在属性名），help 指向登记 variant 或 `style:*`。以独立 bool 驱动视觉、以另一个 bool 驱动 aria，会造成语义与视觉不同步，A1 的 toggle fixture MUST 阻止这种写法。

### SPEC-MACRO-015 `style:*` 只接受 typed 值

- 强度：**MUST**
- 规范：`style:<property>` MUST 接受 typed 值/enum 或绑定（`move ||`）；property 名 MUST 来自登记清单；MUST NOT 解析单位字符串，MUST NOT 接受任意 CSS property 名。
- 失败模式：`style:width="12px"` → 编译错误，示意：`` `style:width` expects a typed length; use points(12.0) (unit strings are not parsed) ``。拼错 property 时错误 MUST 指名具体 property，并 SHOULD 给出最近匹配。

### SPEC-MACRO-016 `aria:*` / `role` 映射为语义字段

- 强度：**MUST** / **MUST NOT**
- 规范：`aria:*` MUST 映射到控件的 typed 语义字段（值可为字面量或绑定）；`role` MUST 只接受框架登记的角色；MUST NOT 把 aria 当样式键或通用字符串字典透传，MUST NOT 把 aria 字符串本身当作可访问性已实现的证明。
- 失败模式：未登记 role/attribute → 编译错误（span 在 key）。把 `aria:pressed` 写成 CSS 条件来源之外的独立 bool → 语义与视觉不一致（见 SPEC-MACRO-014）。

### SPEC-MACRO-017 未知标签/属性/事件/类必须拒绝，不得静默忽略

- 强度：**MUST**
- 规范：未知标签、未知属性、未知 class、未知事件名 MUST 全部编译错误；MUST NOT `_ =>` 吞掉，MUST NOT 降级为 warning。SHOULD 提供 did-you-mean（事件名、property、intrinsic）。
- 失败模式：`on:clik` 被忽略 → 点击无反应且无任何诊断，用户只能在运行时困惑。A1 的 compile-fail MUST 至少覆盖一个拼错事件名。

### SPEC-MACRO-018 诊断必须自身可用

- 强度：**MUST** / **SHOULD**
- 规范：每条宏错误 MUST 包含（1）能定位的最小 span；（2）被违反的规则的具体陈述（不是 “token error”）；（3）可执行的修复（help）。MUST NOT 用整段 `view!` 的 span 覆盖可精确定位的 token。宏内部多处独立错误 SHOULD 通过 `syn::Error::combine` 一次给出。
- 失败模式：`custom attribute panicked`、`expected token`、整段宏高亮——三类都必须视为缺陷。IDE 跳转 MUST 落在用户写错的那个标识符/属性/标签上。
- 错误码命名（是否引入 `ZENU-*` 稳定编号）为 **OPEN**，见 OPEN-MACRO-05；在冻结前，fixture MUST 锁定消息文本而不是臆造编号。

### SPEC-MACRO-019 class 内部子 span 的降级契约

- 强度：**MUST**（降级行为）/ **OPEN**（精确子 span）
- 规范：在 proc_macro span API 无法可靠构造字面量内部子 span 之前，MUST 降级为“整段 class 字面量的 span + 消息内给出 token 与 byte offset”，MUST NOT 伪造比真实源码更细的高亮。实现精确子 span 后 MUST 由 trybuild 固定实际效果。
- 失败模式：伪造子 span 会在 unicode/转义后错位，指向错误字符；用**整串**高亮而不给 token 名，用户要自己数字符。
- 状态：**OPEN**（探针当前为整串 + byte offset，见 §8）。

### SPEC-MACRO-020 生成代码的 crate 路径与依赖重命名

- 强度：**MUST**
- 规范：生成代码 MUST 通过公开 facade 的 `__private`/entry facade（配合 manifest/package 解析）引用内部类型；MUST NOT 硬编码绝对 crate 名。调用方把 facade 依赖改名后，只 import facade MUST 仍能编译；调用方 MUST NOT 需要导入生成代码的私有类型。
- 失败模式：硬编码路径在 Cargo 重命名后产生 E0433，正确 RSX 无法编译，且错误指向宏展开而非用户代码。独立 package 的 trybuild MUST 覆盖“只 rename、无原名依赖”。
- 状态：**OPEN**（探针硬编码绝对路径；实现与 fixture 待 A1）。

### SPEC-MACRO-021 展开卫生

- 强度：**MUST NOT**
- 规范：展开 MUST 确定、离线；MUST NOT 在展开期访问网络、读环境或全盘扫描项目（SPEC-STYLE 规定的具名 build integration 除外）；MUST NOT 注入全局注册、隐藏 `.clone()`、隐藏 `Rc` 提升或线程本地开关。
- 失败模式：展开依赖外部状态 → 不同机器构建出不同语义；隐式 clone → 用户看到的移动/借用规则与实际所有权不一致，性能与预期脱节。

### SPEC-MACRO-022 节点/绑定/事件一次性建立

- 强度：**MUST**
- 规范：每个节点、绑定、事件 MUST 在 mount 建立一次；binding 函数体 MUST NOT 在 mount 求值；事件 MUST NOT 在 mount dispatch；显式求值 MUST 每次只读一次；事件每次激活 MUST 只 dispatch 一次。
- 失败模式：重复建立 → 同一字段更新导致多次重算/多次请求布局；setup 重跑 → 局部状态（焦点、输入）丢失。
- 状态：**PROBE**（探针集成测试已断言“绑定不被急切求值、mount 不触发事件、显式求值一次”；记录见 `EVID-VAL-*`）。

### SPEC-MACRO-023 `TextInput` 的 `value` 是专属 adapter

- 强度：**MUST** / **MUST NOT**
- 规范：`value={signal}` 在 TextInput 上 MUST 走专属 adapter（committed epoch/origin），MUST NOT 被泛化成所有 String props 的通用绑定；普通 `text` 节点仍 MUST 要求 `move || ...with(Clone::clone)`。`value` 与 `on:input` MUST 双轨：`value` 只负责外部→控件，`on:input` 只负责控件→模型；MUST NOT 提供隐式双向绑定。
- 失败模式：把 `value` 当通用 binding → 用户输入被回灌成 `ExternalReplace`，caret 跳尾、IME 中断；删掉 `on:input` → 输入不进模型；删掉 `value` → 外部清空/撤销不反映到控件。三者都必须在 A1 的 TextEdit fixture 中区分。
- 状态：**SPEC**；epoch/reducer 细节归 [05-text.md](05-text.md)。

### SPEC-MACRO-024 应用错误的呈现边界

- 强度：**MUST** / **MUST NOT**
- 规范：组件/命令错误 MUST 通过显式 typed prop（如父级传入的 `Signal<Option<String>>`）或统一 sink 呈现；宏 MUST NOT 自动生成全局错误 UI。可见错误 MUST 是普通文本节点（`role="alert"`），MUST NOT 自动抢焦点，MUST NOT 清空用户草稿。
- 失败模式：宏吞错误或自造 toast → 错误不可追踪；错误展示抢焦点 → 用户正在输入的内容被中断。**UI 错误展示不等于屏幕阅读器播报**，后者只能由 Gate C 真机验证。

## 3. 现状：探针的 PROBE 边界

以下条目只描述“已存在的小宏能做什么”，**不构成产品能力**。每项都只以 `EVID-VAL-*` 引用，明细与命令见 [../evidence/validation.md](../evidence/validation.md)。

| 能力 | 状态 | 证据 | 边界 |
|---|---|---|---|
| rstml 能解析目标 RSX 的 node/component/attribute 形状 | PROBE | `EVID-VAL-08`（H1 夹具，不可复跑） | parse-only，不产生任何运行时结构 |
| `view!` 接受 `view`/`text`/`button`，`class` 字面量编译为 StyleRule，`on:click`/`change`/`input` 保留为 Rust 闭包 | PROBE | `EVID-VAL-02`（H1）、`EVID-VAL-13`（H2 本轮复跑） | 无组件、无 props、无 `aria:`/`style:*` |
| 3 个 compile-fail：DOM 标签、脚本字符串事件、未知 class | PROBE | `EVID-VAL-02`、`EVID-VAL-13` | 错误文案已固定为 stderr 快照；unknown_class 目前高亮整段字面量 |
| 绑定不在 mount 求值、事件不在 mount 执行、显式求值一次、事件不重定向 | PROBE | `EVID-VAL-01`（H1）、`EVID-VAL-12`（H2） | 仅 text content 与单一事件 |
| 组件/PascalCase、props、children、slot、`Show`/`For`/`Index` | SPEC | 无（探针未覆盖） | 探针明确未实现 |
| 宏产出生产 `MountPlan` | SPEC | 无（`EVID-VAL-17` 只证明 0 tests） | 现 `Node` 是 inspection record，**不得**改名冒充生产 plan |
| 精确 class 子 span | OPEN | 无 | 现为整串 + byte offset |
| 依赖重命名（renamed facade） | OPEN | 无 | 现硬编码 `::zenui_macro_runtime` |

`archive/review/README.md` 与 `VALIDATION.md` 都记录了探针的 0-test target；任何“宏已完成”的表述都 MUST 以上表为准，0 tests 不算完成。

## 4. OPEN：需要裁决的宏侧问题

| ID | 问题 | 影响 | 解除条件 |
|---|---|---|---|
| OPEN-MACRO-01 | 如何区分 `{name}`（命名闭包）与 `{value}`（一次性值）：按签名 trait、显式 wrapper，还是要求绑定位置只认 `move ||` 字面量 | IDE 补全与误用率；trait 方案可能把 `Fn` bound 泄漏到用户代码 | 先写三种典型用法 + 两种边缘用法的 fixture，再定签名 |
| OPEN-MACRO-02 | intrinsic/控件属性清单与首批 widget（`form`、`text-input`、`checkbox` 等） | A1 可用语法面 | A1 冻结清单，并给每个控件 payload 类型 |
| OPEN-MACRO-03 | `-> MountPlan` 是否由 `#[component]` 自动包成 `Ok`，还是只接受 `MountResult<MountPlan>` | 新手路径长度 vs 单一失败语义 | 以“是否会有人真的需要不可失败组件”为判据 |
| OPEN-MACRO-04 | children/slot 的语法与多根规则（`children: Children` vs 命名 slot） | 组件组合表达力 | 结合 A1 的 `Panel` 目标示例定稿 |
| OPEN-MACRO-05 | 编译错误是否引入稳定错误码（`ZENU-*`）与编号空间 | 可搜索性、文档链接稳定性 | 与 SPEC-STYLE 的 class 诊断编号一起定 |
| OPEN-MACRO-06 | typed builder 等价路径的公开返回类型与模块位置 | “不开宏也能单测”的可用性 | 先让 builder 生成与 `view!` 相同的 descriptor golden |
| OPEN-MACRO-07 | 展开性能与 IDE 导航（rust-analyzer 对大 `view!` 的响应） | 大页面 DX | A1 后采集展开体积/耗时，不预设预算 |
| OPEN-MACRO-08 | `{..props}` spread 是否支持、class 合并语义（声明集合并 vs 覆盖） | 组合组件写法 | 没有真实用例前不进入主 API |
| OPEN-MACRO-09 | 动态 class 的表面语法：登记 enum 写成 `class={C::Active}`，还是独立属性（如 `class-set`） | 与 `SPEC-MACRO-013` 的字面量规则、IDE 补全、错误信息一致性 | 先写 3 个真实用例（状态高亮、主题切换、用户自定义布局），比较两种拼写的可读性与误用率 |

## 5. 验收映射（A1）

宏侧条目只有在下列 fixture 存在时才可离开 `SPEC`；全部 Gate 的状态以 [08-gates.md](08-gates.md) 为准，本文不复制 Gate 判据。

- golden：同一 `view!` 与 typed builder 产生相同 descriptor/IR；`move ||` 被记录为“注册”而非执行。
- trybuild/compile-fail：mount 错误路径（`?` 失败不产生半个节点）、未知 class、`For` 缺 key、错误 props 类型、错误事件 payload、`on:*` 字符串、拼错事件名、`class:` 自造条件。
- renamed facade：独立 package 只改依赖名 + 只 import facade 仍编译。
- 诊断快照：每条错误锁定 span 位置与消息文本（含 help），不得用 “token error” 兜底。
- 反例：把 inspection `Node` 改名为生产 `MountPlan`、用 `display:none` 实现 `Show`、把 `value` 泛化为通用绑定，三种都 MUST 被判为失败。
