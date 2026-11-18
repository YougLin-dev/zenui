---
doc-id: EVID-PROBE
layer: evidence
status: normative
date: 2026-11-18
owner: 测试与 CI/发布负责人（10y+）
sources:
  - examples/macro_poc/（commit ab5339b）
  - examples/desktop_poc/（commit ab5339b）
  - examples/native_poc/（commit ab5339b）
  - examples/todolist/（design-only）
  - archive/review/text-layout-review.md
replaces: []
---

# 探针台账 EVID-PROBE

本文件回答一个与 `validation.md` 不同的问题：**这些探针各自能证明什么、不能证明什么**。
`validation.md` 记"跑了什么、输出是什么"；这里记"这份代码的能力边界与证伪条件"。
命令与主机不在本文件重复，见 [`validation.md`](validation.md)。

`PROBE` 的含义（`docs/README.md` §2）：**已有探针级最小验证，不等于产品完成**。
引用任何 `EVID-PROBE-*` 时必须同时引用该条的"不覆盖"行；只引前半句就是虚报。

## 0. 总览

| ID | 探针 | 入口 | 可运行 | 覆盖的层 | 对 Gate 的贡献 |
|---|---|---|---|---|---|
| `EVID-PROBE-01` | rstml 宏前端 | `examples/macro_poc` | 是（`cargo test`） | 语法解析 → 静态 inspection Node | 只到 A1-02/-03 的"形状" |
| `EVID-PROBE-02` | 平坦响应式图 | `examples/desktop_poc` → `lib/project_signals.rs` | 是（`--lib`） | 字段 sink / 依赖替换 / 去重 / 整图 dispose | 只到 A0-04 的"会去重"，且**语义与 A0-03 冲突** |
| `EVID-PROBE-03` | typed utility 编译 | `examples/desktop_poc` → `lib/style.rs` | 是（`--lib`） | class 字面量 → 8 个属性枚举 + 单激活状态 `resolve` | 只到 B-05 的"unknown token 可诊断" |
| `EVID-PROBE-04` | 文字 / 布局 / GPU | `examples/native_poc` | 是（`--lib` + 二进制） | Parley shaping、Taffy 测量、sparse-strips readback | 只到 B-01/-02 的**缺陷证明**与 D-01 的"不是模型" |
| `EVID-PROBE-05` | 目标语法 | `examples/todolist` | **否**（40 个编译错误） | 无 | 无；它是 negative evidence |
| `EVID-PROBE-06` | A0 行集投影的边界 | `crates/core/src/{query,txn}.rs` | 是（`--test queries`） | 共享行集快照与单接线 | 只到 A0-07 的计数判据；`Memo<RowSet>` 归 A1 |

---

## 1. `EVID-PROBE-01` — rstml 宏前端

**入口**：`examples/macro_poc/macro/src/lib.rs`（proc macro）、`runtime/src/lib.rs`（生成代码用的 inspection 类型）、`frontend/src/lib.rs`（公开 facade）。

**身份**：把一段 RSX 降级为 `Node` 构造代码。`Node` 是 **inspection record**，不是生产 MountPlan，也不是 retained View。

### 覆盖（可以引用）

| 能力 | 证据 |
|---|---|
| 只接受 `view` / `text` / `button` 三种标签，其余在编译期报错 | `macro/src/lib.rs:46-51` |
| 属性只接受 `class`（字面量字符串）与 `on:click\|change\|input` | `macro/src/lib.rs:61-82` |
| 事件必须是 Rust 表达式，字符串脚本被拒绝 | `macro/src/lib.rs:150-153` |
| 每个元素最多一个文本内容子节点 | `macro/src/lib.rs:89-99` |
| 文本内容只接受字符串字面量或 `move || String` 闭包 | `macro/src/lib.rs:164-202` |
| 闭包在 mount 时不执行，只在显式 `evaluate_content()` 时读取一次 | `runtime/src/lib.rs:158`；`frontend/tests.rs:33-36`（`binding_reads == 0` 后 `== 1`） |
| 事件在 mount 时不派发 | `frontend/tests.rs:34`；`runtime/src/lib.rs:142` |
| 调用方无需导入内部 `Node`/`Content` | `frontend/src/lib.rs:1-3` |
| `class` 字面量编译成 typed `StyleRule`，未知 class 报 byte offset | `macro/src/lib.rs:204-213` |

### 证伪边界（不可引用）

- **不覆盖 PascalCase 组件**：`<TodoItem .../>` 会落到 `unknown tag` 分支（`macro/src/lib.rs:46-51`）。
- **不覆盖 `For` / `Show` / `Index`**：没有对应分支；它们不是"没测"，是**不存在**。
- **不覆盖 props / slot / ChildrenFactory**：没有组件概念，也没有 children 工厂。
- **不覆盖嵌套动态片段**：HTML 注释、doctype、raw text、fragment 全部走 `lower_node` 的错误分支（`:131-135`）。
- **不覆盖 >1 个根元素**、不覆盖非容器根（`:28-40`）。
- **不覆盖事件 payload**：handler 签名是 `FnMut()`，无参数（`runtime/src/lib.rs:138`）；typed payload 无从谈起。
- **不覆盖 renamed facade**：生成代码硬编码 `::zenui_macro_runtime`（`macro/src/lib.rs:112`）。只 rename 的独立 crate 必然失败，这正是 A1 red fixture 的素材。
- **不能当作 MountPlan**：`Node` 自带 `fire()`/`evaluate_content()`，可独立求值、可独立派发（`runtime/src/lib.rs:142,158`）。规范明确禁止这种形状成为第二棵树。

### 升级路径

要变成 A1 证据，缺的不是测试数量，而是**类型**：`Node` → `NodeDescriptor` + `BindingDescriptor` + `ChildrenFactory`，且 `fire`/`evaluate_content` 不得出现在生产类型上。

---

## 2. `EVID-PROBE-02` — 平坦响应式图

**入口**：`examples/desktop_poc/src/lib/project_signals.rs`（`Signal<T>` / `Owner` / `Patch`）。

**身份**：单线程、无 unsafe、无环的平坦 field sink 图。用于证明"字段级更新可去重、可替换依赖、可整图释放"。

### 覆盖（可以引用）

| 能力 | 证据 |
|---|---|
| 同帧多次写同一 source，只产生一次 sink 执行 | `project_signals.rs:164-169`；测试 `multiple_writes_in_one_per_frame_batch_deduplicate_the_sink` |
| 依赖变化时旧边被移除（不残留误订阅） | `:117-160`（`detach` 后再 `current.replace`）；测试 `changing_dependencies_removes_inactive_edges` |
| 无强环：`Signal` 只持 `Weak` graph 与 `Weak<Cell<bool>>` | `:20-24` |
| 整图 dispose 后不再应用更新 | `:171-181` |
| 只有匹配 source 的 sink 被重算 | 测试 `source_update_runs_only_matching_sink` |

### 证伪边界（不可引用）

- **不是 generational arena**：`Signal` 只有 `id` + 一个 `valid` 弱旗标（`:20-24`），**没有 generation**。slot 复用后"读到旧行"在类型上仍可能发生。
- **`Patch` 的 node 是裸 `u32`**（`:11-15`），不是 `ViewId + generation`。A0-03 要求的身份分离在这里完全没有体现。
- **`Signal::set` 返回 `()`**（`:47`），**不是 `Result`**；无效 handle 的写入**静默返回**（`:48-51`）。这与 `SPEC-GATE-A0-03`（旧 handle 必须得到 `InvalidHandle`）**语义相反**——不是"未覆盖"，是"当前实现的选择被规范明确否决"。
- **`with()` 对失效信号仍会执行 `get`**（`:26-45`）：读旧值不报错。
- **`bind()` 立即 `flush()`**（`:117-127`）：绑定在安装时就求值一次。不能拿这个探针证明"mount 不执行绑定"；那条性质目前只由 `EVID-PROBE-01` 的宏侧观测支持，二者不可混用。
- **`batch()` 只是"末尾一次 flush"**（`:164-169`）：嵌套调用时内层会提前 flush，这正是 `EVID-VAL-08` 记录的中间值反例。
- **`subscriber_count()` 数的是 bucket 数**（`:183-185`）：空 bucket 会被当成活跃源。FINAL A0 明确要求新 core 修掉。
- **`dispose()` 是整图释放**（`:171-181`），不是 owner 子树释放；没有嵌套 owner。
- **不覆盖**：Memo、拓扑稳定、Store/query、结构 patch、焦点、事务 journal。

### 升级路径

`slot + generation` 的 handle、`Result` 化读写、嵌套 `batch` 深度、subtree owner —— 四件事任一未做，这个探针就只能证明"平坦图能跑"，不能证明 core。

---

## 3. `EVID-PROBE-03` — typed utility 编译（桌面探针的样式模块）

**入口**：`examples/desktop_poc/src/lib/style.rs`。

**身份**：`class="..."` 字面量 → `Rule{modifier, declaration}` 列表 → 按一个激活状态 `resolve()` 成 `BTreeMap<Property, Value>`。

### 覆盖（可以引用）

| 能力 | 证据 |
|---|---|
| 白名单外的 token 报错并带 `token` + `byte_offset` | `style.rs:56-60,126-134`；测试 `invalid_values_and_unknown_classes_report_source_location` |
| 非法数值被拒（`p--2`、`w-[100%-2rem]`、`hover:bg-???`、未知前缀 `side:`） | 同上 |
| 属性映射到 typed 枚举而非字符串（`Display`/`Padding`/`Gap`/`Radius`/`Width`/`Background`/`TextColor`/`BorderColor`） | `style.rs:25-35,94-133` |
| 三个 modifier：`Base` / `Hover` / `FocusVisible` | `style.rs:37-43,69-78` |
| 命中态只覆盖同属性，不动其他属性 | 测试 `hover_only_supersedes_the_matching_property`（`style.rs:216-232`） |

### 证伪边界（不可引用）

- **白名单只有 9 个精确 class + 4 个前缀族**（`flex`、`flex-row`、`flex-col`、`bg-blue-600`、`bg-blue-800`、`text-zinc-100`、`border-amber-400`、`rounded-md`，以及 `p-*`/`gap-*`/`rounded-*`/`w-*`，且值不能含 `[`）。这是**样本**，不是 Tailwind 覆盖率（`style.rs:94-133`）。
- **`resolve()` 一次只接受一个激活状态**（`:181-189`），没有"多条件同时命中"的概念：pressed/disabled/checked/aria-pressed/theme 全都不存在。
- **完全没有 origin / importance / canonical order**：`resolve` 的规则就是"base 先写、激活态后写，同属性后者覆盖"（`:184-188`）。因此它**无法**证明 `SPEC-GATE-B-06`（普通 inline 恒胜），也无法证明任何 CSS cascade 顺序——它连 inline 都没有。
- **没有 provenance**：`BTreeMap<Property, &Value>` 丢掉了胜出声明来自哪条 rule。
- **没有宏/builder 双入口一致性的验证**：宏侧（`EVID-PROBE-01`）用的是另一份 `Property`/`Value`/`Condition` 枚举，与这里的 `Modifier` 不是同一套类型。

### 升级路径

要碰 B-05/B-06，必须引入 `origin`（inline vs class）、`importance`、固定版本 registry 的 canonical order，以及 provenance。现有代码里没有任何一个字段可以承载它们。

---

## 4. `EVID-PROBE-04` — 文字 / 布局 / GPU

**入口**：`examples/native_poc/src/{text,layout,gpu}.rs`；`layout` 模块是 `#[cfg(test)]`（`lib.rs:3-4`），**只在测试里编译**。

**身份**：三段互不耦合的最小适配器 —— Parley shaping、Taffy leaf 测量、sparse-strips 光栅化 + 外部纹理 + readback。

### 覆盖（可以引用）

| 能力 | 证据 |
|---|---|
| CJK 文本产生多个 glyph run，且软件像素非空 | `lib.rs:20-43`（`EVID-VAL-03/-14`） |
| `parley_engine` 的中日文词典确实改变切词边界 | `lib.rs:45-60`（边界断言 `[true,false,true,false,true,false]`） |
| Taffy 测量会调用 Parley，且换文本后尺寸真的变化、dirty 状态正确 | `layout.rs:33-93` |
| 半透明红在 CPU 上是 premultiplied `[128,0,0,128]` | `lib.rs:67-71` |
| GPU：sparse-strips 产出可见 glyph 像素、外部 `TextureView` 采样、premultiplied overlay | `gpu.rs:86-102`（4 条像素断言）+ `:124`（半透明 readback） |
| 两套后端（Vulkan/NVIDIA、Metal/Apple M4）得到同一个 readback 值 | `EVID-VAL-05`、`EVID-VAL-15` |

### 证伪边界（不可引用）

- **baseline 缺陷（已证实，不是推测）**：`Paragraph::measure()` 算了 `first_baseline()`（`layout.rs:27`），但交给 Taffy 的闭包只返回 `.size`（`layout.rs:104`）。被测绿的是"尺寸会变"，不是 baseline 正确。
- **intrinsic 缺陷**：测量只用 `known.width`，没有区分 available space 与 min/max-content（`layout.rs:104`）。
- **字体合成缺陷**：全 crate 零处引用 `synthesis`/`embolden`/`skew`，faux bold/oblique 不会被转发。
- **字体选择固定**：字号 24、行高 1.25、无 family 指定（`text.rs:18-21`），用主机默认字体；`text.rs:2` 自己写着 fallback 只在本机被观察过。
- **GPU 场景固定**：128×64、`Rgba8Unorm`、单个 target 连续渲两次（`gpu.rs:12-14,55-120`）；`depth_view` 被创建并传入，但**没有深度遮挡断言**，也没有 3D 几何。
- **绿色 clear ≠ 模型**：外部纹理 view 被固定清成绿色（`gpu.rs:184-189`），断言"采到了外部纹理"只证明绑定生效。
- **`atlas_cache(false)`**：刻意关闭实验性缓存（`text.rs:59`），所以不涉及 cache 正确性或性能。
- **不覆盖**：窗口、surface、resize/DPI、device-loss、mask/filter/clip、多 viewport、资源生命周期、任何计时。
- **程序自报主机错误**：输出里的 `SCOPE: ... Windows ...` 是硬编码字符串，与实际主机无关。

### 升级路径

baseline/intrinsic/synthesis 三项是**修代码**，不是加测试；修完之前 B-01…B-03 无法离开 `OPEN`。

---

## 5. `EVID-PROBE-05` — 目标语法（negative）

**入口**：`examples/todolist/main.rs` + `README.md`。

**身份**：**目标 API 的形状说明**，不是可运行程序。根 `Cargo.toml` 设 `autoexamples = false`，所以它不会进入构建。

### 覆盖

无。它不执行、不渲染、不产生任何能力证据。

### 它能证明的一件事

**它当前不可编译**：单独交给 `rustc` 会得到 40 个错误（缺 `zenui` crate、缺 `component`/`view` 宏、缺 `CompletedField` 等）（`EVID-VAL-19`）。

### 证伪边界（不可引用）

- 不得引用它证明任何组件宏、`For`、`Show`、`TextEdit`、事务或错误展示能力。
- `MountResult<MountPlan>` 里 `?` 的**语言形状**证据在仓库外夹具（`EVID-VAL-07`），**不在这个文件里**；本文件只是写法示意。
- README 里描述的 `Transaction::new().write(...).finish()`、`report_runtime_failure`、`aria:pressed` 派生、focus intent 顺序，全部是**规范**（`SPEC-*`），不是已实现行为。
- 如果哪天它被接进构建并"编译通过"，那也只是宏开始存在，不等于 A1。

---

## 6. `EVID-PROBE-06` — A0 的行集投影边界（ADR-0023）

**入口**：`crates/core/src/query.rs` 的 `QueryCache::rows_at`、`crates/core/src/txn.rs` 的 `Tx::rows`。
可运行：`cargo test -p zenui-core --test queries`（`EVID-VAL-42`）。

### 它能证明的一件事

A0 的共享行集投影由缓存快照承担：`Tx::rows` 返回的就是该快照，两个消费者共享同一份分配
（`RowSet::shares_with`），不克隆 `String`。`SPEC-MACRO-011` 括号里「或 runtime 声明的同构投影」
在 A0 指的就是这一形态（[ADR-0023](../decisions/adr-0023-row-set-projection-scope.md) 决定 1）。

### 它不能证明的（引用 A0-07 时必须一起引用这一节）

- **`Memo<RowSet>` 未在 A0 验证，归 A1。** A0 没有 `For`、没有订阅者，代码里也没有任何 memo 节点
  承载行集。把 A0-07 的 `PROBE` 读成「行集 memo 已验证」是误读。
- 可订阅的 `Memo<RowSet>` 随 `For` 在 A1 落地，届时**只能**从 `(membership revision, order revision)`
  派生，不得读整表或 store 节点。
- A0-07 的 `PROBE` 只覆盖计数判据与单接线，不覆盖任何节点订阅语义。

### 证伪边界（不可引用）

- 本条的"能证明"只对 `zenui-core` 的三个 filter 成立；不含真实布局、渲染与窗口。
- 若将来有人把一个 memo 节点当作 A0-07 的证据，同时违反本条与 ADR-0023。

## 7. 交叉提醒

1. **两个探针对"绑定何时求值"给出相反观测**：宏探针 mount 时 `binding_reads == 0`（`EVID-PROBE-01`），响应式探针 `bind()` 立即 flush 并求值（`EVID-PROBE-02`）。引用时按探针分别标注，不要合成一条结论。
2. **两套样式类型互不相通**：宏侧 `Property/Condition` 与桌面侧 `Property/Modifier` 是两份独立枚举，任何"宏与 builder 产出一致 IR"的说法都还没有证据。
3. **所有探针的 `Patch`/`Node` 都不带 generation**：这是三个探针共同的天花板，也是 A0 必须新写 core 而不是改造探针的原因。

## 8. 维护

- 新增探针先在本文件登记 `EVID-PROBE-0N`，写明入口、可运行性、覆盖、**不覆盖**，再写测试。
- 探针的能力一旦升级（例如 baseline 修好、宏支持 PascalCase），更新对应条目并注明 commit；不要删掉旧的"不覆盖"行，改为写明从哪个 commit 起不再成立。
- `EVID-PROBE-*` 与 `EVID-VAL-*` 是两层：前者是"代码现在能做什么"，后者是"某次运行看到了什么"。同一个事实两边都要引用时，以 `CLAIM-*` 作为唯一断言行。
