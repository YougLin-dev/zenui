# 第二轮交叉反驳与主控反方

本文件保留三个不同领域组的交叉评审。候选编号 D1-D12 不是最终合同，主控对它们的取舍写在 [FINAL.md](FINAL.md)。

> **历史候选警示**：D4/D8 的“condition_rank 在 origin 之前、hover 胜过普通 inline”和 `class:if-*` 方案已被独立 CSS 事实核验推翻。当前 ordinary author inline 胜过普通 class/hover，无重要性与 layer 时 hover=false/true 都一样；见 FINAL §6 与 [VALIDATION.md](VALIDATION.md)。其他结论也只代表当时评审，不能绕过 FINAL 使用。

## R2-状态与语法

### D1【部分采纳】Store 只能是键化模型与字段投影，tracked domain 中的查询登记实际读到的成员/字段依赖，不得引入整树 render 权威；production handle 采用 typed runtime/slot/generation，不能借 Rc 存活代替有效性。但必须补齐计算期写保护与最外层 batch 原子提交，否则“精确依赖”仍会把中间态送到 UI。

- 结论：部分采纳
- 被审查的角色/主张：S、X1、A2/E3；已先承认 X1 的字段查询必须建边及 S 的多次 move 捕获需求，但不接受其更激进的即时 Rc 改造结论。
- 最强异议：S 最强反对是：现有 Signal 含 Rc/Weak，若不立即改成单字段 Copy ID，Todo 的多闭包 move 与“不复制数据”仍无法兑现。这个人体工学问题真实存在；但 Copy 既不是 stale 防护，也不证明每次访问无 Rc 成本，把 PoC 强制改成 Copy 反而可能提前锁死 arena 句柄布局。
- 依据：【当前实测】本轮无，全程只读，未运行测试或工作流。【源码事实】project_signals.rs:17-22 的 Signal 含 Rc/Weak；:27-45 失效后仍读旧值，:47-55 静默丢写；:128-160 flush 允许 sink 自写无限续队，:163-169 嵌套 batch 会提前提交。另 Store/Memo 尚无实现，不能把目标查询说成已复用或零拷贝。目标只需冻结行为，现有 Rc PoC 可保留 Clone 并加负向/TODO 验收。
- 提议合同：【目标合同】模型与 UI owner 分离。查询只能在 tracker 内动态登记 (row,fieldMask) 或成员索引边：All 列表不因 completed 改变而重建，Active/Done 与 remaining 应按已完成字段更新，兄弟 title 不失效。生产 handle 的 runtime 标识拥有者，slot+generation 防复用误命中；命令 try_read/try_write 返回 InvalidHandle，binding 遇失效必须结构化诊断并取消本次 commit，均不得回旧值或伪装成功。句柄是否 Copy 是后续 API 选择。执行顺序固定为 Outside  admits writes→Mark→Memo 拓扑稳定化→Binding compute（用户写返回 WriteDuringCompute）→单点 Commit→Owner cleanup；嵌套 batch 仅最外层稳定后提交，cycle/panic 后 phase 可恢复。不要求在此引入 VDOM、第二 Store 或用户手写 phase DSL。
- 验收：【red/green·待验证】red：当前 fixture 证明 dispose 后可读旧值、set()=()，且“binding 读后自增”子进程超时、嵌套 batch 可观察 true 后再 false；另以未登记字段依赖的 Store mock 演示行变色而 remaining 不变。green：失效或复用 slot 后旧 handle 的 try 读写均 InvalidHandle；自写返回 WriteDuringCompute 且不提交，嵌套 a/b 写只产生最终 false 一次；All 下改 completed 只更新该行及计数，不重建 TodoApp/其他行，Active 下精确改成员；同批 diamond memo 仅稳定态可达 binding。Copy 大小/Rc strong_count 不作为通过条件。

### D2【部分采纳】保留组件 mount 一次、普通数据表达式仅取初值、显式动态绑定、typed 非追踪事件、keyed For 及 UI/模型分别销毁；但“任何 For 都取得模型 handle”不成立，静态 FILTERS 不应伪造成 Store 行。仅分离 UI owner 还不足以挡住排队中的旧点击，必须补 View generation 命令校验。

- 结论：部分采纳
- 被审查的角色/主张：X1、U3/E3、E1/U2；吸收 X1 关于筛选保留编辑状态的最强理由，但不把默认 owner 语义偷换成 retain。
- 最强异议：X1 最有力的反对是普通筛选反复卸载同业务行会丢编辑草稿、caret、IME 并重建 layout，不能用“模型还在”掩盖体验损失。正确回应不是宣称 dispose 无成本，而是把可见性缓存与 keyed 结构生命周期分开：本基线只承诺模型不删，若产品承诺恢复局部状态，另用有界 keyed snapshot/显式 retained-visibility 验收，不能默默无限保留所有筛掉行。
- 依据：【源码事实】Todo main.rs:89 的 For 是静态配置数组，:118-125 才消费模型投影；README:10 明确过滤可释放 UI scope 而真实删除才释放 row model。native-macro.md:97-100 只允许重排保留，删除 key 销毁 owner；没有默认筛选保活合同。当前宏亦无 For/Props 实现，故这些是目标语义而非现有行为。
- 提议合同：【目标合同】四类位置各自 typed：组件 setup 每实例一次；普通 {expr} 在 mount 求值一次；数据位置的 move || expr 生成专用 RenderBinding 并只在 tracked context 读依赖；on:* 映射控件规定 payload 的 FnMut(E)，读取永不建边，mount 不执行。For<T,K> 始终要求 key，静态配置迭代与动态 Store 快照共用结构 API，但仅后者向 child 投影带 generation 的模型行 handle，TodoId 只作 key、不是存活证明。key 重排保留 ViewId/owner，key 消失销毁 UI owner而保留模型，重现获得新 view_gen；真实删除才使 row model/字段失效。框架 dispatch 在用户事件执行前先校验 ViewId+view_gen，进入命令后再校验模型行 generation，杜绝旧 UI 恰因模型尚存而写成功。UI 过滤退出后的草稿恢复不是默认免费承诺。
- 验收：【red/green·待验证】red：缺 key、错误事件 payload、把动态字段写成一次性表达式均有定位到属性的编译失败；运行时重放“点击排队→筛掉该 key→执行”会错误改模型即失败。green：静态 FILTERS 无 Store/Memo handle 即可编译；父组件 mount 计数始终 1，普通初值不随 signal 重算，动态字段只 patch 自己，事件连发两次各收到正确 String/bool/SubmitEvent 且零订阅。For 重排不重建，重复 key 在树修改前报错；过滤销毁 UI 而 row 仍可查询，迟到点击被拒，重新出现的新 view_gen 正常；真实删除后旧 field handle 失败且复用 slot 的新行不受影响。

### D3【部分采纳】必须删除 String+derive(Copy) 造型、禁止 Memo<Vec> 隐式拥有式 get、让 Show 使用独立 bool；但“廉价快照”不能只改名为 SnapshotId/Arc 就宣称零成本。需统一 Memo 的成本可见读取合同，并为 RowSet 固定版本、共享和物化次数。

- 结论：部分采纳
- 被审查的角色/主张：E3/S、X1、U1；认可 E3 对 Copy 语病的批评，但反对其“同一文件立即全面 ID 化”验收，及任何借小句柄掩盖整表复制的方案。
- 最强异议：最有力质疑来自初学者与零成本两端：U1 不希望为显示空态学习一套 RowSet/句柄 DSL；X2 类红队则会指出 Arc<Vec> 的 O(1) Clone 仍可能掩盖每次查询 O(n) 重建。故 API 要简单且成本可测，既不回到 String Copy，也不许以类型名 Memo/Snapshot 代替分配与求值证据。
- 依据：【源码事实】main.rs:26-31 的 Copy+String 按 Rust 规则必然不成立；:112、:119 同一 visible 被 Show/For 两个消费者读取，但 Memo::get 尚无真实签名，无法断言目前是哪一种复制。README:12 也明确共享求值待 production runtime 验证。因此禁止的是可能的隐藏 clone API，不是虚构一次已测过的性能故障。
- 提议合同：【目标合同】TodoRow 仅含稳定 TodoId 与 Store 字段句柄；title 是不可变文本投影，在 mount 取得一次显示所需值，completed 是字段投影，重排/计数更新不复制文本。production TodoRow 可复制的是小 handle 而非业务数据，现有 Signal<T> 不必 Copy。通用 Memo<T> 的 get 仅限 T:Copy；非标量只能 with 借读或调用显式 snapshot()，不提供返回拥有式 Vec 的便捷 get。For 接收带 revision 的不可变 RowSet/TodoRow 投影，O(1) 共享底层成员集，revision 改变才触发 reconcile；允许一次必要的 O(n) ID 物化，但不允许每个消费者各建一份。Show 使用单独派生的 Memo<bool>，与 For 共享查询失效信息而非强制共享 Vec 物化。内部 Rc/Arc 是实现选择，不作为公共生命周期工具。
- 验收：【red/green·待验证】red：现 TodoRow 得 E0204；Memo<Vec<T>>.get() 无论要求 Copy 还是隐式 clone 均应被拒绝；仪器化空态读取物化 N 行或 For 再物化第二份也失败。green：目标 fixture assert_copy<TodoRow>()（使用生产句柄）成立，ienna Show 仅取 bool、不进行 TodoRow/String clone；每次 changed list revision 至多一次明确的 RowSet 物化，For reconcile 只处理增删/换位，未变行 setup 不重跑。remaining/has_completed 等标量 Memo 同批仅计算一次；删除模型后旧 title/completed 访问走 D1 错误路径。Handle 是否单字长和 Rc strong_count 不作门槛。

### D4【最终未采纳其条件排序，保留 StyleIR 字段化方向】动态 class/IR 的字段化是候选输入；本节具体 condition_rank 与 class:selected 语法已被后续事实核验及 FINAL §6 取代。

- 结论：采纳
- 被审查的角色/主张：E1、U2/X2；赞成 E1 对整表 styles 覆盖的源码发现，但不接受让属性来源顺序成为临时优先级；同时保留 U2 对熟悉 class 写法的合理需求。
- 最强异议：最强反对是完整 variant/cascade 表工作量可能压过核心 Todo，E1/X2 希望先限范围。确实不应 Gate A 复制浏览器 CSS；但推迟冲突矩阵不等于可以不定义规则。只需固定已开放 variants 的小表和可扩展算法，未登记组合直接编译错误，不必实现任意 selector/plugin 才能得到确定行为。
- 依据：【源码事实】macro runtime Node::styles 在 :122-125 替换整表，宏可重复生成 .styles；desktop style.rs:181-188 只是按迭代顺序覆盖仍 active 的字段。style-system.md:62-63 让 direct inline 排在 class 后，却又说 dynamic class 不自动胜过 literal/inline；README:16 仅明确“同层同条件 inline 优先”。两者的差异必须在合同里解决，不能让两个实现各选一种。
- 提议合同：【目标合同】动态类只写 class:selected={move || bool}：selected 在构建期 registry 映射到完整不可变 StyleProgram/property mask，bool 只启停其条件；不提供 ClassString、运行时拼接/拆分或任意 ClassId 字符串。静态 class 的 variants、typed style:* 和具名动态类均先 lower 成 Declaration。逐 property 过滤当前 eligible 声明，再取排序键最大值：important→layer_rank→condition_rank→origin_rank→编译期 variant_rank_tuple→utility_rank→source_order。condition_rank 首批固定 Always(0)<selected/checked(1)<focus-visible(2)<hover(3)<active/pressed(4)，组合条件由同一 registry 预编译 PredicateId；origin_rank 为 intrinsic<theme<class<inline，只在前序相同时比较，故 inline 不压过更活跃条件；source_order 仅允许完全同 canonical 静态重复项后者覆盖，绝不能由宏属性遍历顺序决定。selected 等视觉条件与对应 aria 状态由同一字段派生，active 永远表示 Pressed。
- 验收：【red/green·待验证】red：交换 class/style 属性顺序或 Node 调用顺序会改变结果；动态字符串 class、未知 class:name、未注册组合 variant 分别在宏与 builder 路径失败；若无条件 inline 覆盖 active/selected class，按新合同亦失败。green：设 unconditional class=A、class:selected=B、unconditional inline=C，selected=false 得 C，true 得 B，hover 激活再按固定 condition_rank 得 hover 声明；非竞争字段不被 mask 清掉。同 canonical 重复 class 才 later-wins，所有矩阵与 builder/macro golden 一致，每个结果可输出 declaration/provenance 解释；状态切换只重解受影响字段，不重挂组件。

### 对其他角色的直接回应
- X1（U3/E3 的焦点与 IME 证据支持其顾虑） 提出：普通筛选应默认走 retained-visibility/hide-store；其最强论点是 keyed For 无法自动保留已离开可见数组的行，反复过滤会挂丢草稿、caret、IME 并付出重挂/layout 成本。
  - 回应：该成本成立，但不能把默认 keyed owner 销毁无条件翻转成永久隐藏缓存。D2 保留明确的结构语义、模型独立与 view_gen 迟写拦截；产品若承诺筛选恢复，需另设按 TodoId 的有界 snapshot/TTL、焦点和 IME restore 验收，或显式 retained-visibility。这样不会让大列表为了偶发草稿无限保留节点，也不把隐藏误当数据删除。
- S 提出：现有 Rc Signal 必须立刻改为单字段 Copy ID，且用 Rc::strong_count 证明零成本；其最强论据是 Todo 若不能把一个 handle 分别 move 到多个事件，所有权教学与 API 基础就同时破产。
  - 回应：现 Rc PoC 确实不能支撑该示例的 Copy 叙述，但 D1 不要求它立即改造是正确边界。production 的 runtime/slot/generation 与不可读旧值、不可静默写是真正的正确性合同；Copy 是之后可验证的公共人体工学，句柄也不必塞进一个 u64。单测应统计业务 Vec/String 复制、mount 与 patch，而不是把 runtime 内部合理 Rc 引用一并禁掉。
- E3（兼挑战 A2/X2 第一轮的失效矩阵表述） 提出：失效 handle 一律 Err；其最强理由是 UI 已过滤掉而 Store row 尚存时，排队 checkbox 若还能修改模型，用户会遭遇不可见状态变化。
  - 回应：问题在命令来源，不在所有 row handle 必须随 UI owner 一起失效。其他模型查询/服务仍应读未删除行；过滤只使 ViewId+view_gen 失效并移除 UI 订阅，dispatch 必须先拒绝旧 UI 命令，实际删除才使 row fields generation 失效。D2 把这两个 Gate 串在同一验收中，而不是让 UI 生命周期污染 Store。
- E1（U2/X2 对 typed payload 的担忧） 提出：事件必须 typed handler registry/专用 trait，但内部动态分发应清除；最强理由是现有裸字符串 fire 只调首个零参 handler，无法保证参数和调用次数。
  - 回应：保留其公开合同，反对把实现手段当目标：on:* 在宏中编译到控件限定的 on_input(FnMut(String))/on_change(FnMut(bool))/on_submit(FnMut(SubmitEvent))，公共 builder 不再接受任意事件字符串，执行域 untracked；底层类型擦除异构表若由这些类型安全入口构造可以保留。red/green 证明错误参数不编译、同名不同控件不串值、每次 dispatch 恰好一次，同时 mount 不读事件、不建立订阅。

### 未决
- 筛选后退场的局部草稿/caret/IME 到底默认丢弃还是按 TodoId 有界恢复，源码只列 snapshot/hide-store 两种可能，尚无唯一产品合同；焦点真实窗口行为也需后续验收，不在此次静态结论中冒称通过。
- 失效便利 API 的 dev panic/release 诊断映射、WriteDuringCompute 是拒绝单写还是事务回滚，尚需 runtime 原型定稿；不可协商的是不得返回旧值、静默成功或提交中间态。
- RowSet 的持久化/增量存储形式与真实分配预算尚待 profile；O(1) 共享不等于查询零 O(n)，应据过滤和重排 workload 定物化次数、内存基线。
- 组合 variant 的完整 PredicateId/rank 表及与固定 Tailwind 版本的差分仍需 fixtures；D4 已冻结算法和首批状态秩，但不据此宣称完整 CSS/Tailwind 兼容。

## R2-真实桌面体验

### D9：TextEdit 本地状态与 Store committed draft 分权

- 结论：部分采纳
- 被审查的角色/主张：X1、X2、A2、E1（事件）
- 最强异议：X1 的 TextEditState 一体权威与 origin split 已经足够；引入双模型/事务 id 会凭空扩 API，且 composition 真状态仍需平台验证，未必该第一门落定。
- 依据：方向正确：受控字符串回灌是 caret/IME 风险源，README:15 也把仅外部逻辑值更新当目标；但“外部值改变”没说如何区分同一命令回传、异步外部覆盖与 active composition，revision 单调性及 selection 定义均空。不能以一个名义 API 声明解决。
- 提议合同：【目标合同，非现状】采用双轨+带 origin 的 TextEditCommand，而非让 editor 直接写 Store 又把当帧回传认作外部 revision。TextEditSession 持有 value/selection/composition/revision；draft 为 committed logical value。用户一次 committed input 原子提交 Command::LocalInput{editor_id,editor_revision,origin,value,selection}：Store 记录 (value,origin_editor_id)，禁止同步向 origin editor 发 ExternalSetValue；其他 editor/binding 可更新。外部 draft.set/load/undo 用 Command::ExternalReplace{origin,value,revision,SelectionPolicy::{PreserveClamped|SetRange|CollapseEnd}} 替换整个编辑事务，不逐个闭包推断；若在 composition 中，默认先 commit 当前 value 为最后 committed logical value，再替换，需外部取消时显式 CancelComposition。Key/Submit 路由先问 composition，IME 确认 Enter consumed；automation 直接走同一 reducer。初始仅单 editor 可设 origin token；多 editor 并发冲突规则另门验收。
- 验收：red：本地 composition 输入引发同 editor 的字符串外部 patch，或以字符串等值/当前 selection 猜测 revision；候选 Enter 触发 submit，或外部替换只留下未规定的 caret。green：reducer fixture 覆盖本地 input→仅写 Store、本地 Store patch→不再回灌 editor；外部 revision 非 composition replace→selection 夹到合法边界，replace 正在编辑的 composition→按合同 commit 到唯一 logical value 并清 composition；旧 revision 拒绝。OS 真窗再验中文候选确认 Enter 不 submit，下一 Enter 只 insert 一次。

### D10：最小真窗验证前移，headless 只验底层

- 结论：部分采纳
- 被审查的角色/主张：A3、E3、U3
- 最强异议：A3 最强反对是把 OS/input 提前到 Gate A 太早：应先独立 C0 测 per-window DPI、surface format、surface error/device lost 与能力矩阵；结果类型无法替代 backend 能力发现，也不要阻塞聚焦的纯交互 milestone。
- 依据：方向应采纳：A3 的图形能力发现和 DPR/设备丢失正是需要提前暴露的问题；但初门不应把 AccessKit/OS IME 的可获取性混为底层输入设计失败。源码目前只有 headless 离屏图、typed style 和手工 fire，历史 PASS 在文档中明确非三平台。规格需定义 gate 与 failure 的可比验收，而非仅提前把 Gate C 笼统提前。
- 提议合同：【目标合同；当前实测仅仓库历史，非本轮复验】不设含混的新 gate 名，改 Gate A 定义最小可运行交互合同：winit WindowEvent/TextEdit/InputRouter/focus provider/scroll geometry 驱动同一 Todo 小窗，AccessKit adapter 提供 tree/focus/action，事件 reducer 可 headless 重放。Gate A 先在可获得的 Windows 真窗必跑 IME、wheel、Tab 与 semantic dump；headless 只能覆盖 reducer、selection、generation 和 geometry。Gate C 完成 Windows(中文 IME+NVDA)、macOS(VoiceOver)、Linux(可用 AT-SPI client) OS 矩阵，Wayland/X11 分列。任何 unsupported capability 是 typed Result+可见 UI，禁止静默 fallback。首个标记支持桌面的 release 以 Gate C OS 实机为准，不把 OS runtime 的不可用判作 Gate A 红。
- 验收：red：headless/unit 测试称输入/焦点/读屏已通过；任一支持 OS 窗口缺结果仍发桌面首版；不支持 Wayland/CPU 特性时 panic 或静默假成功。green：Gate A target 发真窗事件，Tab、wheel、TextEdit、focus action 驱动同一 reducer，AccessKit snapshot/focus/node identity 同 geometry；只缺少未获 OS runner 的能力明确记待验证，不以 headless green 冒充。Gate C Windows CJK+NVDA、macOS VoiceOver、Linux 可用 AT client 都有实机日志；Wayland/X11 分开；CPU/format unsupported 返回 typed Result 加用户可见错误，测试退出码为失败阻断发布。

### D11：Todo 固定控件回归普通 Rust，Store 必须可构造

- 结论：部分采纳
- 被审查的角色/主张：U1、U3、E1、X2
- 最强异议：E1/U3 最强反对不是新手复杂，而是不得牺牲类型和所有权契约：固定按钮也许简单，但源示中 TodoRow 包含 String 又 Copy，又如 add_todo 复用单个 captured closure，简化表层语法会掩盖 move capture/事件载荷/child factory 语义，仍无法编译。
- 依据：采纳简化及禁止虚 Store 注入，但源码事实不允许原样通过：main.rs:26-30 的 Copy+String 直接 E0204；TodoItem 中 title 移动到 format! 后的 text/aria 也可能所有权冲突；add_todo 在一个 move handler 中被捕获会导致其它 handler 复用问题。条件按钮若写成普通 builder 层 if，组件 setup 一次执行，响应行为必须依靠动态 child/binding，与原生宏草案所强调的一致。这些不是细节缺陷而是 D11 验收必须覆盖的主所有权问题。
- 提议合同：【目标合同+必须从源码修掉的错误】TodoApp 无参数，main 自己 new/run application-owned Store；另给一个 store fixture 便于 command tests，不能把外部无实现 Store 当唯一运行入口。三筛选 button 直接写；真正任务行保留 For。Clear completed 用 Data Binding {move || if has_completed.get(){Button(..)}else{Node::empty()}} 这类明确 Rust if 表达式，若 Show 只作备选不可混成等价语法。title String 为一次性初始 prop，index/key/checked 事件仅复制 id+Copy handle；删除 TodoRow derive Copy，或构造时取出 String 建 OnlyCopyRow，不能维持源码不符。visible 空与 store 空各用独立 tracked bool，对标题/accessibility 的重复 String 读取需 with 或一次 clone 明示。示例不得暗示仅删除复合 For 就能运行。
- 验收：red：三按钮仍由 For 生成且继续写 Copy+String，或 TodoApp/TodoItem 还需不存在的外部 TodoStore 注入；只删除语法却让 mount 不执行事件等回归失效。green：一个完整可 cargo check 的 fixture 自己定义 main/window bootstrap 与 application-owned Store；三个筛选 button 静态列举，Clear 用普通 Rust if；真正任务列表保留一次 For key，出现一次性 title 初值、Copy handle 多闭包、String 的 with、tracked Store 查询。空态用两个 bool/Show 分支；删除再用 inline component 构造 TodoItem。新手 fixture 对动态绑定必须有明确 move/binding wrapper，不承诺普通 Rust if 自动响应。

### D12：焦点继任、generation 提交和两个空态

- 结论：部分采纳
- 被审查的角色/主张：A2、E3、X1、U3
- 最强异议：A2/E3 的反对是单个 successor 规则仍不够：For 临时过滤释放 UI generation 但 row model 仍活，彻底删除又让 row generation 失效；late async action/resource 提交也必须各查对应世代，隐藏/恢复可能保留 editor snapshot 又要避免恢复错误 IME。
- 依据：核心策略对，允许优先 filter 不能回避禁用 Add 的可访问性，且一个通用 generation 含混了 A2 强调的 UI/模型/resource 三代。D12 最弱点是只写卸载 focused row而没规定同 batch 多行删除、指针抢占、active composition、scroll reveal 和 focus kind，per-node assertion 不够。live status 现在不存在输出，需明确目标与实现边界。
- 提议合同：【目标合同】在引起卸载的 Store/Filter command dispatch 前捕获 FocusRemovalIntent{old TodoId,focused control kind,old visible index,mutation epoch}；按移除后的可见快照重建候选列表，相同 control kind 才能自动接任。顺序：origin 的原位置之后第一可见行→之前的最后可见行→Add input→filter toolbar。D12 候选含 Add 才能避免禁用 Add 时再次卡死；如果一定要 filter/Add，必须明确禁用 Add 也不可聚焦则跳 filter。Direction-aware 延伸：后续正向键/search-next 若意图没消费则优先下一行。比对原 origin（用户中途 Tab/click 合法取消旧 intent），事务提交时一次性更新 focus pointer/reveal/AccessKit；generation 分三类判定时校验：View(view_id,generation) 对结构/焦点、Row(TodoId,generation) 对字段命令、Resource(owner/key,generation) 对异步加载。filtered-out：可保存 committed edit state，但不自动恢复 active composition；model delete：作废对应 restore。focus 批量删除取所有未删可见行，从原 focused 位置重算目标。empty_store vs empty_filter 是独立 tracked bool，对应固定消息。隐藏节点从 hit/focus/AccessKit tree 移除；live-polite announce 是目标合同，首门可验 event+tree，发布才可宣称 live support。
- 验收：red：checkbox Space 使行消失后 focus 到高索引控件；中途用户操作仍应用旧焦点 intent；过滤回显恢复一个 composition 已 commit 的旧快照；stale async remove 成功或 focused clear 只留 body；空数据文案相同。green：active 行删除后保 focus kind 到原索引的下一可见行，再上一，再 Add（若 enabled）或 filter(kind=Add,kind=None) 按规范；node resource commit 分别拒绝不匹配 generation。Clear 批量同 focused 规则，未涉及 row 时 focus 留 button。语义 visible set 不包含隐藏行，NVDA 无 active desc 指向已删节点；两个空消息精确匹配；live region 实际 AT 通知在实现之前标目标。

### 对其他角色的直接回应
- A3真窗和平台能力前移的优先顺序 提出：A3 反向挑战把 IME/焦点/scroll/AccessKit 一律前移为过宽 Gate A，认为应先验证真实 surface、DPI 与 device-lost 的 renderer boundary。
  - 回应：承认该反对的真因，修正为 Gate A 最小 input/semantics driver＋并行 platform probe。聚焦交互不以三 OS 不可用来判红；首个 OS/DPI surface 端到端及 capability Result 单列 Gate A/C，D10部分采纳而非一股脑提前全 Gate C。
- E1类型/所有权与DX 提出：E1 认为 D11 表层简化不能解决 Copy+String、闭包 move 和 typed event/child factory；简单例子可能掩盖而非解决 ownership。
  - 回应：接受，D11 的规格加入修 TodoRow String→初值/独立 Copy row，add_todo 以每次复制 handle 构造命令；普通 Rust if 只能在reactive DataBinding位置使用，并获得明确包裹，而不是运行时隐式捕获任意表达式。
- U1/U3新手跟做门槛 提出：U1 反对一个综合Todo同时教 Memo/For/外部传入无实现Store，新手会先被闭包与运行入口卡住；D11 应进一步缩成阶梯示例。
  - 回应：同意把示例Store所有权纳入main且三按钮直写，但不删除真正列表For和Store查询，因为这是Todo必须验证的UI生命周期。可读性靠固定控件与少了嵌套For实现，不得把未实现Store说成初学者自己负责补齐。
- X1状态分层与过滤保留 提出：X1 的强论点是过滤基于Vec删key天然卸UI owner，不应与一般重排混同；TextEditState应整体维护selection/composition，不是靠表单特判和string相等。
  - 回应：D9补充origin command和外部替换事务，避免相同自环写回；D12采用View/Row/Resource三代并禁止恢复活跃composition。过滤是否默retained-visibility不在本组四项中直接定局：若选用隐藏保留，small Todo要有限量与真正删除可回收的验收，不能以性能名义拖住最小闭环。
- U3读屏/真实可学交互 提出：U3认为除名字外还须form/list/listitem typed role、update announcement和真实NVDA验证，否则写完aria:label只会造成虚假已支持的安全感。
  - 回应：纳入D10/D12：当前源码宏会拒绝aria:label/aria:pressed且runtime只有零参Event，因此全是目标合同。首门只要typed semantics tree＋事件可验，不能写NVDA已支持；Gate C实机听到焦点接任与公告再开放live status。

### 未决
- D9 多 editor 并发与 composition 中外部替换的默认 commit/cancel 策略尚待与文本编辑 API 定型；当前源码没有 TextEdit。
- D10 的真实 IME、AccessKit action/announcement、Wayland/X11、Mac/Windows、DPI/resize/device-lost 与 CPU capability 仍全部待验证；历史 Windows headless 记录不是本轮实测。
- D11 最终示例是一个综合可运行Todo还是另拆Counter/Input阶梯，以及 macro与typed builder是否共用同一动态子节点语法，待主控选单一版本。
- D12 过滤时默认卸 owner＋committed snapshot 还是有界 retained-visibility、方向键例外以及多行删除的明确 focus target 尚需统一；禁用 Add 时的继任必须含于此决定。

## R2-更新、后端与样式

### D5

- 结论：部分采纳
- 被审查的角色/主张：A2；X1（UI 派生树是否构成第二权威）；E2（默认结点更新契约）
- 最强异议：X1 的最强担忧是结构化过滤会诱导再建隐藏 Store/三树，或 strict phase 让常规 Todo 变得不可写；A2 则证明现行 flush sink 中自写会无限驱动、嵌套 batch 泄露中间态。这两个观点并不冲突：反对的是第二套可变 UI 权威与不受控写，而不是一切派生缓存。
- 依据：单树/单派生图避免多权威，计算期禁写为解决已证实重入风险的正确边界。但当前候选把 cleanup 放在 binding commit 之后、且没有事务与 Memo 自计算、排队后续写入的规格；不能让人体工学 closure 获隐式写权限，也不应将所有计算/命令变成专家 DSL。
- 提议合同：采用【目标合同】单一 NativeTree（ViewId+generation+widget state）与唯一依赖/派生图；Signal arena、Memo、field binding、layout/text/style/Scene 均为该图/树的数据或投影，不建可独立提交的 UI/VDOM。每事务只允许 Outside 用户事件/队列写 -> Mark（全 transaction 标脏）-> 拓扑 Memo 稳定（equality 仅裁无效传播）-> Binding compute（只算 patch，禁用户 Signal/Store 写、树结构变）-> Tree commit -> deleted ViewId 的 focus/capture/IME/resource cleanup -> layout -> paint。batch 为嵌套计数：内层不 flush；frame 合批多个 Outside 命令。非 RenderBinding 的 Effect/业务 command 留在下一 Outside 排程，不能借 untrack 在计算段写。用户写入口 Result/dev 诊断明确错误，不得静默忽略；runtime Memo 内部标脏是唯一豁免，不是用户 API。当前【源码事实】project_signals.rs:17-55 的 Signal 含 Rc/Weak，失效仍可 with 读旧值、set 静默返；128-169 的 pending 循环和 batch 不是所定稿协议。
- 验收：red：当前读写 count 的 sink 在 compute 内 count.set 将再次入 pending，嵌套 batch 内层可 flush，且无上限；counterexample 可挂起或暴露中间值。green：嵌套 batch 仅最外层稳定一次；compute 用户写返回 WriteDuringCompute 且本帧提交该 binding 前旧值不变/按统一回滚合同处理；guard 恢复 phase；memo cycle 报 ReactiveCycle；同一批事件连续运行，diamond 无中间态，panic 后可继续下一事务。明确 memo 内部写是 runtime privileged path，不开放给普通用户。

### D6

- 结论：采纳
- 被审查的角色/主张：X1（主张过滤默认走稳定 ID+retained-visibility）；U3/E3（主张筛选有状态保留与可预测焦点）
- 最强异议：X1 最有力的反驳是过滤现有 key 序列时，未删除行会频繁 unmount/Parley layout 重建，局部编辑/IME 丢失；普通用户希望过滤像隐藏而非销毁。此担忧真实，但用稳定全集 For 默认保留会把偶发缓存变成所有权合同，且 slide hidden rows 的 layout/hit/a11y/虚拟化仍须定义。
- 依据：采纳 D6：X1 默认隐藏会混淆 For/Show 和生命周期，且未稳定 ID 集在无界列表上并不是免费缓存；但不能因此把过滤行全当不变结点。D6 明确 UI owner 与业务 row 的生命周期恰能避免 stale command 和资源无界增长。U3/E3 补的焦点/IME 规则必须由默认卸载路径提供，而非只有 opt-in 保留策略才处理。
- 提议合同：For keyed 合同为【目标合同】：reorder 保 ViewId、owner、局部 widget state；key 离开 each 的默认动作是 dispose UI owner/subscriptions/native resources，业务 row model 不归 For 拥有；同 key 再出现得到新的 UI generation。默认不承诺保留草稿、焦点、caret、scroll 或 IME。显式状态保留只允许二选一命名 API/组件策略：retained_visibility（保 owner，隐藏即 layout/hit/a11y 不可见）或 dispose_and_snapshot（释放 owner，按业务 key 保存/验证/恢复快照）；两者都不能复活真删除行，任何迟到 UI command 以 old ViewId+gen 拒绝。virtual cache 有明确行数/bytes 上限；超过回收；真实删除彻底销毁 row model。Todo 当前无行编辑器，README 应明确默认会卸载和焦点转移，不以 future hide-store 暗示默认免费。
- 验收：red：默认 For 在 reorder 中换 ViewId、过滤后遗留可命中/可读屏节点、过滤后迟到 gen1 回调改 row、无限 retained cache 命中都失败。green：A/B 互排保持 ViewId/UI generation/订阅/焦点合法；filter out/in 建 UI gen2、旧 click/IME/resource completion 拒写而 row model 尚在；retained-visibility/dispose+snapshot 需 API 明示、限定 max retained rows/TTL 和焦点去向；隐藏行不参与 layout/命中/AccessKit；业务删除使 model gen 失效；10^4 churn 后 nodes/edges/cache/focus 回基线。重复 key 必须在树修改前诊断。

### D7

- 结论：采纳
- 被审查的角色/主张：A3（backend capability matrix 与 Gate C0）；S（生产候选叙述与依赖域）
- 最强异议：A3 正确地指出两套线的 alpha/format 合同相反：desktop compositor.wgsl:12-19 为 straight-alpha source-over，sparse gpu.rs:69-127 是 premultiplied headless 参照；只换 crate 名会错色/格式不支持。S 的最强压力是 PoC 距生产还有 mask/filter、resize/device-loss、atlas 生命周期和真实窗口，不能声称平台能力。故采纳候选技术线，但拒绝任何“现有 PoC 已证明生产”的读法。
- 依据：方向与已复核源码一致，但没有实测依据把这套候选提升为平台已成立。需要把 CPU reference/必要 fallback 和真实 window capability gate 中的退出与降级纳入规格；只“保留 Classic 对照”不足以清掉它在生产编译与 API 的泄漏。
- 提议合同：Taffy 0.15.1 + Parley 0.13.x + sparse-strips vello_gpu/vello_cpu 0.4 + wgpu 30.x 保持【生产候选】而非已验证生产后端。统一内部 Scene/paint projection；vello_cpu 是 reference 与 capability fallback；Classic 0.12 锁在 probe-only adapter，不进入公共生产 backend/二号稳定 ABI。RenderCapabilities 必须查询 adapter/queue/surface formats、alpha/texture usages/limits 及每一种 effect 的支持；不支持的 Result::Unsupported，不能 GPU validation panic 当 fallback。外部内容接口是 {TextureView/TextureId、rect、format/color-space/alpha、size/scale、frame_epoch、owner_generation}；默认独立 offscreen color + depth（真实 3D 自有 Depth），再 ordered composite，不引用 Vello 内部 depth_view 作 3D depth、不同读写同一 texture 于一个 pass。当前【源码事实】gpu.rs 只单默认 adapter/device/Rgba8Unorm/128x64 headless，外部纹理是纯色占位；desktop 64x64 Classic 是 straight-alpha probe，无 winit/AccessKit 命中；validation-and-roadmap.md:3-14 是仓库自报【历史实测】，本会话未运行。
- 验收：red：headless PASS/文档自报被标为窗口或三平台通过即失败；unsupported 格式、atlas、filter/mask 走 wgpu panic 即失败；外部资源被释放后仍被 in-flight scene 使用即失败。green：产品 adapter 枚举 features/limits/surface formats/alpha/external usages 并 Result 报告 unsupported；headless RGBA premul 与 vello_cpu 同一断言通过，真实 surface 的 sRGB/BGRA 各自色彩 fixture 通过；Winit 窗口的 resize/DPI/SurfaceLost/DeviceLost 经 GPU+CPU 合同测试并标实际平台结果；triangle/mesh 到自己的 color+Depth，2D under/overlay 经 metadata 绑定纹理；owner/generation 回收无旧 TextureId 引用。Classic 仅留在 probe/reference，不作为默认 feature backend。

### D8

- 结论：部分采纳
- 被审查的角色/主张：X1（CSS-first 扩张风险）；E1（样式 IR 被 builder 覆盖）；U2（active/selected 迁移语义）
- 最强异议：U2/E1 最强反驳是 active 与筛选/勾选业务状态极易混用，而多个 class/inline 不是求并集就能成立；仅限制数量不足以定义正确样式。U3 反驳另一面是在研究文档过严不能牺牲 Todo 可读性。合理边界是保留普通 class 字面量和 typed style 字段，有限注册命名条件，不做 CSS selector DSL。
- 依据：严格 N/D/R/X 白名单与后置 CSS-first 很符合当前仅 toy 六个 class token 的事实。问题在候选仍含混地称 named condition 为 extension 却未统一现有 class:if-*、aria:*、future variant 表；若允许两套条件名，CSS 迁移和 typed native builder 都会分裂。必须语义与语法分清：少量与 Tailwind 对映的原生伪态可 N，业务命名注册 grammar 另设并标 Zenui。
- 提议合同：首版支持集是 pinned Tailwind version 下工具生成的 allowlist manifest：N 才放行，D 明文档+fixture、R 未接能力须 unavailable、X/未知/任意动态 String 编译错误；当前设计文档的 N 不能等同已支持。统一 StyleIR/field mask/provenance/Condition，宏与普通 typed builder 共用；不拼接字符串取 later-write 胜利。active 唯一映射瞬时 native Pressed，绝不等同 selected/checked。凡与固定版本 Tailwind 伪态同义的条件（hover/focus-visible/disabled/checked 等）可列 N 白名单并测试；class:if-selected/class:error 等命名条件只能在 Zenui 专用 namespace（例如 class:if-*），文档写 Zenui extension、不能写成官方 CSS/Tailwind selector，不能自造 selected: 冒充官方。selected/toggle 应用状态连接 typed semantics（合适 aria:selected 或 toggle button aria:pressed），不是样式别名。优先级固定成有限表：按 origin/condition rank/important/declaration order 逐字段合并，明确同层 inline 胜过 class；不出现浏览器 cascade/specificity general engine。CSS-first/@utility/@custom-variant/complex filters/animations 从核心 Todo build 依赖中移出，后置独立 gate。
- 验收：red：Todo 的 class/style/aria 即使 parser 认识却不同字段 IR/不同 builder 结果，或允许 arbitrary selector、动态拼 class 字符串、CSS-first 成为核心依赖即失败。green：pinned N 白名单覆盖 Todo 每 class token，macro/typed-builder golden StyleIR 相同；unknown/D/X 精确诊断；press/release/hover/disable/selected/checked 矩阵证明 active 只 Pressed；展开优先级表格（theme/default < class < dynamic class 同 condition ordered < inline；active variants > plain 同 origin）的传参 fixture 全过；filter 同时驱动 aria:pressed+视觉值；reorder 后无样式错绑；关闭 CSS-first/animation 的 build 仍可完成 Todo。report 只能由 manifest 工具生成不能抄 N 设计表。

### Common prerequisite（修正 S 将 Signal 改为 Copy+generation 即证明安全的推断）

- 结论：部分采纳
- 被审查的角色/主张：S（Copy ID+generation 即解决 Copy handle 核心承诺）；E2/X2（Memo/克隆与 typed payload）；U3（最短可运行路径）
- 最强异议：S 最强论点是当前 Signal 就是 Rc/Weak，声称 Copy handle 根本不实，且 stale read/write opacity 是所有目标事件正确性的根基。这必须修，但不能把降低 Rc count 当成 stale command、IME、Memo 一致性、平台能力都已解决；枯竭接口也会牺牲新手的人体工学。
- 依据：S 有关 Rc 属性和无 generation 的源码观察成立；但 Copy ID 只防复制成本/所有权形状，若 owner 可快照、commands 不验 UI generation，仍可陈旧操控。E2/X2 对 typed payload、MEMO<VEC> 和失效错误是独立合同而非 ID 换形就自动满足。避免所有方向待 runtime 写完而宏继续虚构 API，中央规格应明确“core contract-first，不是更多 Rust wrapper/GUI DSL-first”。
- 提议合同：四候选共同前置【目标合同】：先把可运行 typed core 收缩为最小 todo slice，再从宏暴露普通 Rust 风格语法，不能反向用目标 RSX 定义 runtime。类型上补：TodoRow 移除 String Copy 或 title 改 Store/field handle；Signal/Store/row IDs 带 owner/slot/generation；失效读写 Result；typed event payload；Copy ID 不持 Rc；普通应用命令/Event 用 Read/Write handles，不需理解 phase，框架宏将动态字段句法降到只读 binding。同步 core 的 backend 接口【待验证】，Style N 表【待生成】，所有【当前实测】仅限仓库自述/源码 fixture，不冒充组内实测。
- 验收：red：Copy 编译错误、stale 读旧值、静默写、同名事件错负载、样式覆盖、平台 unsupported panic 任一项被另行 API 习惯掩盖则失败。green：一个 un-macro core fixture 先过类型/读写/事务/StyleIR capability test；TodoStore/real input/For later slice 逐条打开并标目标能力；公开文档支持矩阵由 test manifest 生成。未达 Gate 的所有宏/UI 预览保持不能误构建，不能将 Into facade 路径、手动闭包、编译成功当运行或通过 Gate。

### 对其他角色的直接回应
- X1 （对 D6） 提出：第一轮提议默认稳定全集 For+retained-visibility，理由是重复过滤会持续 mount 和 Parley layout，未删业务行被卸载、草稿/IME 丢失。
  - 回应：性能/编辑连续性是真问题，但不是默认所有权合同的充分理由；默认 For 是 ownership projection，key 离开即 dispose UI scope，row model 存活，重新出现用新 UI generation。若应用要保编辑，在 list/API 显式 retained_visibility 或 dispose+snapshot 并给出界限；不能让优化暗中改生命周期、隐藏焦点/命中/读屏、无界保留列表。
- A2/U3/E3 （对 D5/D6） 提出：A2 要求递归批原子性和计算期封写；U3/E3 要求 Active 下勾选即过滤时焦点、编辑、IME 仍连续。
  - 回应：采纳其风险，但拆开：phase 合同必须补嵌套 batch、memo internal write、Else 至 Outside 而不是给 binding 写权限；默认卸载 filter row 时必须把 focus/selection/IME 转移/取消为确定状态，而不是靠保 owner 默认解决。当前仅有清理 deleted ViewIds 的愿景，没有后继焦点实际规则。
- S/A3 （对 D7） 提出：S 担心把 PoC 平滑拼成生产、Classic/sparse-alpha 代际依赖混用；A3 认为 headless 不能证明窗口，必须前置窗口 capability matrix。
  - 回应：读完两边后保留候选主线则给硬边界：Classic 只在 probe，不是生产第二 backend；sparse CPU/GPU 共享 Scene contract 但按 rgba、sRGB、alpha、device loss/effect cap 运行时检查，真实窗口另设 Gate。已有 native GPU 单裸绿纹和 parley CJK 仅是仓库自报 headless PASS，不是平台证据。
- E1/U2/X1 （对 D8） 提出：E1 发现多 class/inline 会被整个 styles 覆盖；U2 担心 active 被误写为 selected；X1 反对 CSS-first 全量优先。
  - 回应：三者应同时满足：采用 N 白名单，但白名单外 application condition 用显式 Zenui namespace，不能含混官方 CSS selector；StyleIR 逐字段合并、固定起源/variant 优先级，共享 builder 与宏。普通 class/inline 保留近似 JSX 的外观，无需要求用户写 selector engine 或 compiler IR。
- S/A2/X2 （cross-cutting core） 提出：S 把 Copy/ID 改造作为下一步中心；A2/X2 分别给 phase、generation、typed/快照缺口。
  - 回应：Signal 替换非 Copy Rc 必要，但不充分：copy u32+generation 仍不验证 UI 和模型两代、不解决嵌套事务、Memo<Vec> 多消费者或事件负载；请用公开 contract fixture 覆盖四类合同，再以其驱动宏/runtime，不把 public handle 也换成专家 API。

### 未决
- 【待验证】本会话只读；未重跑 cargo/trybuild/native GPU，所以仓库文档的Win11/NVIDIA/Vulkan/Rgba8Unorm headlessPASS仍只是历史自报；真实窗口、Mac/Linux、IME/AccessKit、device loss、performance均不能标实测通过。
- 【目标合同】需要主控明确binding compute失败的提交政策（拒绝该binding、回滚整个事务或错误后halt），一般用户失败与panic如何恢复下帧，即D5 red/green需选择唯一策略。
- 【目标合同】D6 retainedvisibility资源上限采用内存bytes、rows还是viewport overscan；filteredrow focus后继顺序、IME cancel/commit、snapshot TTL及真删冲突尚未选默认值。
- 【目标合同】D8固定Tailwind commit、首版Nallowlist/variant排序与inline冲突表尚未生成；Todo aria:pressed为toggle语义还是后改tab/listboxselected semantics需与Accessibility验收统一定稿。

## R2-主控反方

### D1 的字段查询建边与 D3 的不可变 RowSet 之间仍缺关键读取语义：tracked filtered 若要登记每行 completed，实际会产生 N 条字段边；若用 RowSet 缓存，memo 可能只登记查询/revision，完全未读取字段。现有“不因 completed 重建 All、但仍精确更新 remaining/Active”的验收没有禁止静默 stale。

- 原因：同一个普通 Rust 查询 hovered across 两种外观相同但依赖不同的实现，字段细粒度与廉价 RowSet 目前靠叙述拼接，无法验收；这会直接复现第一轮 X1/W 的“行变色、计数与筛选不变”。
- 最小修正：冻结算法而非仅名称：Store 查询逻辑在 tracked scope 执行并记录访问过的 (row, completed)；visible 将按 filter 适用的 RowSet/shared snapshot 缓存结果并暴露其 revision。All 有独立成员/membership revision，不依赖 completed；remaining/has_completed 与 Active/Done 明确登记当前相关行的 completed。规定 revision 的 bump 规则、共享对象和查询缓存失效规则，普通应用开发者不需接触 RowSet。
- 验收：三组可观察测试：All 勾一行的 completed 后该行、remaining 更新且 membership revision 不变、其它行不失效；Active 勾选导致恰好一次 query/cache 重算及 RowSet 物化，增删成员且 Show 只消费 bool；创建 N 行后断言 visible 的依赖记录/失效覆盖 N 行，而非仅 memo revision，复制过滤条件且无变更时不重复查询。

### D3 把 RowSet 的共享与 O(n) 物化放在 general Memo/Todo 合同，却未冻结一次 revision 与多个事务的关系。D5/D1 接受同批多次 Store 写，row-visible、remaining、has_completed 均可能各自使查询 dirty；如果 visible revision 在每次写入时递增，就可能多次物化。

- 原因：当前 green“每次 changed list revision 至多一次物化”可以通过在每次字段写都生成 revision 的方式虚假通过，或在实现中要求 field-write 直接物化列表，重新把图复制成本交给 Todo 新手语法背后的每一事件。
- 最小修正：将 RowSet 定义为 per-store transaction 的 derived snapshot，不能每个 row field set 立即生成；只在 topo 稳定后、实际 list 消费者读取时共享构造一次，并用 store transaction epoch+filter identity 标识。若 repeated query 消费不同 filter，各自只允许一次；空态 must never request RowSet contents。
- 验收：一个 batch 连续 toggle 两行、同帧读 remaining 和 visible，instrument query_evals=1、id_materializations=1、String clones=0，第二次消费不增加物化；分别在 All 与 Active 下断言字段写不直接 bump membership/materialize。空 store 和空 filter 各只算 bool 且 zero row visits/zero materialization。

### D2/D6 与 D12 对“过滤卸载后焦点和编辑状态”仍互相抵消。D6 默认明确不保住 draft/caret/focus，D12 却要求可保存 committed editor state，且文字中“active 行删除后”可被理解为数据删除而非 completed 后过滤消失。

- 原因：Todo 的 Active 中 Space 会让当前 focused checkbox 消失；若按 D6 无 intent 立即 dispose，则 D12 事后捕获的旧 visible index/control kind 已丢失，IME composition 也无法定义是 cancel、commit 还是 snapshot。结果是唯一危险的键盘流程没有确定默认行为。
- 最小修正：选定一条默认并贯串三份合同：结构变更 dispatch 前 capture FocusRemovalIntent，commit 时连同 owner cleanup 执行；过滤卸载只丢弃 local transient draft/selection、cancel composition（or retain no state），不声称默认恢复；真实删除作废全部 snapshots。若产品要 committed editor restoration，则明确 opt-in snapshot key、内容与 TTL，不在第一 Todo 加入。把 D6 的“no focus preservation”改为“不保留 focused identity，但有确定的 focus transfer”。
- 验收：Active 下在 checkbox 按 Space，一次事务内断言旧 View generation 先失效、IME cancel 一次、focus 转同 kind 下一可见行否则上一再 Add/filter，AccessKit 与 hit tree 同步；All→Active→All 后未删 row 获新 UI generation，draft/caret 不默认恢复，late click/cancel 均失败；同批 Clear completed 多行删除使用同一 intent 且不留下隐藏焦点。

### D9 的双轨仍缺“本地 Store 写所在事务何时通知 editor”的硬时序。合同只说 LocalInput 禁止 origin editor 的 ExternalSetValue，没冻结是否跳过 value binding 的完整 re-evaluation、在 queued value bind 早于/晚于 LocalInput 时依据什么 epoch 比较。

- 原因：只记录 origin_editor_id 不能防止同一 Store commit 触发通用 value binding；当 add draft.set("") 与 composition 中本地 input 交叠时，若 reader 用 stale equality 或同 revision 猜测，就容易重置 caret/composition。Origin token 与 Store entry 的生命周期也未代数绑定。
- 最小修正：把公开 value 投影明确降成 TextEdit::apply_store_revision(session, committed{value, origin, epoch})，而非独立 String binding 将值写回 widget。LocalInput 与 ExternalReplace 均通过同一 editor reducer 排入事务；同 epoch 仅最后 committed logical value 获胜。本地 input 仅发 Store update；对应的 Store commit 通知在 origin session 只做 revision 标记、不复用/不执行 ExternalReplace。外部 command 有 sender/revision policy， composition 中外来替换确定先 commit-last-local 后 replace，navigation/Enter 先让 IME consumed。
- 验收：reducer 覆盖本地 input→Store commit→binding notification，断言对 origin 的 apply 次数为 0、下一次正常外部 set 恰好一次；queued value gen1 在 input gen2 后被拒绝；中文 composition 中外部 replace 的 commit 顺序和 candidate Enter 不 submit 均断言。再加 editor generation 销毁/复用，旧 origin token 不得跳过新 editor 的外部更新。

### active/selected 的样式合同与 aria 仍不是一个统一语义。D4/D8 强制“selected 等视觉条件与对应 aria 状态同一字段派生”，但同时 aria:selected 与 toggle button 的 aria:pressed 未定，D8 又允许动态 StyleDeclar active/pressed rank 高于 selected；此外 class:selected 与既有 class:if-selected 两套名称并存。

- 原因：同一 filter 按钮会出现 class selected=true 而 aria:pressed=false 或 selected CSS hook 直接修改语义的风险，且 selected 的含义在 Todo 里实际是当前 filter，不是 toggle pressed 或可 listbox selected；条件大于 origin 的 D4 也否定了 D8 的“同层 inline 胜过 class”，两个 golden 不可同时满足。
- 最小修正：冻结 Todo 唯一写法：filter button 具有 toggle 语义，视觉 selected 条件与 aria:pressed 同时读取同一个 `filter.get()==entry_filter` 派生状态，绝不允许独立公开 setter；checkbox 只用 checked，active 只 Pressed。选择并删除一套语法（建议沿用已文档化的 class:if-* namespace），不要让 selected 伪态冒充 Tailwind。优先级采用 D4 的全序，并删 D8“同层 inline 胜过 class”或明确 condition_rank 先于 origin 时 inline 永不压过活跃 condition。
- 验收：对 filter 的 selected 条件扫遍 All/Active/Done、pointer press/release，断言视觉条件与 aria:pressed 每项都同源、永不出现 selected-but-not-pressed；aria:selected 用法编译失败。用一组同时含 plain inline、class:selected、active class、hover class 的 golden 比较唯一结果并验证换属性/builder 调用序不变。

### 第一 Gate 实际包含两套不能一起当最小验收的目标：Common prerequisite 要 un-macro core 先过、D11 要可构造 Store 的完整 cargo check，D10 又把真窗 IME/Tab/wheel/AccessKit 集进 Gate A；而 D7、样式白名单、For/RowSet、typed events 尚无实现。当前 todolist 目录也只有 main.rs/README，没有 manifest 或 bootstrap。

- 原因：任何一个缺 OS runner、第三后端 TextEdit、样式 manifest 或未选 TextEdit composition 策略都会把整个 Gate A 判红，即使 core batch/field dependency 可推进；反过来又可能只做 tiny counter 就宣称 Todo gate 已通过。没有清晰的可交付分界，Gate 名无法驱动实现。
- 最小修正：将 Gate A 严格拆成有依赖顺序的两门并在文档改名：A0 为无宏 typed-core+纯 Rust Todo store/reducer（两代、事务、字段查询、RowSet/空态、typed payload、headless focus）；A1 为唯一可运行 Windows Todo slice 的 macros+真实 winit/TextEdit/IME/focus+semantic dump，列明静态按钮、一个 keyed For、一个 Add editor，不含 retained editor、多 editor、CSS-first/3D。真实 IME/AT 不可获得时只标阻塞 A1，不伪称 A0 红；发布 OS 矩阵仍属 Gate C。
- 验收：根 workspace 有独立、可执行 manifest：A0 `cargo test -p zenui-core-contract` 无宏且覆盖第 1–5 项的 reducer/所有权负例；A1 独立 `cargo check/run --manifest-path examples/todolist/Cargo.toml` 从 application-owned Store 启窗并跑 add/toggle/filter/remove、IME Enter、focus transfer。测试报告中 A0/A1/Gate C 分离，缺 runner 不以 headless PASS 或跳过测试冒充通过。

## 被刻意压住的取舍
- 不强制 production handle 只能是单一 u64 Copy ID；暂保留 Clone/Rc PoC 与后续布局自由，只要求目标 TodoRow 使用可 Copy 的 typed handle。
- 不重启默认 retained-visibility 与 dispose+snapshot 的全面框架之争；第一 Todo 可选确定卸载行为，行编辑状态恢复留到显式 opt-in。
- 不要求 Gate A 覆盖三 OS、完整 AccessKit/NVDA 实测或 GPU 能力矩阵，但不得以 headless 记录宣称桌面通过。
- 不扩写完整 Tailwind cascade/variant DSL；只要求消除当前命名、语义及优先级冲突。
- 未列入 TextEdit 多 editor conflict-resolution 和外部 replace 的全部 API 选择；第一门应明确只支持单编辑器的 origin 合约。
