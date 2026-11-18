# Zenui 跨领域评审统一规格 v1

日期：2026-11-18。本文是**目标设计规格**，不是已实现的框架。已运行证据见 `VALIDATION.md`，三个阶段的原始意见见 `round1-experts.md`、`round2-debate.md`、`round3-review.md`，文字专项核验见 `text-layout-review.md`。

## 0. 最终技术路线

**单一原生保留 View 树 + 单一细粒度响应式图 + 一次性 MountPlan，拒绝 VDOM 或整组件反复重建。**

组件仅在实例 mount 执行一次；Signal/Memo/Store 查询经 tracked binding 计算字段/结构 patch；只由同一 UI 主线程事务提交这些 patch。粒度是脏传播/重算/提交范围，不声称为每帧全局 O(1) 或局部 GPU 自动光栅化。

| 子系统 | 唯一候选 | 当前事实边界 |
|---|---|---|
| 前端 | `rstml 0.13.1` + syn 3 + quote；HTML 外形、Rust 表达式/typed props | 小型真实宏验证；完整组件/For 未实现 |
| 运行时 | 安全 Rust Owner/arena/字段订阅、拓扑 Memo、显式事务 | 现 PoC 只有平坦 sink 图，须先过 A0 |
| Store | 模型实体+字段/查询依赖，非另一个刷新权威 | 目标合同，未有完整实现 |
| 布局 | Taffy 0.15.X 投影、统一 GeometrySnapshot | 实际 adapter 有遗留 baseline/intrinsic 缺口 |
| 文本 | Parley 0.13、自有 TextEditWidget 与输入法/可访问性 adapter | CJK/ICU已小验；编辑/bidi intrinsic 待验 |
| 渲染 | 候选 wgpu 30.0.2 + vello_gpu/vello_cpu 0.4 + glifo 0.5 | 单机 headless 已验；surface/能力矩阵未验 |
| 样式 | 固定版本 utility 白名单+原生 typed StyleIR | 极少数探针≠Tailwind v4 覆盖率 |
| 窗口 | winit 0.30.x、AccessKit / platform services | 没有真实窗口/输入/屏幕阅读器验证 |
| 3D | 自有离屏 color/depth + 普通 TextureView | 只有占位纹理/三角，真实 glTF未完成 |

版本沿用当前研究 pin：taffy 0.15.1、Parley 0.13、sparse-strip Vello 0.4、wgpu 30.0.2；winit 0.30.13/accesskit 0.25.2/accesskit_winit 0.34.2 为待组合基线。正式 pin 以单一 workspace 与 lock 统一，CPU/Metal 等跨平台能力要另验。Classic vello 0.12 **仅为 probe，不承诺第二生产后端**。不默认引入 Bevy/Tokio/QuickJS/完整 DOM/CSS。

## 1. 权威、身份、Owner 与组件执行

### 1.1 一棵 View 树、一张 graph

- **View tree** 持有稳定 `ViewId+generation` 和 widget 本身的状态（focus、scroll、selection、composition、hit mapping 等）。布局和保存的绘制记录是从该树投影/cache 派生的，不能分离成第二份可独立提交的 UI 权威。
- **Reactive graph** 管理 Signal/Memo/field-query、订阅关系与产生 patch 的计算。Store 只作为模型及字段依赖适配，不另跑全量 UI render。
- **OwnerScope** 管理 component/结构子树的订阅、异步 ticket、cleanup 和回收。`ViewId`、`OwnerId`、`TodoId`、`SignalId`、`Taffy NodeId`、ResourceId 是不同的 newtype，不能一个裸 u64 同时充当。
- 生产 field handle 目标为 `{runtime_id, slot, generation}` 的轻量 `Copy` 结构，但内部可安全 Rc/RefCell 实现，不能公开为业务生命周期工具。Copy 不等于永久有效，不意味着访问无成本。lookup 应返回 `Result<_,InvalidHandle>`；不能返回残留旧值、转入同 slot 新对象或静默写失败。数据 binding 可能把检查转交运行时，不用每次用户手写 Result，但错误必须进入 sink，不可忽略。
- 事件分发先检查 UI generation，业务写再核 Store row generation，异步再核 Resource ticket。过滤消失使旧 UI command 失效但不删模型；模型删除才使 row fields 失效。

### 1.2 一次性 MountPlan

```text
Component::mount(Props, parent_scope)
  -> 创建 component Owner
  -> Rust setup 只在 mount 时执行一次
  -> Rust `Result<MountPlan, MountError>`（可提前失败）
  -> descriptor { kind, initial_fields, binding slots, children/listeners }
  -> runtime commit 时才建 retained native View+ViewGeneration 并注册 binding
```

与现有 `macro_poc::Node` 区分：它是 inspection builder；**不是生产 MountPlan，不演进为另一棵 UI 树**。生产 descriptor 只能在事务统一 commit 时创建 View，不可单独 fire / evaluate / render。

- **普通组件实参 `{expr}` 是 Rust 值或 handle 本身**。如 `current: Signal<Filter>` 属性写 `current={filter}`，传的是 signal handle，不是 `filter.get()`。类型正确时 `Result<MountPlan>` 内可用 `?` 提前取消该组件的 mount；失败不发节点/订阅或默认标题，错误进入统一 sink。
- 普通 owned `String`、`&'static str`、Copy enum 与高分组件 Props 都不要求 Copy；一个 struct 表示一组 typed props，而不是把 String 塞进 Copy row。
- Child/Slots 是显式 `ChildrenFactory`，多根规则由类型控制；无默认 dynamic closure 猜测。子工厂在 Show/For etc 按需执行，不得在 parent Show false 时提前执行。
- 事件仅接收控件特定 payload 的 `FnMut(E)`，或由框架 adapter 授权的 `FnMut(E) -> Result<(),AppError>`；mount 不执行，事件环境不做 tracked 读取。内部可做类型擦除，公开 builder 不接收任意事件字符串或无类型的空回调。**统一错误路径**：普通 `Signal::set` 返回 `Result`；result-aware 事件 adapter 把错误交给应用 sink，直接写 setter 的示例也必须显式处理错误，不得把返回错误的 setter 伪装成 void 来迎合事件签名。无效 generation 属领域失败，而不是可忽略的普通返回值或业务 panic。`TextInput.value` 与其 `on:input` 用同一次控件事务，专用适配器把写入失败路由到同一 sink。

### 1.3 瞬态 UI 与数据模型生命周期

- `Show` 是 mount/unmount，`hidden` 是不参与布局/绘制/命中/语义树的呈现策略；`visibility:hidden` 保 layout 但不绘/命中/聚焦；disabled/inert/clip 分别表述。
- `For` **强制 key**，无隐式 index fallback。重复 key 在 patch 前拒绝，保留之前合法 View 状态。
- `FocusRemovalIntent` 在结构 mutation 前捕获受影响控件、同 kind 的前后邻居、可用的输入/工具栏回退和 command generation。reducer 只在 commit 后选择仍存活且可见的后继；先匹配同 kind 下一项，再同 kind 上一项，再可聚焦的输入，最后筛选工具栏。禁用或被裁剪/隐藏的控件不可聚焦时就跳过；删除最后一个匹配项后允许落到输入而不伪造“下一个”。批量 Clear 只结算一个 intent。若用户在本事务或 commit 后至 dispatch 前主动转移焦点，原 intent 必须取消，不能重新抢回。
- `For reorder` 保 ViewId、generation、Owner、subscriptions、local widget/caret/selection/focus。
- key 消失/filtered out **默认 dispose UI owner 与订阅**；row model 不归 For 拥有，仍在 Store 中。再次同 key 出现创建**新 UI generation**，旧 click/IME/resource ticket 永不复活。
- 不支持 `retained-visibility` 与无限 hide-store 地宣称编辑状态保留，也不能把 filtered out 偷换成数据删除。要保存行内草稿，必须显式选择有界的 `retained_visibility` 或 `dispose_and_snapshot(key, TTL)`，定义独立容量、焦点、IME 和真删除清理规则；第一 Todo 不承诺这项能力。
- `Index` 仅用于按位置的 slot；改变 index 的 content 更新 item signal，不能直接当 For 使用。

## 2. Store query 与所有数据接线

模型示例：`TodoId -> {immutable title, completed field}`；可取得稳定 id 与轻量字段 handle。Query 一行读取发生在 tracked domain 内，**字段与查询边都属同一张 reactive graph**。

```text
field/membership mutation
  -> reverse field edge / membership index 标脏对应 shared query cache
  -> query lazy evaluation 才访问 row 的相匹配字段/索引
  -> query value revision 改变
  -> Memo<RowSet> 跟踪 query revision（不为每个消费者再连 N 条字段边）
  -> For reconcile 数量与次序
```

唯一接线合同：

1. `(query_id, filter)` 的共享 query cache 由 store/computed graph 拥有，不得另设无订阅的 Vec 当第二现实。
2. 该 query 求值时登记适用的 membership/候选行索引，以及**求值实际读取的 `(row, completed)` 反向边**。Active/Done 不能只给已物化的 matching rows 建边：候选索引必须覆盖因字段变更而新加入/离开 filter 的行；否则非 member 改 completed 不会使 query 标脏。命中字段或索引只标脏 shared query cache，不立即 materialize。新增的候选行可在下次共享求值时扩大实际 field-edge 集，replace/unregister 不能遗留空桶或误连旧 consumer。
3. `rows(filter)`：All 不因 completed 改 membership revision；Active/Done 在有关字段变更后会重新评估/物化成员。**不能把 filter 已缓存结果又忘掉关联字段的监听。**
4. `RowSet` 是共享的有序 id/projection 数组（Arc 或等价实现），只包含 handle，不包含 String。`get()` 对通用 Memo<T> 只允许 T: Copy；For 接收已声明的 `Memo<RowSet>`。“O(1) handle clone”不等于查询/O(n) 分配消失。
5. field write 不直接 materialize RowSet。每个 `(store_epoch,filter)` 在 topo 稳定后、真实 list 首读时共享评估一次，至多一次 ID materialization，第二个消费者只共享 snapshot；空 filter 有独立 scalar empty，绝不能通过 `visible.get()` 又物化。
6. `has_rows` 来自 store counter；`is_empty(filter)` 来自 query 的标量 empty，**禁止** 对 row collect、遍历或 Vec.get()。必须测 row_visits=0、materialization=0、String clones=0，而不只是布尔返回值恰好正确。
7. `remaining/has_completed` 由字段感应聚合/同名 query graph 索引派生；成员插入或删除必须更新 reverse membership edge；字段 write 不得漏发 remaining/has_completed。一个 list 的行字段变化不能影响另一个不相关 title 的读取/订阅。
8. 不可变 title 以 `Result<String,InvalidHandle>` 从未删除的 model 中于每次 UI mount 读取一次；filter 返场伴随新 mount，因此可再读一次，reorder 或 completed 改变不能重读。它是普通 Rust String，位于 onetime prop/text，而非 Copy 模型行。

以上所有合同需要真实 query_eval、id_materialize、edge_count、subscription count、String clone 数据，只靠 Memo/Store 名称不能判断对错。

## 3. Frame transaction、提交及错误边界

```text
上次已 publish GeometrySnapshot 用于命中与无障碍 action
  -> Outside：核 UI/模型/资源 generation；用户事件写 Source/ queried model
  -> Mark：source/字段查询索引标脏；nested transaction depth 只增厚
  -> topological Memo/Query：到稳定值才传播；equality 只抑制无意义下游
  -> Binding compute：只产纯 typed patch，禁止外部世界写
  -> Full preflight：key/topology/style/generation/focus/IME/resource/Taffy 操作写 journal
  -> CommitPlan：一次性推进 ModelScratch + ViewScratch + 所有者/资源事务 epoch
  -> committed cleanup：释放 staging 中已删除 owner，不直接 flush/mount/dispose 子回调
  -> layout scratch / geometry snapshot publish
  -> retained paint / external 3D texture / presentation
  -> 成功后才启用 postframe effects、deadline 或 ControlFlow::Wait
```

### 3.1 写入在哪里发生

- 业务事件可写 Signal/Store；Memo/binding 只能算值。`WriteDuringCompute`、memo cycle、measure 中写 model 都是领域错误。
- `batch` 是嵌套深度事务，最外层离开才结算/提交一次；不能内层 leak intermediate boolean、不能 binding 事务中同步 flush。
- `untrack` 只关订阅登记，不开放 measure/commit 不写；post-frame Effect 承担 I/O，不担任本帧必须同步完成的 binding。
- Memo internal mark/version update 时 runtime privileged path，和用户业务调用 set 权限分开。

### 3.2 事务失败不能制造半棵树

- preflight journal 包括字段、key/order、resource/generation tickets、owner/focus/IME、ViewId→Taffy 映射及 widget state queue；在权威树之外演算。
- 任何 key duplicate/InvalidGeneration/read failure/phase violation 都丢弃本事务的 model+派生 scratch + View journal。pending snapshot、geometry epoch、focus pointer、ID mappings 与之前提交相比必须完全一致，不能只回滚一部分。
- commit 时不执行用户 setter 或 cleanup callback；提交后开始 postcommit cleanup/clocks。失败对 reactive phase/queue guard 恢复并允许下一干净事务，panic 仅作为额外故障边界测试，绝不作为业务错误 API。
- layout/paint 在差分 scratch 中执行；失败不能事后改变已提交的业务结果。已提交事务之后的 unsupported capability 通过 typed `Result::Unsupported`/last-good-frame 处理；设备或绘制故障发生在不同时间，不能靠一句“回滚 GPU 提交”假装业务事务可逆。
- 已发出的外部 I/O 不回滚；只能 postcommit 才启动，失败有单独 Result 和恢复路径。保存文件必须 generation 验证、必要 temp/fsync/rename，低层 platform errors 展示给用户。
- 数据/一次性投影/bindings 由 `BindingResult<T>` lowering 或 runtime adapter 把 `Result<T, InvalidHandle>` 交给同一领域错误 sink，无隐式旧值、无把 Result 塞进文字。`read_once` 采用真实 Rust `Result<String,InvalidHandle>`。mount 目标函数 `Result<MountPlan,MountError>` 接受 `?`，取消 mount 不建临时 UI。这一“结果型返回”作为目标 Rust Type 形状已单独夹具通过，但**宏降级与 `view!` 结果处理仍待 A1**。

## 4. 最终宏、Props、字段读取及错误呈现

| 写法 | 唯一意义 |
|---|---|
| `{value}` | mount 初值、普通 typed prop；不是绑定。若为 Signal/Memo/Store handle 则传递 handle |
| `{move || expr}` | intrinsic 的文字/attr/style/Show 等显式 `BindingDescriptor`；读取所在依赖自动登记 |
| `{name}` | 命名后的 `move ||` 闭包和显式内联写法同类，非 bool 当前值 |
| `on:input={move |String| ...}` | 控件明确 payload 的非追踪 handler；不写也不会自动注销事件内容 |
| `<For each={visible} key={|&row| row.id}>...` | `visible: Memo<RowSet>`，key 借用 id、行 factory 接收 id+字段 handle 投影 |
| `<TextInput value={draft} />` | **专属控件属性 adapter**，承担 committed epoch/origin，不是通用 clone string |
| `count.get()` | 只 Copy 标量/enum 读取（checked runtime，默认只返回正确世代值） |
| `text.with(|&T| -> R)` | 非 Copy 借读；true owned String 输出写明 `Clone::clone`，不接受 fake String Copy |
| `<For each=...>` | 不接受任何 free Vec 隐藏复制或裸迭代反射，projection/conversion 必须是已知类型 |

最终组件目标返回 `MountResult<MountPlan>`（即 `Result<MountPlan,MountError>`）。`view!` 生成正常 `MountPlan`；`#[component]` 与 setup glue 将无显式错误的普通函数体包装为 `Ok(plan)`。组件函数体可在真实 Rust 函数形状中使用 `?`；属性宏的 lowering/generator 仍须另做 golden fixture。当前仅验证普通 Rust `?` 的编译形状，尚未验证宏降级和 `view!` 的错误结果处理，见 A1。

- PascalCase 组件上的所有 `{}` 首先都是**普通 Rust 实参**。`current: Signal<Filter>` 对应 `{filter}`，传递 cheap handle；若参数需要 `Filter` 则显式 `move || filter.get()` 是支持的动态 props trait/参数 type，而不是由宏泛猜。
- `TextInput.on:input` 只写应用来源，value 只读 committed epoch。保留 `value`+`on:input` 双轨以体现控制流，**不是**伪自动双向绑定 API。普通 text 不会自动享有专属 TextEdit conversion。
- Styled intrinsic `view` 默认 block-like normal flow；row/column 明显指定 flex direction；`class=flex` 只设置 display flex，不改写 intrinsic DOM/浏览器 block。
- `Form` 是原生表单 role/frame；`SubmitEvent{origin, command_id/generation, default_prevented}` 是框架命令，不是 DOM event。一个普通 Enter/button activation 只产同一 SubmitCommand 并一次调用；IME candidate Enter 在前一阶段 consumed，不应 submit；`prevent_default` 仅取消默认命令，不是业务 add 触发器。
- 常规源码诊断优先使用 `syn::LitStr`/source map 定位完整 unknown token、For 缺 key、错误事件 payload 和组件类型不匹配。普通 `{value}` 用于动态位置的类型错误可建议 `move ||` 或在之前命名相应 typed Props，不能用 `missing move` 作笼统固定诊断。精确 token/attribute span 可由 source map 补足，回退时须标注是小范围近似，不把 class 字面量内的 byte offset 错当 proc_macro span。
- 未来正式 proc macro facade 不重复当前绝对 `::zenui_macro_runtime`；用 entry facade/manifest `proc_macro_crate` 等方案，独立 crate original package 不存在 only rename 的 trybuild 通过。公共 caller 无需导入生成私有 Node/Content/support types。

### 应用错误显示

- `TodoApp` 创建 `error: Signal<Option<String>>`，在组件调用位置通过普通 typed prop 显式传给 `TodoItem`。业务命令错误通过 error binding 展示 `role="alert"` 的可见文本，不自动抢焦点，不用错误提示清掉用户草稿。其他信号处理器的无效 field 由运行时 sink 路由，应用决定展示/记录。**UI 错误展示不等于已通过 AT 播报**，只有 C Gate 的真实读屏验证才能宣称 announcement。

## 5. TextEdit：同一 reducer 的两类更新

```text
session:    {editor_id, editor_generation, editor_revision, value, selection, composition}
committed:  {epoch, value, origin: Option<{editor_id, editor_generation, input_revision}>}
```

Widget 保存 selection/composition，从而不只是 `String`。app 的 `draft` 是 committed logical value；分离的两个 phase 不成为“两个互相未知更新的完整权威”。

- 本地 committed input 通过 origin ticket 更新 Store。回到 origin editor 时值 adapter 只 ack epoch，**不发起 ExternalReplace**。`on:input` 依然只做业务 nontracking command。
- 来自外部 set/load/undo 用 `ExternalReplace{value, origin, revision, SelectionPolicy::{PreserveClamped,SetRange,CollapseEnd}}` 与本地更新走同一 reducer。相同 epoch 只最后 committed 值赢，过期 ticket drop。
- composition 期间外部替换默认追加一个显式的「提交当前本地 composition，再替换」命令；也提供明确的 cancel composition。IME 候选激活时 Enter 由输入法/编辑器优先消费，不走表单提交；这次确认结束后的下一次普通 Enter 才触发一次新增。这里涉及本地编辑命令与输入来源 ticket，须实际 Chinese-IME trace，不靠名称猜测行为。
- `value={draft}` 只能经专属 `apply_store_revision` 进入编辑器；不得再新增普通 String binding 读取后回写 origin。local-only ack 仍保留下一次外部 revision 的 epoch/订阅 bookkeeping，不是跳过整个 binding。第一 Gate 仅做单个 editor；多个 editor / 跨 view 语义冲突留到后续验收。
- UTF‑8 bytes (Rust)、UTF‑16/native OS offsets、Unicode grapheme、Parley glyph position、AccessKit char range 是五个不同坐标集，任何选取都要经 adapter，不可裸 glyph index 当 selection。
- `set_text()` 等底层文本 editor 可能 cancel compose，故字段更新不得随意借库重调 replace。若实验证明 PlainEditorDriver 已覆盖此 contract，可以包装它而非再创能力。

## 6. StyleIR：Tailwind 词表，不是 DOM 引擎

```text
固定版本 candidate literal + typed inline
 -> UtilityAst{base,value,modifier,variants,important,span}
 -> canonical declarations {property_mask,value,conditions,origin,ordering,provenance}
 -> immutable StyleProgram
 -> condition/token epoch 仅触发该字段 StylePatch
```

### 优先级单一规则

普通作者 CSS/no-important 时，排序目标是：

```text
origin / importance（author inline 与 class rules 在此层分离）
 -> 只在 class rules 内比较 registry 的 canonical variant / utility order
 -> 只有 origin、importance、canonical variant/utility 都相同的项才比较 source order
```

不是先选最活跃条件再回头看 origin。`hover` 是否命中只能决定某条 class 声明是否 eligible，不能让普通 class 跨 origin 胜过 ordinary author inline；编译期 canonical order 也不等于把条件激活时间当排序依据。组件默认值与主题等 origin 的精确层级按固定版本固定清单。

- `!important`、layering、transition/animation state 的完整排序规则必须在固定版本 fixture 后冻结。当前不声称支持任意 inline-important/layers。已确认的 ordinary 情况是：ordinary inline 胜过同属性的普通 class/hover 状态声明。早期让 condition 跨过 origin 的候选未采纳，不能实现为另一条优先级。经典 Tailwind `aria-pressed` 变体只匹配 `"true"`，没有对应的 false 简单变体【固定 commit ac7fd400... 的官方 `variants.ts:868-880`；CSS Cascade `#cascade-sort`】。
- Property 子字段展开是完备的 mask：padding 轴、border width/color、transform parts/shadow/ring 无整表 `.styles` 覆盖。Compiler order 是 canonical utility+variant，而不是在 attrs walk 时后写 wins、更不是按 appearance 字段排在后面。
- class：只接受从 pinned Tailwind v4 registry 生成的支持清单，未知类与 X 类必须给出诊断，不能静默忽略。构建器不使用 CSS selector interpreter。inline 是 typed Rust 值/enum，不是运行时 CSS 字符串。
- `active:` 永久是 native Pressed。Todo filter 是 toggle `aria:pressed` 与标准 `aria-pressed:` 的同源派生；**取消 `class:if-*`/class:selected**，不再保留另一套条件语法。`TodoItem` 完成态用 typed color/text-decoration，不能给行 text 挂 `aria:checked`。
- 动态 class 首版只接受**编译期完整候选的 enum 或静态 class 条件表达式**，不接受任意 class 字符串。若未来启用 `ClassId` manifest，它只能引用已生成的静态 StyleProgram，不能引入未验证的 condition namespace。`class:if-selected` 是已取消的早期候选，不再作为后续 Gate 的隐含要求。
- 支持矩阵中 N = 已实现且有已验 fixture，D = 可说清差异，R = backend/平台依赖才能支持，X = 诊断拒绝。**TAILWIND coverage 现在无百分率，须官方版本 source commit 生成 manifest + 差异测试才可报。**
- 默认单位 logical CSS pixels，环境映射显式 `root rem`；`vh/vw` 是 window 逻辑 viewport；DPR 是 physical targets/text epoch，不混入布局；`px` 不是 physical pixel。

| 类别 | 优先原生支持的候选 | D/R | 首次明确拒绝 |
|---|---|---|---|
| Flex/Grid/Spacing | display、行/列、gap/padding/margin、align/sizing、position/inset/order | intrinsic/baseline/subgrid/safe align等具体差异表 | table/float等无adapter 不能装作 same geometry |
| Typography | family/weight/size/line height/color/basic decorations/letterspacing/wrap基础 | CJK/bidi/emoji/mixed font、font synthesis、intrinsic | vertical writing、任意 font 语法无 fallback policy |
| 颜色与绘制 | palette/alpha、border/radius/color/image/basic gradient/opacity/transform | mask/backdrop/multi-filter/blend、颜色空间映射须过能力门 | arbitrary property/CSS selectors、未登记残段或 no‑op mask |
| 条件 | hover/focus-visible/focus-within/disabled/checked、`aria-pressed`、theme/root-size、已命名 ancestor | container 需 accepted available-size 反查、platform preferences、retained semantic map | 任意 has/peer DOM selector、未发布的 arbitrary property |
| Theme/工具 | local `@theme` 后置、typed utility registry/source watches 本地资源 | `@apply`、`@custom-variant` 白名单且要与 canonical order 等价 | @plugin/@config/JS、HTML content::before、namespace 与能力不明的任意生成 CSS |
| Transform/Animation | typed one-field transition/bounded named keyframes | pt scheduler/DPR/hit updates/reduced motion/backend invalidation | 任意 CSS keyframe/任意 offscreen 滤过 |


其余光色空间、P3/HDR 和 color-scheme/theme 变量继承 metadata 必须与 backend 的实际 format、alpha 和 color-space 能力共同验证；无固定 token 到 physical correct pixel 不能称为高覆盖。post CSS-first 构建必须 offline、reproducible、锁定 theme，不能 proc macro 偷取上游网络下载。

## 7. 渲染、文字、平台与3D adapter责任

### 7.1 Layout / Font / Editor（E2 已有硬发现）

- `ViewId→Taffy NodeId` 内部私有 mapping，实际签名 `LayoutInput, NodeId, Option<&mut NodeContext>, &Style`，正文 leaf走`compute_leaf_layout`。
- 保存 text shape/source，分开计算 `calculate_content_widths()` 的 min/max-content 与不同宽度下的 accepted wrap；`known_dimensions` 和 `available_space` 不可合并为一个 `Option<f32>` 后冒充所有测量模式。公开资料对 mixed-bidi intrinsic 未给出充分保证的地方要保留 canary/fallback，不能默认所有输入都正确。
- **现有 PoC `measure()` 返回 LayoutOutput 后实际只把 Size传给 `compute_leaf_layout`，而 Baselines 被 leaf 外层置 NONE。** 这不是产品已能 baseline，B Gate须修：从最终 accepted text layout 算首 baseline+content top再配外 layout，已知尺寸/命中 cache 的早退路径仍用同 epoch，不得从 stale measurement 补 baseline。
- Font synthesis/embolden/skew需由 Parley `run.synthesis()` 实际导向 glifo，cache key含真实 face + weight + variation + scale。当前只有font_size/normcoords不能称支持 faux bold。
- 字体 shaping/wrap/painted cache key还含 font face/geometry accepted width / scale/engine data/version。CJK/日文 ICU4x dictionary models 必须在发布 artifact 中验证存在。
- IME、editor、AccessKit 使用不同的字符偏移坐标，必须经同一 Widget Editor 提供的 adapter 转换，不可把函数命名当作相互兼容的证据。真实输入状态统一由本地 reducer 管理，不由 ARIA attribute 反向驱动。

### 7.2 Paint / Window

- zenui-render 保有自身的 retained command pages与后端适配，不暴露 Vello Scene/Texture owner identity。`vello_cpu` 当 reference/显式 fallback；Classic只作 proof，不是 public ABI。
- `RenderCapabilities` 必须枚举 features/limits/surface formats/texture usages/alpha mode/color‑space/clip/mask/filter/atlas，未支持用 typed unsupported/error，不允许用 wgpu validation panic 或 try-panic 探测。
- Classic probe 的 straight‑alpha层 shader与 sparse‑strip premul输出分离；禁止跨版本共享默认。External `TextureBindings` 需反应 premultiplied filterable target，并带 `format, color_space, alpha, extents/scale, device_epoch, committed_serial, owner_generation`，paint page key不只是 TextureId，陈旧/重复 ID 不能调用当前 surface。
- 每次结构更迭且 layout完成发布一份 `GeometrySnapshot`: logical rect、accum transform、scroll/sticky、clip chain、z-index (paint/hit同)、DPI/epoch。hit/paint/caret/AccessKit皆从该唯一快照，不得 paint viewport外还 hit、新树还按旧 rect dispatch。结构变化时先使迟命令被拒，再令 focus与 GeometrySnapshot 同时替换。
- real window per‑window surface与DPI，resource/device epoch都重置，multi monitor不共用全局 scale。device lost清掉相关 atlas/binding/scene，rebuild成功后才 publish。GPU readback only证明当次输出，没证明 retained paint cache。
- 窗口无闲置应该 wait；非活动3D请求重绘需要需求/动画驱动，默认并不是时刻 render。

### 7.3 可选3D：与 OS 窗口不同对象

```text
合规 glTF subset worker
 -> mesh UV basic material/indices/camera + shared wgpu Device/Queue
 -> per View generation own offscreen Rgba color + own Depth32Float
 -> resolve普通 TextureView+完整metadata+completed serial
 -> 2D TextureBindings / clip / opacity / z-order
 -> present
```

绝不复用 Vello internal depth给3D，也不能同 pass 既采样又写纹理。要对该边界留多 viewport新旧纹理替换、加载completion after owner dispose、device loss等测试。CPU可以 2D fallback或明确昂贵的readback benchmark，不能默认共享纹理免费。

第一阶段分开：最少合法 glTF load→mesh with depth遮挡→对应 2D overlay相投。此前绿色图片不等于已加载模型。之后渐进PBR material/lighting/normal texture/animation，不关心何时调用 import就称全部glTF完成。

## 8. 关键分歧与主控裁决

| 分歧 | 主控采纳/部分采纳/不采纳与理由 |
|---|---|
| Copy handle 与 String | **拒绝 Copy+String 模型行；采纳 U/X 对复制与失效的质疑，保留 A2 的底层 Rc 实现自由；不要求现 PoC 使用单 u64。** 目标是小型 field handle 带 `{runtime_id,slot,generation}`，经过 checked lookup。String 在明示 mount 初值/投影时拥有一次，不能藏在 Copy 结构里无限复制或要求 String 为 Copy。 |
| Store 与 reactive graph 重否 | **保留模型+查询 adapter，不采纳再建一套整树 diff。** 字段/revision 只有一条生效路径；目标是 query 的 field-edge、membership-index 和 cache revision 接同一张图，依据实际读取动态新建/替换，A0  fixture 验证其边数和回收，不是声称现在已连线。 |
| 过滤 vs identity | **部分采纳 X1 编辑连续性，不采纳默认hide-store。** 默认dispose保业务row，UI新gen；若优化须显式有界retained/snapshot验收。焦点另行转移。 |
| Memo<Vec> 的便宜快照 | **采纳 X2 对重复物化的质疑，采纳反方对字段依赖的修正。** empty bool 独立、For 直接 Memo<RowSet>，不按消费者重复取值/物化。query字段依赖是真考核非只是 Arc。 |
| inline/class/style优先级 | **采纳 StyleIR 字段化方向，拒绝其自定义 condition 压过普通 inline 的早期排序。** 当前固定的普通 inline 与普通 class/hover 遵守 CSS origin；important/layer 仍须固定版本差分校验才能够启用。 |
| active 与应用状态 | **采纳禁用 selected 冒称 active 的建议**，取消 class:if-selected；Todo 的 aria attribute 与状态样式同源，不用独立 bool 制造伪状态。 |
| mount & error处理 | **采纳 ErrorResult mount与错误sink必要**，但改成 Rust 真实 Result<MountPlan,MountError>。不把原生 target `?` 的适用性说成现有宏已经做过测试。 |
| Tree staging 与回滚 | **采纳A1/A2完整的提交边界和事务失败放弃 journal；简化需要全量pre‑render每个frame tree clone的反对为snapshot/staging journal。** 性能未测，没有完整树复制实现的承诺。 |
| 验证阶段 | **采纳 U/E3真实窗口并早期可驱动、A3无窗口不可宣称桌面；拒绝把这些全塞同一Gate。** A0 pure Rust, A1 Windows Todo, B metrics/text/style, C 3 OS, D 3D。 |
| Backend & glTF | **采纳sparse+wgpu为候选；只用 classic 当作纹理互通证据；拒绝绿色占位冒称真模型，拒绝同旧 alpha shader 套新后端。** |

## 9. 实现验收 Gate（均未完成）

### A0 — 纯 Rust core + Todo reducer（当前第一优先）

- **范围与入口**：不使用 proc macro、winit 或 GPU。先建立最小 `crates/core` 与 `crates/app` 原型，以空 Store 创建 root Owner、draft、filter、QueryMemo 和 fake native View，在单线程 headless 场景中运行事务并输出 trace。规划的 workspace 和 crate 尚不存在。
- **身份、读取与错误**：checked field handle、typed Store row 和组件参数都须落到可编译的 Rust fixture；无需为此再造独立的源类型系统。覆盖 `(runtime_id, slot, generation)`、跨 runtime 拒绝和 slot‑reuse 世代变化。`read_once()?` 失败必须取消该次 mount：不提交部分 MountPlan，不遗留订阅、native View 或默认替代文本。普通数据读取、Memo、Binding 和事件写入通过统一 adapter 进入领域错误 sink。
- **已观察到的反例**：旧例 `Copy+String` 触发过实际 E0204；现有 flat PoC 在嵌套 batch 中实际产生过 `true` 中间值，随后才到 `false`，不满足事务原子性。独立普通 Rust 夹具仅证明了 `Result` mount body 的 `?` 和 Copy handle 所有权，不证明新运行时已经实现。
- **事务与 Memo**：嵌套 batch 在最外层只发布一次最终值；diamond、长链和动态条件切换须经过拓扑稳定，不能 glitch。可以允许拓扑算法必要的内部计算，验收发布值和每个稳定 epoch 的可观测次数，不能笼统要求所有 Memo 在任何场景只执行一次。`WriteDuringCompute`、cycle 和首个 preflight 错误须回滚 model、derived cache、原生 View journal、focus、resource ticket 与 geometry epoch；随后一个干净事务仍可提交。
- **代数与复用**：删除 row/View 后旧读写和迟到事件只能得到一个明确 `InvalidHandle`。slot 被复用时必须分配新 generation；旧 handle 永远不能读到别名对象，更不能操作另一行的 View。过滤消失只销毁 UI owner，不删除模型 row。
- **Query 计数**：按第 2 节测量 query evaluation、ID materialization、row visits、String clones 和活跃依赖边。All 的 membership revision 不因单项 completed 字段改变；Active 相关变更后每个 `(store_epoch, filter)` 只共享求值/物化一次；两个 list consumer 不重复物化。空态的 row visits 与 RowSet materialization 均为零。反复动态源切换和卸载后活跃边回到基线，且空的 subscriber bucket 不计为活跃源。
- **For 与 focus**：reorder 保留 ViewGeneration、Owner、局部编辑状态和订阅。duplicate key 在任何 native patch 前拒绝。结构 mutation 前记录 `FocusRemovalIntent`，commit 后且仅在目标仍受影响、用户未另行转移焦点时执行同 kind next→previous→可聚焦 input→filter toolbar；不抢用户后来主动设置的焦点。模型删除与 UI 结构删除必须分开。

建议的例子为 `crates/app/examples/todolist_headless.rs`：empty→insert A/B/C→toggle B→Active→reorder→delete focused→bulk clear→invalid id rollback→next insert success。输出 query/mount/materialization/edge/geometry/focus/caret/I/O traces。`cargo run -p zenui-app --example todolist_headless --locked` 是**规划命令，尚未存在或执行**。

### A1 — 单 Windows Todo Window

- 由 application 创建 `TodoStore`，通过 `zenui::run(Application)` 启动一个真实 winit window，而不是要求用户手写占位 runtime API。目标命令为 `cargo run --bin todolist --features windows-desktop --locked`，本轮未运行。
- MountPlan 只包含 descriptor、field bindings 和 children factories。`#[component]` 把普通 Props 转为真实 Rust 参数，setup 返回 `Result`；`view!` 与 typed builder 生成相同 golden IR。独立 renamed-facade、trybuild 和 compile-fail fixture 必须覆盖 `?`、错误 mount、未知 class、For 缺 key、错误 props/event payload。
- `For each={Memo<RowSet>}`、`Show`、typed listeners、`TextInput` 专属 epoch adapter 在原定 feature 范围内验证。属性值与闭包的 binding 规则、String 的 `with`、Copy bool/enum 的 `get` 和 component handle props 在两种入口中一致；不能把 inspection Node 改名为生产 runtime。
- Windows semantic dump 中 Tasks 固定为 list，子行是 listitem；filter 是三个 toggle button 组成的 group。空 Store 与筛选无匹配是两个独立提示，Store 非空时 list 容器不因当前筛选为空而消失。
- 捕获 focus intent、窗口滚动 reveal、Tab/Shift+Tab、Space、Delete click、wheel 和 clip 外不可 hit 最少建立一条可重复 trace。错误呈现和 IME 坐标转换不能在 A1 被宣称真实 AT 已通过。Windows semantics dump 和 a11y action 属间接验证，**真实 NVDA 归 Gate C**。
- 中文 IME candidate Enter 应消费确认而不 submit；紧随其后的普通 Enter 或 Add 只提交一次。`SubmitEvent` 保留 origin、command id、UI generation 和 `prevent_default` 语义。本地输入与 origin acknowledgment 不能触发外部替换。Add 的 insert 与 draft clear 只能**同时提交或同时不提交**：成功 insert=1、clear=1；任一步失败则 insert=0、clear=0，保留已提交值，并把错误送入 sink。
- 核验 filter/编辑器 Signal 写入失败同样进入 sink，统一错误显示不自动抢焦点。Windows CPU reference 与 GPU 对本 Todo 所需 source rect、clip、alpha、logical size 和 target resize 做像素对比；只支持实际用到的 utility token，并明确诊断未知输入。

### B — Layout / Text / Style 基线

- 修复 E2 确认的 baseline 丢失、intrinsic 不可用宽度假设和 Font synthesis 未转发问题。固定字体与 ICU artifact 下，对 CJK、emoji、RTL/mixed-script 正文、accepted-wrap、首 baseline/content top、known-size 早退和 stale measurement dirty 做 golden/differential 验证。
- 分别测 min/max intrinsic、known dimensions、available space、空字符串、wrap、padding、border、字体与 DPI 替换；Parley mixed-bidi 未决之处必须有明确 canary/fallback，而非默认正确。文字 cache epoch 必须覆盖 accepted width、字体/合成、geometry 和 scale。
- StyleIR 建立 pinned Tailwind commit 的 N/D/R/X manifest 和 candidate golden。普通 author inline 在 hover=false 和 true 时都胜过同字段普通 class/hover；`important`/layer 未通过固定版本差分前标未支持。active 仅表示正在按下，toggle 样式和 aria:pressed 同源，枚举/静态 candidate 的动态 class 不得成为任意运行时字符串。
- `GeometrySnapshot` 由 layout、paint、hit、caret、accessibility 共享，旧 epoch action 被拒。texture format、usage、extent、color space、alpha、device epoch 与 capabilities 在 Scene 执行前返回明确的 `Result::Unsupported`；不能用 wgpu validation panic 充当能力检测。
- 对 A0/A1/B 的固定 workload 采集真实 time/count JSON，分别记录 mount、title read、field edge、style、layout、text、retained encode、GPU substeps、live resources。一万行一次 filter/reorder/scroll 与 10000 次 mount/unmount 的 frozen workload 须附输入、环境、warmup 和样本分布。没有实测就填 N/A，不得把单像素读取或单次耗时当端到端性能上限，也不给未测帧预算命名。

### C — 三平台真实发布门

- 同一 locked Todo 在 Windows 中文 IME + NVDA、macOS VoiceOver、Linux AT-SPI client 真实执行；Linux Wayland 与 X11 分开记录。任一平台缺 runner 都是 blocked，不能由 Windows 结果推导三平台通过。
- DPI、resize、multiwindow、clipboard、drag、nested scrolling、focus reveal、keyboard hit、IME coordinates、high contrast、reduced motion 和 fallback font 使用相同场景和 Geometry epoch。AT action 先检查当前 View/generation，selection 与 accessible char ranges 经过单位适配，绝不能把 glyph/byte 索引当字符索引。
- Windows 至少验证 DX12 与可用时 Vulkan，macOS Metal，Linux Vulkan 或实际可用 backend；固定 CPU reference artifact，逐 backend 测至少一种 Unsupported path。检查 format、alpha、logical/physical 单位、device loss 和 atlas 重建，旧 device/texture generation 不能继续使用。
- 发布门为真实执行 `cargo test --workspace --locked`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo package`、独立 renamed facade、字体/ICU 资产与 licenses/lock hash 齐备。**当前根 package 无统一 workspace，不能把这些命令标为 passed。**
- Idle 在真实窗口和 `ControlFlow::Wait` 下记录 30–60 秒，静态场景不能自动每帧重绘。单字段更新不得重跑组件 root/sibling setup。长时间 churn 比较 warm-up 后活跃 owner、edges、GPU resources，回到已记录的温缓存基线，不要求 allocator RSS 严格归零。保存真实 CPU/RAM/GPU、driver、display protocol、OS、电源状态，缺性能数据标 blocked，不写零毫秒。

### D — 可选3D与性能定稿

- 第一阶段加载合法最小 glTF：buffers、UV、index、basic material、camera，确有通过 own `Depth32Float` 的 mesh 遮挡。每 UI generation 有独立 offscreen color/depth 和普通 resolved TextureView，每帧验证 format、color space、alpha、extent、device epoch、committed serial、resource generation。
- 两个 viewport 各自 resize、clip、opacity 和 2D overlay，不串用纹理或深度。device loss、reload/unload、late completion 只能以新 owner/generation 原子替换；worker 失败保留 last good model，并报告错误或 unsupported。所有 release ticket 与 live resource counts 可重放。
- 后续 PBR、normal、lighting、animation 逐项测能力和成本；不包含任意 2D/3D depth 交插、AR 或游戏引擎场景。
- 最终性能记录 GPU submit/resolve/readback、retained encode、font/layout、draw calls、live resources、idle 与 frame values。固定 benchmark 元数据与真实 baseline，未跑平台为 blocked，不套用其他机器的估值。

## 10. 交付后的准确状态

**已有**：目标 RSX 能由真实 rstml 解析；少量 typed utility 与 flat sink 更新/解绑测试；Parley CJK glyph、CPU output、Taffy 测量小探针；两种本机 headless wgpu interop。多领域评审同时指出，完整 MountPlan、生产 Owner/arena 和事务 Query 尚未实现。

**尚无**：生产 Signal/Store/Memo/Owner/事务 journal，完整 typed component/For/Show/Resource，live winit/AccessKit/IME，跨 layout/paint/hit/caret/a11y 的 GeometrySnapshot，完整 StyleIR 与 fixed-version coverage，修好的 baseline/synthesis/mixed-bidi，macOS/Linux 实机验证，真实 glTF/独立 3D depth 和生产资源生命周期/性能证据。

**结论**：沿单一 View 树与事务式细粒度更新的主干推进，先完成 A0 的最小可证明闭环，再做 A1 Windows Todo。规格是当前证据下可执行的统一目标，不是框架已成熟或跨平台兼容的声明。

---

开发过程中的候选和争论保存在 round1/2/3；本 FINAL v1 是唯一当前推荐目标。实现应让可复现的红测试通过，若发现规格与实际平台能力冲突，应记录证据并修订规格，不能为迎合实现而改写失败测试或虚报 green。
