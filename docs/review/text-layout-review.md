# E2 独立补审：图形、布局与文本

首轮对应调用未收尾，补审成功返回；内容和生成时间晚于首轮报告，不重新计为同一评审。

## 核查证据

- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\docs\research\runtime-and-rendering.md`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\docs\research\native-macro.md`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\todolist\main.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\todolist\README.md`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\macro_poc\macro\src\lib.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\macro_poc\runtime\src\lib.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\desktop_poc\src\lib\project_signals.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\native_poc\src\layout.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\docs\research\validation-and-roadmap.md`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\docs\research\tailwind-coverage.md`

## 具体发现

### P1 过滤不是普通 reorder：For 的 UI scope、持久模型与焦点恢复必须分层

- 证据：【设计目标，尚未有结构 runtime】`examples/todolist/main.rs:47,112-125` 两个 binding 读取 `visible`，并以其 key=TodoId 构造行；`examples/todolist/README.md:9-12` 已承认过滤会释放 UI scope、返场可能重建、持久焦点尚需策略、共享求值待验证。目标生命周期约束还分散在 `docs/research/native-macro.md:97-99`。现有 `examples/desktop_poc/src/lib/project_signals.rs:58-65,86-90,117-125,183-196` 只有一个平坦 Graph/Owner，没有嵌套 owner 或结构观察者；宏探针仅构造静态 children（`examples/macro_poc/macro/src/lib.rs:100-116`），所以并非一个遗漏测试的已实现 For。
- 失败场景：Todo A 的编辑器获得焦点且 IME 正在组合，用户切换到 Done。若实现把 For 简化成 hidden，hidden 行仍在树中、订阅仍存活；若普通重建用新 ID，Store 的 completed 可能保留但 caret/IME 局部状态丢；若框架缓存旧 ID 后重挂，删除行的 queued click/AccessKit action 会误作用新行。切换 All 时 Show 与 For 若分别求值未固化的 Store 投影，也可短暂看到 ‘No matching tasks’ 与非空列表并存。
- 建议：保留单一树 + 一图，补一张明确的生命周期矩阵：Show/For 删除是移除子树、dispose 子 Owner、退订、废弃 generation；For reorder 只移动现有节点，保留 Owner/ViewId；visibility 是同一节点的呈现补丁，不卸载。为展示型过滤定义默认焦点转移及可选择的按 `(StoreId,TodoId)` snapshot/restore，不靠隐式保留整行 Owner。`visible` 应是框架订户共享、同一 commit 内冻读一次的 Memo；For/Show 都是结构/字段 sink，而非组件重跑。定义 Tree transaction：先 validate key topo → attach/move new/remaining nodes → unlink old nodes → dispose old owner/resources；Store row model 与 UI subtree owner 分域。
- 验收：Red：沿用全局 Owner、For 每次重建或把过滤等价为 visibility，分别触发断言；有两个以不同顺序读取 `visible` 的 binding 时，旧实现也只能得到两次查询。Green：All 中三行各 setup 一次、Owner 增删时对应 `on_cleanup` 各一次；Done→Active→Done 重挂后 `ViewId`、`ViewGen` 都改变，已失效回调无法命中旧节点，而 Store row model/handle 按独立契约保留；默认焦点策略落到已声明幸存者，若有 opt-in snapshot 则按显式 key 恢复局部状态。重复 key 在 patch 前 fail；`visible` 一次 Store change 只求值一次，Show 与 For 都收到同一 coherent snapshot，原行 Text binding 可独立更新。

### P1 必须固定组件 run-once 的 MountPlan，而非让目标语法自然长成全树 builder

- 证据：【目标与真实探针的差距】`docs/research/native-macro.md:46-65,127-145` 定义数据位置 `{move || expr}` 为同步 binding、run-once 组件返回 builder，但 `examples/todolist/main.rs:122-124` 却把 `<TodoItem .../>` 包在 `move |todo| view!` 行 factory 中，尚无 props 何时求值的正式边界。实际宏 `examples/macro_poc/macro/src/lib.rs:44-51` 仅接受 view/text/button，`:84-118` 预先生成嵌套 `Node::child` 调用，`:121-201` 只支持文本块；真实 runtime `examples/macro_poc/runtime/src/lib.rs:98-110,134-136` 的 Binding 闭包不带 Owner/ViewId、无 node commit。`docs/research/validation-and-roadmap.md:10` 也明示 PascalCase components/show/for 未实现。
- 失败场景：若把返回 `impl View` 直译为递归 Node builder，组件内文本闭包在真正响应式注册前执行：初读没有 tracker，后来 signal.set 不会更新。若每次绑定重跑整组件，点一行 checkbox 会重建该行、重发本不该发生的 setup effect，并把已捕获 Store handle 变成第二套持有/生命周期来源；若 children 是 eager View 而非明确 factory，Show 尚未决定时也创建子树。
- 建议：把组件规范为纯 setup：`Component::mount(Props, MountCx) -> MountPlan`，在当前父 Owner 下新建 component Owner；只调用一次 Rust 函数。MountPlan 包含静态 `NodeDescriptor`、`Vec<BindingDescriptor>` 和 children factories，不返回另一棵 VDOM。绑定 evaluate 只产 typed FieldValue/StructureCommand，由 runtime commit；Props 中动态表达式必须已是声明的 Signal/Memo/BindingHandle，不能偷传裸 closure 再在组件内不同上下文求值。event 在 command phase 非 tracking 执行；deferred effect 不可承担同帧 binding。现有小宏可以作为 parser 保留，但其静态 Node builder 不是最终 runtime 形态。
- 验收：Red：任一 field sink 重跑 TodoApp/TodoItem、递归下载组件 props 内嵌文本、或 children factory 在每次 field patch 重执行，测试即失败。Green：对 TodoApp、TodoItem、header remaining、TodoItem 的 color/decoration binding 分别计数：点击 checkbox 后两个 component setup 仍各为 1、两行重建数为 0、仅对应行的字段 sink 增加 1；Memo 读取同步；事件 handler 不生成订阅。无宏的 typed builder 与宏展开产生同一 mount plan，并通过父/子 Owner dispose 测试。

### P1 谁改树、何时改树需要从叙述升级为唯一可检查的提交边界

- 证据：【部分有源码验证，但缺生产所有权】`docs/research/runtime-and-rendering.md:25,32-35,54-68` 描述 commit TreeMut、非 tracked commit 和 phase 限制；真实 probe `examples/desktop_poc/src/lib/project_signals.rs:10-14` patch 是裸 `node:u32`，`:117-125` bind 执行即时 flush，`:128-160` 同步 drain 整个 pending queue，`:171-180` disposal 置整个 graph disposed，而非卸载一棵子树。尚无 mutable Tree、phase token 或 patch generation。`docs/research/runtime-and-rendering.md:40-44` 也说明尚需评估 reactive_graph，不能把候选库当已接入。
- 失败场景：删除按钮 handler 若同步 Store.remove → flush → remove_subtree，当前 handler、owner 和底层 View 正被借用；递归 flush/tree mutation 可造成 reentrant panic 或使用后删除。更隐蔽的是让 `visible` 或任意 RenderBinding 在 flush 中写树并再触发全局 flush：旧 pending patch 可能在节点重挂后被应用。弱 owner 能解引用环，但不能给已删除 ViewId 提供代际安全。
- 建议：写成一条不变量：用户事件/effect 只写 Signal；RenderBinding 是同步纯求值 typed patch 的 computation；只有主线程 UI TreeCommit 以 phase token 应用 `TextPatch/StylePatch/StructurePatch`。命令边界批量 signal writes，再 flush graph，随后单一 structure transaction；不在 binding/Owner Drop 中立即 flush 或递归 mount/dispose，而排队嵌套事务。捕获资源由各自 owner 释放，事件路由和画布状态持弱 ID，不把 Store handle 的强持有当 UI liveness。分开定义 PO C 的 minimal reactive model 与真正 scoped Owner/generation arena，并明确后续是否复用 reactive_graph。
- 验收：Red：sink 直接返回任意 Node/子树 Vec、绑定内直接调用树 mutator、layout measure 中 signal.set/TreeMut，或接受 `node:1` 类裸整数节点，测试必须失败。Green：A 的 completed 更新后只收到该行 color/decoration patch，TodoApp setup 与子组件 mount 不重跑，patch 仅以 `(ViewId,ViewGen)` 命中；screen reader action、pointer click、post-frame effect 分别在 command/effect phase 运行并只能排队下一事务。clone Store handle 可留活，但 10^4 行 remove 后 sink、树节点、绑定资源、强引用计数回基线；显式嵌套 sink commit 有稳定 topo 顺序，write-during-measure 报领域错误而非 RefCell 文案。

### P2 焦点/命中/AccessKit 要共同消费 committed geometry，而非只共享一个 ViewId 名词

- 证据：【设计意图，无窗口/A11y 实现】`docs/research/runtime-and-rendering.md:58-64,81-83` 从 command phase 的 hit/focus/IME 到布局后 ‘same facts’ 输出，未定义 geometry snapshot 的发布时点；`:61` 甚至写在 Taffy layout 之前 cleanup focus。`docs/research/tailwind-coverage.md:33,87,104,111-123` 将 hidden、pointer-events、transform、focus、inert 分开列为目标。真实 `examples/native_poc/src/layout.rs:95-107` 只验证 Taffy measure；claim matrix `docs/research/validation-and-roadmap.md:9-13,49-55` 明示尚无 system input/A11y。
- 失败场景：滚动一个 For 行到 viewport 外，paint clip 了它，但 hit map 使用未 transform 的 Taffy rect，仍点到不可见 Delete；点滚动/切换分支同一帧内布局 N+1、A11y 仍发布 N，屏幕阅读器对错位矩形发 action。若 visibility:hidden 当 owner 卸载，筛选输入丢订阅和 IME；若 merely invisible 节点仍有 a11y focus，键盘会落在看不见的删除按钮。
- 建议：每帧生成不可变 committed GeometrySnapshot（含 logical rect、累积 transform、完整 ancestor clip、scroll/sticky/DPI epoch、paint/hit z-order），paint/hit/A11y/caret/action 都以它作唯一几何源；command 可读取最后 published snapshot，但所有 action 都经 ViewId+generation 在当前树复核。明确结构/焦点清理与 geometry snapshot 的 publish 顺序，禁止异步并行各自拿一份 Taffy layout。visibility 保持 owner/lifecycle 不变；语义单独建模 Hidden（不参与 layout/paint/hit/a11y）、VisibilityHidden（保 layout，不画/不命中/不可聚焦/从 a11y subtree 隐藏）、Disabled、Inert 和 clip，不能用同一个 bool。
- 验收：Red：用 unclipped layout rect 命中滚动区外 child、pointer-events-none 仍命中、删除后 action 命中新 generation、分辨率 1/2 时四路坐标不一致，均失败。Green：同一个 committed `GeometryFrame {clip_chain,transform,scroll_offset,dpi}` 产生 paint/hit/A11y/caret；fixed+scroll+transform+父子 clip+overlap z-order 的金样四路一致；layout frame N、结构事务 N+1 交错时旧 pointer capture 被取消或按明确规则转移，不误触发。table-driven 测试覆盖 Hidden/Visibility/Disabled/Inert 各自布局、绘制、命中、focus、A11y 与 owner counts，符合好的 native 语义；focus-cleanup 在 publish 前完成。至少真实 winit/AccessKit 的垂直切片再宣告完成。

### P1 路线判断正确，但当前纵向验证没有碰到“保留树+图”的交汇点

- 证据：`docs/research/CONCLUSION.md:13-26` 和 `docs/research/README.md:11,17-19,41-49,66-74` 已经选对保留树+图及明确拒绝 VDOM；但真实根 `Cargo.toml:1-8` 无 runtime 依赖，`src/main.rs:1-3` 仅 hello-world，`examples/todolist/main.rs:1-4` 和 `examples/todolist/README.md:1-3` 也标为目标。实际 reactive assertions 只覆盖局部 sink/dynamic edges/batch/dispose（`examples/desktop_poc/src/lib/project_signals.rs:212-336`）。
- 失败场景：项目后续先扩展最省力的嵌套 Node builder，把 components 当普通函数递归展开、For/Show 每次重建 children，再加 reactive signals；发现局部状态无法保留时才补 keyed reconcile，最终同时存在静态 Node 树、VDOM diff、signal sinks 三条状态到 UI 的通道。这样做即使 TodoList 外观运行，也违背 run-once/For 保留和精确 resource cleanup 的核心目标。
- 建议：不用换架构，要把架构落实成最小代码：先给 zenui-tree 的 NodeId identity、scoped Owner、fieldset patch 和只有 For 使用的 keyed structure transaction 建真正 probe，然后将现有宏 parser lowering 到相同 MountPlan。不要给 macro_poc::Node 继续加功能直到它看起来像实际框架，也不要把尚未验证的统一 Memo/render graph 语义画成已执行结论。优先级最高的是‘单个文本 binding + 两个条件共享 Memo + 可过滤 keyed 行 + 真实 subtree dispose’的可执行纵切。
- 验收：Red：根 crate 引入 reactively 重建 `Vec<Node>`、compare-old/new-tree 或新建的通用 vdom crate 作为默认路径，架构检查拒绝；基线文档未标目标而声称组件/For/AccessKit 可用，文档验收失败。Green：将当前宏标为 static parser probe、reactive 文件标为 flat graph probe；实现 `zenui-tree` 最小真实 ViewId 和 scoped Owner 后，宏生成的同一 mount plan 可在无宏 builder 中运行；然后再接 native window/geometry。所有文档继续区分已执行的局部 sink/measure/closure preservation 与目标语义。


**总建议**：明确保留“一棵原生保留 UI 树 + 单一 reactive graph”，且不要引入 VDOM：树持有稳定 ViewId、widget/focus/scroll 状态和资源，图只负责派生值及生成 typed patch，OwnerId 负责图与子树共同卸载。这样字段更新不必重建视图或 diff，列表移动仍可保留原生状态，而 `For` 才用 keyed diff 决定节点增删重排，不能直接替换为任意孩子 Vec 重建。应先实现并验证上述 For 过滤与共享 Memo 的最小结构切片，再扩展宏、组件和 visibility/geometry。边界：根 crate 目前仅 `src/main.rs:1-3` hello-world；已验证的 reactive 与 macro probes 不等于已有生产树、组件运行时、窗口或 AccessKit。
