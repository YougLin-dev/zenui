# 第三轮成稿评审

评审对象为统一规格及当时的 Todo 示例。四位评审均有不同关注面，以下意见经 FINAL.md 的正文错误边界处理与执行记录修订，不是照单全收的推荐。

> 本文件保留当时行号和缺陷，不声称当前文件仍相同。**其 P2 的“hover 激活后应胜过 inline”已被 CSS Cascade 核验明确推翻；ordinary inline 在两种 hover 状态下均胜普通 class/hover。** 最终采用规则见 [FINAL.md §6](FINAL.md)，执行证据见 [VALIDATION.md](VALIDATION.md)。

## U1+U2 第三轮成稿评审

**结论：保留需小修**

### P1 FilterButton 的 `current={filter}` 和 `value={Filter::All}` 让初学者无法判断哪种初值/字段绑定被使用：`filter` 是 Signal<Filter>，`Filter::All` 是 Copy enum，`label` 是 &'static str。Spec §3 说 `{value}` 是"mount/setup 求值一次"，但同一 component 又通过 `current.get()` 在闭包里订阅文档没有说清楚：component props 传递的是 Copy handle 还是值本身，这决定了 `current` 究竟是不是 signal handle。初学者会以为 `current={filter}` 是把 Signal 的当前值传给子组件一次。

- 证据：main.rs:94-96 `<FilterButton current={filter} value={Filter::All} label="All" />`；main.rs:139 `fn FilterButton(current: Signal<Filter>, value: Filter, label: &'static str)`；FINAL.md §3 `{value}` 仅在 mount/setup 求值一次；value 不需要 reactive。
- 必须修正：在 README.md "入门顺序" 第 2 或第 4 步补一条 3-5 行的小段：「组件参数 `{name={signal_handle}}` 当参数类型是 `Signal<T>`/`Memo<T>`/字段句柄时，宏把 handle 本身（一个 Cheap Copy）传给子组件；子组件用 `current.get()` 或 `{move || current.get()}` 才真正建立订阅。普通标量和借用 `&'static str` 走的是普通 props 一次性传值。」并在 main.rs:94 上方加一行注释 `// 这里传 Signal<Filter> 的 handle（Copy），不是它当前的值`。不一定需要改 SPEC 正文。
- 验收：读者读完能在草稿纸上解释：把 `{filter}` 改写成 `{filter.get()}` 会立刻产生 Rust 类型错误（因为参数期望 Signal<Filter>），并知道这是为什么。

### P1 `aria:pressed={selected}` 中 `selected` 是 `move || current.get() == value` 的 Fn() -> bool，与 FINAL.md §3「动态 attr 用显式 `{move || expr}`」一致；但旁边 `disabled={move || draft.with(|text| text.trim().is_empty())}` 使用的是显式 move 闭包字面量，两种写法混在同一个文件中没有点名它们是等价的（都是 "FnMut/Fn 闭包"）。初学者会以为 `selected` 是 boolean 变量而不是 closure。

- 证据：main.rs:140 `let selected = move || current.get() == value;`；main.rs:144 `aria:pressed={selected}`；main.rs:87 `disabled={move || draft.with(...)}`；README.md 没有解释 `selected` 是闭包。
- 必须修正：README.md "入门顺序" 第 2 步补一句：「`{move || expr}` 和先 `let name = move || expr; {name}` 完全等价——属性类型是 `FnMut() -> T` 的 typed binding，框架在依赖变化时重新调用它。」并在 main.rs:144 上方加 `// selected 是闭包；{selected} 等价于 {move || current.get() == value}`。
- 验收：读者能指出 `aria:pressed={true}` 和 `aria:pressed={selected}` 的差别：前者是 mount 时定值，后者是每次 `current` 变化时被重新求值的闭包。

### P1 README.md 与 FINAL.md 都把 `value={draft}` 称为「专属 Signal 属性 adapter 不是通用双向绑定」，但初学者不熟悉 adapter 这个词；同时 `on:input={move |value: String| draft.set(value)}` 就在同一行，看起来像经典双向绑定——文档没能回答「那为什么还需要 on:input？adapter 自动做了什么、不自动做什么？」 FINAL.md §4 解释了 origin/epoch，但初学者读那一段像在读引擎规格，记不住重点。

- 证据：main.rs:81-82 `value={draft}` + `on:input={move |value: String| draft.set(value)}`；README.md §15-17 长段；FINAL.md §110 `value={draft} 宏转为 apply_store_revision，非额外 String binding 通知注入`。
- 必须修正：README.md 加一段 4 行内说明（不是替换现有长段，而是在 §"输入不是 value + on:input 的简单循环" 标题下先放 TL;DR 卡片）：
「`value={draft}` 让 TextEdit 每次 draft 变化时收到新文本，**但不会把用户敲键盘的内容写回 draft**——那是 `on:input` 的工作。两者必须在 reducer 内去重（同 epoch），否则光标会跳、中文输入法会被打断。`draft` 是 String，但 adapter 不要求你写 `with(Clone::clone)`；普通 `<text>` 仍然要写 `move || draft.with(Clone::clone)`。」
在 main.rs:81 行内加 `// 显示 draft 当前值；下方 on:input 才把用户输入写回 draft`。
- 验收：读者能解释：如果把 `on:input` 删掉打字不会进入 draft；如果把 `value={draft}` 删掉外部 `draft.set(...)`（例如 Add 后清空）不会反应到控件。两个设定缺一不可

### P2 `style:color={move || if completed.get() { rgb(...) } else { rgb(...) }}` 用了 typed inline style，但通篇没有解释为什么这里不用 `class:` 或条件 class 切换。初学者会问「为什么不写 `class={if completed.get() { "text-zinc-400 line-through" } else { "text-zinc-950" }}`」——文档在 FINAL.md §5 解释了禁止 ClassString 拼接，但读者读到 main.rs 那段 inline style 时还没翻到 §5。

- 证据：main.rs:172-182 `style:color={...}` 与 `style:text-decoration-line={...}`；FINAL.md §144 禁止 ClassString 任意拼接；README.md §26 简单一句。
- 必须修正：README.md §"Todo 的语义边界" 第 4 条补一句：「completed 行的颜色走 typed inline style 而不是 class 切换，因为 Zenui 禁止 `format!("bg-{theme}")` 之类的运行时 class 拼接；`style:color={move || ...}` 接受一个返回 `Color` 的闭包，Type 由框架校验。」
- 验收：读者知道：想让完成行换颜色可以；想写成 `class={some_string_variable}` 会被编译期拒绝。

### P2 `For` 的能量对初学者不直观：`each={visible}` 是 `Memo<RowSet>`，`key={|row: &TodoRow| row.id}` 是 borrow closure，`{move |row: TodoRow| view! { ... }}` 是 consume closure。三种签名不同却让初学者无法预测什么时候是 `&T` 什么时候是 `T`。`RowSet` 在 README 被一笔带过为「共享低成本投影」，初学者会把它和 `Vec<Todo>` 混淆，并问 `visible.get()` 在哪里被调用。

- 证据：main.rs:111 `<For each={visible} key={|row: &TodoRow| row.id}>`；main.rs:112 `{move |row: TodoRow| view! {...}}`；README.md §13 仅一句。
- 必须修正：README.md "入门顺序" 第 5 步加 3-4 行：「`For` 的 `each` 参数接受一个 `Memo<RowSet>`，`RowSet` 是共享的行 id 列表（不是 `Vec<Todo>`）；框架在每个 store epoch 最多物化一次。`key` 用 `&TodoRow` 借读取得 id（所以闭包不能消耗 row）；children closure 用 `TodoRow` 拿到整行句柄。resources 与 UI generation 见 FINAL.md §1.2。」main.rs:111 上方加 `// RowSet 是共享 id 列表；children 每行接收 Copy 的 TodoRow（含 id + 字段句柄，不复制 String）`。
- 验收：读者能回答：`For` 不会因列表里别的行被勾选而重新执行 `{move |row| ...}`，只会对新增/删除/重排的项目调用（key 不动 → 不清 subscriptions）。

**保留主线**：保留当前 FINAL.md/main.rs/README.md 的总体结构与所有 A0/A1 验收；本轮只在「初学者看得懂 API 形状」这一维度补最小说明。核心做法：(1) 不把 Signal<Filter>/bool 闭包当作初值、(2) value/on:input 双轨先用 4 行卡片解释再放长规格；(3) 解释 typed inline style 取代 class 拼接的理由、For 三种闭包签名的差异。建议在 README.md 顶部新加一小节「读这份示例前请先回答」列出 5 个自测问题（对应上述 5 个发现），每问 3 行内答完，链接到 §已有详细章节——不要去改 FINAL.md 的冻结合同。不必重启 round2 debate，也不必把代码改写成更简单的形态：当前 API 形状（typed handle 传递、闭包绑定、value/on:input 双轨、typed inline style、Memo<RowSet>）是正确且必要的，缺的只是初学者视角的导读句。第六候选发现（未列入的：``No tasks yet`` 与 ``No tasks match this filter`` 的 has_rows/no_matches 区分）已经足够清楚，不需要新发现。

## S + A2 红队 / 响应式架构敢质

**结论：保留需小修**

### P1 Row query『(store_epoch,filter) 单次物化缓存』与同一段『精确字段依赖 (row,field)』没有任何 wiring，会出现两套事实权威：query cache revision 一边当唯一现实，字段订阅一边当唯一现实。结果正是 round2 主控反方在 182-186 行警告、但 FINAL 实际未接线的『行变色而 remaining/Active 不更新』。

- 证据：FINAL.md:58『字段读取在 tracked scope 动态登记访问过的 (row,field)』；FINAL.md:60『All 听 membership；Active/Done 还听当前相关行的 completed』；FINAL.md:63『Store transaction epoch 标记查询 dirty；拓扑 stabilization 之后每个 (store_epoch,filter) 在真实 list 消费者首次读取时共享计算和至多一次 ID 物化』。第 209 行 A0 green 只断言了『All membership revision 不变』『Active 一次 query/cache/RowSet 物化』，但没断言『(store_epoch,filter) 缓存条目失效其登记过的精确 row 字段边/成员关系边』。两个名称出现在相邻段但没有任何一处说明『触字段 → 标哪个 query 缓存 dirty』的边方向或所有权。
- 必须修正：在 §1.3 加一段唯一接线规则：(1) rows(filter) 在 computed-query 求值时同时建立两类边——membership 索引边 + 对每个已求值 (row,completed) 的反向字段订阅（与 Signal 边同一张图）；该依赖被任意字段命中时只 bump/标脏该 shared query cache，并不立刻重算/物化；(2) Memo<RowSet> 仅 track『query cache revision』，不 track 每行字段（避免 For 自身 N 条字段边）；(3) 空态（is_empty/has_rows）只 track 对应的 membership/count bucket，绝不在其求值中枚举 row。
- 验收：A0 加（HEADLESS）测试：(a) Active filter 下对一条非 member 行 set_completed(true) → 该 filter 的 query cache 被标脏 + 重算 + 恰一次物化；(b) All filter 下同操作 → membership revision 不变，query cache 不被该 completed 字段边命中；(c) is_empty/has_rows 两种 filter 值同批消费时 query cache 仅构建一次，RowSet 物化次数=1，String clones=0；(d) 字段订阅数随活 row 数线性而非 consumer 数平方。

### P1 『事务回滚到此前 checkpoint 不提交部分树』与 第 70-81 行阶段——Binding compute → TreeCommit → cleanup/layout/publish——之间存在结构性未定义区间：TreeCommit 不是一次原子写，而是 steps of attach/move/unlink + focus/IME/Resource cleanup，再 Taffy/Parley layout、再 publish。中途一旦出现 InvalidGeneration／DuplicateKey／panic，规格没说『是否能部分已写入 native 树』，也没说 publish 前的 GeometrySnapshot 是否当作权威被使用。

- 证据：FINAL.md:32『事务内遇失效结构提交拒绝该 patch，保留已验证树/焦点并上报领域错误，不允许半应用』；FINAL.md:76『TreeCommit：验证 key/topology、提交 fields/attach/move/unlink、focus/IME/capture 策略』；FINAL.md:77『cleanup disposed owner/bindings/resource tickets（无同步 flush/自 mount restriction自销毁）』；FINAL.md:86『事务遇到第一个错误回滚本事务用户写入与派生状态到此前已提交稳定 checkpoint... 此原则要由 write journal/revision、分离 runtime scratch state 实现』。但 write journal 在 §1.1/§2 两处都只覆盖用户写入与派生状态，未明确覆盖 native View 树本身的附加/移动/解脱以及 Taffy NodeId 的私有映射。
- 必须修正：把 §2 的阶段图与 §1.1 一起补一条：『TreeCommit 必须全部 patch（topo、style、resource tickets、Taffy adopt/unlink、widget-state queue）先在 shadow/staging 提交缓冲区完成领域预演（full preflight），通过后才一次性 swap 进 authoritative View tree；任何阶段失败即丢弃 staging，native 树仍是上一次 commit 的 bit-for-bit 镜像。』并明确：commit 之后到 publish 之间禁止再发生领域错误（layout/paint 错必须走 capability Result，与事务回滚无关）。
- 验收：A0 fixture：(a) 构造一次包含『合法 attach + 中途 InvalidGeneration』的事务，断言 native View tree、ViewId→Taffy NodeId 映射、focus pointer、GeometrySnapshot epoch 与事务前完全一致；(b) ID 复用 slot 的 stale patch 不会污染新 ViewId；(c) 失败事务后立刻发起一笔全新生产事务能正常 commit 并 publish，phase guard 已复位。

### P1 示例 `FilterButton current={filter}` 把父作用域的 Signal<Filter> 直接传给子组件，但 §1.1 / §3 只允许『动态 Props 是已声明的 Signal/Memo/Binding/Projection handle』+『typed binding wrapper/handle』——既未规定 Signal<T> 可以直接作 prop 表达式(不需 move ||... 包裹)、也未规定 PascalCase 组件属性 vs intrinsic 属性的语法分化。结果 FilterButton 的 props 解析路径写在示例里，却没写在规格里；按现有 §3 字面还可能是错的（父作用域 `filter` 是 `Signal<Filter>` 不是 closure，子组件 prop 类型是 `Signal<Filter>`，需要宏为其生成 SourceProjection，但没规定什么时候这个动作发生）。

- 证据：examples/todolist/main.rs:94 `<FilterButton current={filter} value={Filter::All} label="All" />`；main.rs:139 `fn FilterButton(current: Signal<Filter>, value: Filter, label: &'static str) -> impl View`；FINAL.md:45『动态 Props 已是已声明的 Signal/Memo/Binding/Projection handle，不是让它附于不同 Owner 再随意求值的裸函数』；FINAL.md:91-92『Props 各自有 Rust 类型... 动态 prop 在父级位置是 typed binding wrapper/handle』；FINAL.md:92『宏只处理小写 intrinsic/ PascalCase component/特殊 For/Show/Index、属性与 class；closures/block/generics 交给 syn Rust』。但没有任何行说：PascalCase 组件位置的 `{ident}`（裸 handle，非 closure）何时是合法的 typed binding wrapper。
- 必须修正：§3 明确一行：『PascalCase 组件位置上的 `{ident_or_field_access}`，当其类型本身就是 Signal/Memo/field handle/Store projection 时，被宏直接透传给 prop；不需要 move ||... 包裹。属性类型不是 reactive handle 时仍按普通 {expr} 一次性求值』。同时把 todolist README『入门顺序』末尾补一句同样的规则。
- 验收：A1 trybuild fixture：(a) `<FilterButton current={filter} .../>` 在 filter: Signal<Filter> 时编译并 prop 类型确是 Signal<Filter>；(b) 写 `current={move || filter.get()}` 在 prop 类型为 Filter 时同样合法；(c) 把 Signal<Filter> 传给声明为 Filter 的 prop 时报错位置在属性上，并给出『期望 reactive handle 还是一次性值』的 help。

### P2 动态 Style 优先级在第 141、142、197 行用了三种措辞，存在重新引入 round2 D4/D8 字面冲突的风险（一位实现者读到「inline 是消费者明确覆盖 source」会以为 inline 永远胜过 active variant，违反 D4 的 condition 主导裁决）。

- 证据：FINAL.md:141『origin order：framework/widget default < ... < dynamic named class program < inline typed style。CSS important 只做尾部 origin/inversion rule』；FINAL.md:142『class 内部仍以 fixed Tailwind v4 编译参考确定 utility/variant canonical order，不使用用户自创 active>hover>selected 特定级别。条件的激活不改变 CSS rule total order，普通 inline 是所有非 important 消费者明确覆盖 source』；FINAL.md:197『inline author declaration plain 胜 ordinary class/hover variants；class internal 按 fixed Tailwind canonical variant order+utility rank，important 最后规则』。这三种说法指向同一件事但读者无法只看字面对『hover 激活时 inline 还胜吗』得出唯一答案。注意 round2-debate.md:207-209 仍是显式冲突裁决——FINAL 只在第 197 行收束。
- 必须修正：用一段排序键 tuple 替换 §5「origin order」段：『ConflictKey=(important_class, origin_rank, variant_rank, utility_rank, source_order)；variant_rank 对 plain class 与 inline 都为 0；active/focus-visible/checked/aria-pressed 永远 > 0。两声明逐字段比较，逐层取胜』。并删除『普通 inline 是消费者明确覆盖 source』这类误导描述——它只在 variant_rank 相等时为真。
- 验收：**本节早期候选已废止，不能作为 golden。** 当前 B fixture 是 `(bg-blue-500 inline&color=red)` 与 `(bg-blue-500 hover:bg-green-500 inline&color=red)` 的普通声明比较，在 hover=false 与 hover=true 时都取 red。其余 important/layer 规则仍未通过固定版本差分，不能启用。完整最终规则见 FINAL §6。

### P2 round2 主控反方在 186 行明确提出空态必须『zero row visits/zero materialization』，但 FINAL §1.3 ≤62 行 + §8 A0 验收只写了『empty 不必物化 N 行/text clone』，『零 row visits』没进入验收；实现里若 is_empty(filter) 对 membership 集合做一遍 collect 仍能通过现有 A0 验收。这是把之前质疑的『只是用名称替代成本』风险留在验收空白。

- 证据：FINAL.md:61『空态不能需要通过 N 个 Row 拷贝来判断 empty』；FINAL.md:63『empty 由布尔查询单独派生，不得仅为了 empty 再取出整份 Vec 或 materialize』；FINAL.md:209『empty 不必物化 N 行/text clone』。对比 round2-debate.md:192『空 store 和空 filter 各只算 bool 且 zero row visits/zero materialization』——FINAL 比以前略松；FINAL.md:64 只允许 For 接 Memo<RowSet>，但没规定 is_empty 的实现路径不能内部迭代 membership。
- 必须修正：在 A0 green 一条里把 haven 『empty 不必物化 N 行/text clone』扩成『空态查询的 row-visits=0、String clones=0、RowSet 物化次数=0；由独立 counter/membership-bitset 维护』。
- 验收：A0 fixture：构造 1000 行 store + Active filter，断言 `has_rows`/`is_empty(Active)` 求值期间：(a) visits 计数器零增长；(b) RowSet 物化计数=0；(c) 仅访问一个 O(1) 派生计数；(d) `filter.set(Done)` 切换空态查询时再次断言零 visits。

**保留主线**：保留单一原生保留树 + 单一细粒度响应式图、一次性 MountPlan 与 Rust 风格 HTML 外形宏这一条主线，无替代路线建议。需要立刻修的：把 query cache 的精确字段依赖边接线写死（finding 1）+ TreeCommit 加上 staging/swap 一刀切（finding 2）+ 子组件位置的 handle prop 透传规则（finding 3）。其余两条是文档化精度问题，可一起小修。

不改的 decision：(1) 默认 For 退出 filter 时 dispose UI owner 而非 retained-visibility — 第 54 行已经清楚，且第 125 行 focus transition 兜底；(2) 主动放弃 class:if-*/class:selected 等自创条件语法、走 aria 同源（§5 + §7）— 与 round2 D8 裁决一致；(3) 单 editor + origin ticket epoch 排斥本地回灌（§4）— 多 editor / 跨 ViewEditor 并发明确留到后续 gate；(4) 生产 GPU 仍是 sparse-strips vello_gpu + wgpu 30、Classic 仅 probe — 候选而非已支持；(5) Gate 拆 A0 无宏核心 / A1 真实窗口 / B 渲染文字样式 / C 三平台 / D 3D — round2 主控反方已裁决，不应重新合并。

整体判断：当前草案不是把复杂度『藏』进 Arena/RowSet/事务 journal — finding 1、2 是把契约写到合理的半途卡住了而不是藏了两权威，要解决只需要补两段接线规则，不需要重新选型。

## 第三轮成稿评审：E3 + A3 + U3（真实桌面与可访问性）

**结论：需要重要修改**

### P1 TextEdit 双轨约定需与示例的 on:input 对齐，并冻结原生 submit 的单次提交路径。

- 证据：docs/review/FINAL.md:110-112、218 和 examples/todolist/main.rs:70-87 的目标输入适配器禁止额外 String binding；FINAL.md:96 仅写“角色+命令 submit”；main.rs:82 另有手写 on:input。若保留该回调，尚未承诺孤立的界面适配器不能误解为普通字符串回灌；submit 也未明确单步 activation 与 native command 的路由。
- 必须修正：最小保留 value={draft}+on:input：在 FINAL §4/A1 与 README 明确 value 只负责 apply_store_revision，on:input 只负责普通非追踪业务写；其 Signal 写回到自己的 editor 仅 ack，不能触发 value binding 或 ExternalReplace。提交限定为一种组合：button activation 或编辑器的 normal Enter 转为同一 typed Form SubmitCommand；IME candidate Enter 在前一步被消费，empty/disabled 根本不发 submit。补 SubmitEvent{origin, command_id/generation, default_prevented}（名称可等价），prevent_default 仅取消默认命令，不由 handler 调用默认 add。
- 验收：候选确认后：日志仅一条 input/commit；草稿清空一次，insert 恰好一次、focus 不变；随后普通 Enter 再产生一条 submit/insert；两条必经输入与鼠标 Add 各 add 一次。外部 set、删除当前 editor、复位草稿在 A0 reducer、中英文候选确认与 Button activation 在 A1 同一 trace 断言；缺 Windows runner 只标 A1 blocked，不得把 headless green 算桌面通过。

### P1 任务列表的父角色与项目根角色不成一致语义树，示例不能作为 A1 结构验收基准。

- 证据：FINAL.md:127 的“role=list/listitem/group”确为未定 mapping，A1:217 要 General semantic tree/action/cursor/hit box 同 View+geometry；README:27 称列表边界固定，但目标代码 main.rs:93 用 group，105 用 group，161 用 listitem，没有 list 容器，三种 group 会混淆 filter 和 tasks。
- 必须修正：在本 Todo 固定将 `<view aria:label="Tasks">` 直接给 `role="list"`、每行 `role="listitem"`，且 For 空成员时该 list 仍存在；filter group 不改。若 AccessKit 的平台映射用 Group 近似，必须单列逐平台差异，而不是在 main 提前写 slash 模糊角色。没有要求改三个 toggle 为 tab/listbox，保留 aria:pressed 单一来源设计。
- 验收：A1 semantic dump 对一组 TodoItem 断言稳定的 Tasks 容器角色；其后代 checkbox/title/delete 关系及三个 filter toggle 状态正确。特定空向量：tasks=[]、filter=All 显示 No tasks yet；tasks=[done]、filter=Active 显示 No tasks match，不消失容器，没有 row rebuild 或两个消息同时出现；For 不产生 listitem；filter/勾选后 active descendant 不指向 removed node；Gate C 真 AT 整理角色不无脑使用 ARIA 字符串。

### P1 focus/IME/row rebuild 主路径已定义，但 A1 缺少 reorder 保持与焦点转移后 reveal 的显式验收，避免只验收 dispose。

- 证据：FINAL.md:51-54、118-125、193 的 dispose 默认与明确 focus transfer 是合理裁决，并重排留 ViewId；A0:209 实际只要求 Active 一次查询与 focus 转移，A1:219 只命名重排、过滤、销毁，未列同 owner 重排的稳定性断言；C:233 的 scroll reveal/clip 仍未说明它与 focus/GeometrySnapshot 的先后与同 epoch。
- 必须修正：A1 给 For 验收加一条 same-owner reorder：filter 改变或 key 重排不销毁未移除行，保留 ViewId/Generation/Owner/subscriptions/caret，UnmountCount=0；焦点 successor 只在 intent 有效且被 focus 控件消失或禁用后执行。定义只有在 successor 不在 effective visible viewport 时 post-commit scroll reveal；于 layout 后新 Snapshot 再裁 hit、a11y、caret；A1 可断言 snapshot/reducer，Gate C 真滚屏验证。
- 验收：非空 query 重排的零字段、metadata/caret/owner 断言同 geometry epoch，不只测增删。focus 的同一 todo/同一 kind 重挂若 owner 保留只 transfer；snapshot 是明确单测的策略，不增加 Todo 默认草稿恢复。Future out-of-range case 从 origin post-mutation 找同 kind next、prev、Add、toolbar。Clear 同 batch 仅一次；焦点转移触发 scroll 后，Paint/hit/a11y/caret 引用同一个新 frame，无原始 rect 保留。

### P1 应用可见错误承诺没有落地目标；aria/空态正确不等于错误已可感知。

- 证据：main.rs:74、124、166、190 以及 README:30 都把命令错误转为用户可见 Result/report_error；但 report_error 没有目标作用域、fallback destination、错误语义或验收。FINAL.md:127 只规定 Announcement 是 semantic event+adapter，A1 只 semantic log，Gate C 才能宣布可听，这部分边界正确，不能靠 aria 扩写代替。
- 必须修正：保留单一 `report_error(Error)`，最小定义其路由到 TodoApp 拥有的 last-error signal，添加位于任务表单之后、任务列表之前的普通文本错误区（或等价 helper 提供同样内容）；包含稳定 happened message 和保留输入，不自动聚焦错误区、不重置草稿。错误写入走同一事务 rollback/diagnostic 规则；只有 A1 能验语义节点/日志，Gate C 才验 AT announcement。若错误指 last action 可写 action + message，不需加 Error DSL。
- 验收：示例手动在最后一个任务被 remove/clear 时产生并显示可判断 empty 模型, no error；人为 set_completed 一个不存活 id 的负例则显示错误且旧合法模型视图保留。失败原 focused control 若被事务移除仍走统一 successor，不把 focus 移动给错误区域；A0/A1 检查新语义错误节点与 hit bounds 同 GeometrySnapshot。Gate C 用真实 NVDA/VoiceOver/AT-SPI 听/观察断言，尚无公告不标 available。

### P2 A1 的真实窗口范围缺最小键盘与裁剪可见域验收；viewport 多义，易让窗口证据和 3D 证据混淆。

- 证据：FINAL.md:172 有内含 clip/scroll/z-order/scale/epoch 的 GeometrySnapshot，A1:217 要求 hit box 同 geometry，C:232-236 才覆盖 DPI/resize/multiwindow/commands/keyboard/scroll；round2-debate.md:76-77 曾建议初窗 Tab、wheel，成稿 A1 没保留这两项，且没有说明 viewport 是 offscreen 3D 资源还是窗口可见区域。
- 必须修正：不改路线图顺序，A1 补显式但未扩大至整个 Gate C 的 Windows 小窗验收：Tab/Shift+Tab 当前任务行与 filter 路径，Space 切换、删除 click/focus command；本地鼠标 wheel/scrollbar 任选声明的路径，超出窗口/clip 的 Todo 不可命中或成为 accessibility focus target；focus transfer 后 reveal 按上一条统一 snapshot。文件中将 §2 的 external viewport 改成“可选3D外部纹理视口（非窗口 viewport）”；Todo 窗口明确不是已存在。
- 验收：真实 Windows 窗口中 Tab/Shift-Tab、checkbox Space、Delete click、filter toggle、Add 与 Ctrl/Cmd 滚轮之外的普通滚动既定路径可通过，没有死焦。缩窗产生一个暴露 region 的测试，焦点 moved 或被点击后可见子集排除 clip 外项目。Output 标注 viewport 与 3D viewport 是不同对象；Failing OS window path 不发布该 OS 桌面能力。

**保留主线**：主线不改：A0 纯 Rust reducer/core -> A1 单 Windows Todo 真窗、单编辑器、少量白名单 macro/typed builder -> B 基线 -> C 三平台实机发布，是可验收次序；这里不需要重新加 gate 或改成三 OS 先行。保留 Form typed SubmitEvent/checkbox/aria:pressed 同源、两个独立空态、默认过滤 dispose 而 reorder 不变 owner、FocusRemovalIntent next->prev->Add(可聚焦)->toolbar，以及 IME Enter 优先于 submit；保留把多 editor、retained snapshot、全量 ARIA CSS 类名和平台 AT 公告延后，不声称已有任何窗口。已做得好：aria 明确只用作语义映射，实际可访问性以语义 action、焦点、selection/caret、同 GeometrySnapshot 的 hit/clip、键盘操作与 Gate C 真读屏证明，不得只写 aria。需最小补的是：目标源码不能仍让 on:input 承担看似字符串回灌闭环；统一任务 list/listitem；给 report_error 一个可见目标；给 A1 的 reorder、Tab/clip/scroll reveal 和 IME trace 立即可验收执行记录；真正 3D offscreen viewport 不等于已有真实 OS viewport。若修改超六条，优先前四条和第五条，不另开方向。

## E1/X2：Rust 宏、类型系统与人体工学（第三轮成稿，只读）

**结论：需要重要修改**

### P1 Store 的 Result 只在 command 路径可观测，tracked 读取和一次性 title 投影的错误通道没有冻结；实现者容易以裸 read_once 默认值或 binding 内 panic 补上缺口，违背已选合同。

- 证据：/E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/todolist/main.rs:158 的 `store.title(id).read_once()` 未给出返回类型；FINAL.md:32 要求 lookup `Result<_,InvalidHandle>`，FINAL.md:86 又要求错误可诊断。Todo 只给 Store 写命令和 report_error 接线，未定义 title/memo/binding 的读失败怎样从计算闭包送达事务错误及 UI。
- 必须修正：保持普通 UI 数据闭包返回字段值，新增统一的绑定/Memo 事务错误 sink（或内部 `BindingResult<T>` lowering）和 report_error 到该 sink 的连接合同；为 `read_once` 固定 `Result<OwnedTitle, InvalidHandle>`（实测便于 builder 一次移动的 owned 类型即可），由 mount adapter 提取并路由错误。不能把 Result 塞进文本。明确 stale row/title/checked 访问在 mount 失敗不会建节点，在已提交视图上仅回滚该事务，不产生 patch。
- 验收：FINAL 明确 binding adapter 的结果与事务领域错误接收器，且给出的等价 read_once 可观测签名包含 Result；A0 注入直接删除行后投影及 stale owner binding，均只产生一条带 row/slot/generation 的 InvalidHandle、无默认值/旧值 patch，事务按既有规则回滚且 checkpoint 保持，scope 后续可继续新事务；A1 Todo 让该错误进入现有 report_error/错误展示路径，无须应用逐处写 phase 逻辑。补一个 model row 尚存但 UI 已销毁对照用例，区分 View 与 Row 失效。

### P1 示例把草稿 Signal 排除在 Result 错误合同之外，处得出的误导比纯文档措辞更直接影响 TextEdit committed value 与 Add 成功反馈。

- 证据：/E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/todolist/main.rs:12 只说 insert/remove/clear_completed/set_completed 返回 Result；:56 在 `map(|_| draft.set(String::new()))` 中丢掉返回值，:82 的 `draft.set(value)` 也当语句放弃结果。FINAL.md:32 同样规定 SignalId lookup Result。
- 必须修正：最少在头部备注草稿 Signal 的 set/get/with 也走同一 InvalidHandle 和错误 sink，规定事件 handler 可让适配器路由 `FnMut(_) -> Result<(),...>` 的返回值，避免每处添加噪声闭包。也可以显式用 `if let Err(error)=draft.set(...) { report_error(error); }`，但不允许保留当前 silent no-op 外观。insert 成功后的 draft 清空失败需具体界定为回滚/保持 committed value/提示何者，不要重新发明 I/O 回滚。
- 验收：示例顶层头部列出的每个 Store 写入均与代码一致，且 Signal 草稿的 input/清空失败也进入统一错误路径。A0/A1 断掉 draft owner 后调用 input 或成功 insert 后的清空必须得到可诊断错误；失败清空不得伪装为 committed 空值，draft committed origin/epoch、c总额与事务 rollback 相适应。至少有一次事件运行中失效的回归，不能只静态检查签名。

### P2 README 仍混入已撤销的 Store 形态讨论，并把 title 的一次性 read 误述为行创建时，而不是 UI row mount 时。

- 证据：/E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/todolist/README.md:5 突然讨论并否定 `RwStore<Todos>`，:12 则说标题在“行创建时”读取；main.rs:10 与 :156-158、FINAL.md:191 均是在 UI 行 mount 时用 `store.title(id).read_once()`，row model 创建时没有该调用。
- 必须修正：只改 README 两句：应用自己创建并保有 TodoStore，row 通过其 key/field projection 得到，不引入 RwStore 命名；“行创建时读取”改“每个 UI 行 mount 时读取一次，筛选返场是新 UI mount 会重读不变 title”。不要在示例实现 RwStore 或把 title 塞回 Copy row。
- 验收：对外展示的同一摘要不再包含 RwStore/TodoStore 冲突或行创建时读取的描述；文档中若提 Store 形态只保留一个 Todo 的应用层签名。静态检查能从 README 与 main 同步确定 title 仅在每 UI row mount 读取一次；For reorder 不读取、筛选返场为新 mount 可读取。

### P2 值/属性的类型化分流是必要且已经足够，但“哪些属性直接吃静态 handle/函数体，哪些由宏装 BindingDescriptor/专属 adapter”尚未落到可逐位置核对的合同；实现时可能把 TextEdit 的专用路径误推广为一切 String props。

- 证据：/E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/todolist/main.rs 同时使用 :32 `TodoApp(todos: TodoStore)` 静态 handle、:63-82 `value={draft}` 专属 adapter、:111-114 `For each={visible} ... <TodoItem todo={row} store={todos}>`；FINAL.md:90-95 只以文字区分这些情形，宏现有 lower_element/lower_block 只对 inspection Node 输出。
- 必须修正：保留现有写法，不增加极端 DSL。只补最小分类合同：普通 `{expr}` = mount 初始值/props；显式 `move ||` = typed binding；`value={draft}` 只以 TextEdit widget Signal 属性 trait/adapter 提交 committed epoch；For 只收 Memo<RowSet>。组件 PascalCase 的 current/filter/row/store 只是普通实参 props，不按值推导响应边。静态文本与 owned String 可直接移入一次 descriptor；不得推导全类型 Into 并隐藏 clone。
- 验收：FINAL/RADME 或 A0/A1 条目列出至少有 String Signal with 读、String owned title+Signal 属性适配器、copy 标量绑定、Signal/Counter typed prop、Filter typed prop、Checkbox bool/TextInput String 属性、Store 字段和 Memo<RowSet>等几种反例所对应的实际类型降级测试。宏不要求动态闭包通用转换成 Store/Transaction；TextEdit 只有 draft committed epoch adapter。A1 宏与 builder 输出相同 descriptor/golden，并且 RowSet 不作为 Copy 值读。

### P1 MountPlan 的目标语义必须与现有 PoC 静态 Node inspection record 明确切分，否则把 `Node` 改名就可能让框架退化回可独立求值/可嵌套提交的第二棵树。

- 证据：/E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/macro_poc/runtime/src/lib.rs:98-111 的 `Content::{Static,Binding}` 与 `Node` 含真实 children/String/事件，:105-140 已是可下发 fire/evaluate 的 mutable record；macro/src/lib.rs:100-116 直接 `.child` 递归构造，:180-195 生成 `Content::Binding(Box<closure>)`。FINAL.md:35-47 要求 production 一次性 descriptor plan；这不是一个把类型改名的充分证据。
- 必须修正：在 FINAL 的 MountPlan 段加一段明确边界：现有 macro_poc::Node 仅是 inspection record，不是 native View，也不会演变成生产权威。生产 plan 用 `NodeDescriptor { kind, initial fields, binding slots, child descriptors, listeners }`，只有 runtime 在单一事务的 mount/commit 阶段构造真实 View/ViewGeneration；BindingDescriptor 与 slot 关联、本地不会运行或触发 commit，ChildrenFactory 一次消费产出 plan，For/Show 只能提交 typed structure patch。
- 验收：生产 MountPlan/IR 再不直接使用 inspection Node/Content 类型，明确只有 descriptor 和 children factory 回给 framework runtime 去创建 native record。A1 golden 检查 `move ||` 被记录为注册而非执行、view 字段不盖 whole styles；component/For/Show 的 ownersubscription 生成与释放一致，且 native ViewId/Generation 仅发布后存在。原 PoC 宏测试继续只声称 parser/静态记录验证，不声称 MountPlan 已实现。

### P2 成稿把证据入口指向不存在的 VALIDATION.md，影响目标/已验事实的区分，但不要虚构已跑测试结果。

- 证据：/E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/review/FINAL.md:3 与 examples/todolist/README.md:3 都指向 `docs/review/VALIDATION.md`；目录下实际仅有 FINAL.md、round1-experts.md、round2-debate.md、text-layout-review.md。读者无法沿成稿唯一入口查证据和流程例外。
- 必须修正：补实际仓储内证据文档的最小链接，或将引用指向真实施力范围的一节；若确实没有该文件，应去掉引用并直接列现有 PoC 的只读范围与限制。维持首页和 Todo 头“目标语法、当前不可编译”声明。
- 验收：FINAL.md:3 的 VALIDATION 路径与仓库实际证据文档一致，README 的同一引用也一致。无需为修复文档缺链执行编译或启动 workflow。

**保留主线**：保留单一原生保留树+细粒度图+一次性 MountPlan，采用显式一次值/动态 binding/专属 TextEdit adapter 三条已有路径。A0 优先统一 checked handle、事务错误 sink、rollback 和 Store 派生物化计数；A1 让宏与 typed builder 生成相同 descriptor，并演示真实 TextEdit/For/Show。保留 Copy handle、owned title、RowSet 共享投影和默认筛选 dispose；不要为消除示例闭包而引入全局 Rc 克隆、VDOM 或极端 DSL。
