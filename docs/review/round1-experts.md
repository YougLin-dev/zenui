# 第一、二轮专家评审记录

日期：2026-11-18。本文保留评审输入与主控决策依据；不是各位都重新实测，也不是赞同程度投票。最终规则见 [FINAL.md](FINAL.md)，当前实际执行见 [VALIDATION.md](VALIDATION.md)。

> 历史记录：文中的 source 行号、示例缺陷、API 候选和通过建议指向评审当时；其中已经修正者不代表当前目标仍有同一问题。D4 等 condition 胜过 ordinary inline 的规则及 `class:if-*` 已被 FINAL §6 撤销。

## 执行状态

- 初始 12 角色，10 位返回有效结构化报告；A1 输出格式重试失败，E2 无 StructuredOutput。未将主控总结冒充为缺席角色。
- 另行补审 A1/E2；A1 成功，E2 的结果见本文末尾状态或主控验证章。
- 三个交叉组覆盖 A2/E1/X2、A1/E2/S、A3/E3/U 类；主控反方成功返回六个 blocker。
- 本轮主控实际重跑 13 个测试和两个 GPU 程序，结果见 FINAL.md 的验证记录。根目录目标自动发现问题已修正，目标 Todo 无需伪装生产运行时。

## U3：桌面应用开发初学者

不确定性：本轮只读源码和研究文档，未运行工作流、PoC 或桌面测试；因此窗口、键盘、IME、焦点和读屏的真实故障概率均未实测。已将可执行 Store/宏缺失与需实现的交互语义分开标级。研究文档中的验证记录只作为范围证据，没有当作本轮执行结果。

### 证据
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/todolist/main.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/todolist/README.md`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/macro_poc/macro/src/lib.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/macro_poc/runtime/src/lib.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/macro_poc/frontend/tests.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/native_poc/src/lib.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/desktop_poc/src/main.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/native-macro.md`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/runtime-and-rendering.md`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/validation-and-roadmap.md`

### P0 示例没有从 TodoStore 到窗口的可运行闭环，宏实际不能消费示例

- 证据（有源码依据）：examples/todolist/main.rs:38-49 只有 TodoStore handle 型别和 filtered/remaining 等方法调用，没有构造、实现或 main；41 的 #[component] 也没有当前 PoC 定义。真实宏在 examples/macro_poc/macro/src/lib.rs:46-50 仅收 view/text/button，在68-81 仅收 class 和 click/change/input；Todo 的 main/form/input/aria/submit 都会被拒。macro PoC 的例程只是构造 Node 并手工 fire，examples/macro_poc/frontend/tests.rs:21-44；目标示例 README:3 也承认 Store/For/Props 未实现。
- 失败场景：初学者照 README 复制示例准备做增删任务，在 #[component]、main/form/aria:on 处接连得到 probe 级错误；即使绕过宏，也没有可实例化 TodoStore 和执行入口，无法完成最基本的新增任务。目标语法覆盖面积很大，但缺少从空 Store 到一个可见窗口的最短可运行路径。
- 建议：不要继续扩写目标组件 API，先建立一条可运行垂直切片：TodoStore::new/insert/remove/filtered 的最小纯 Rust 实现 + 显式 main/window bootstrap + 当前宏/typed builder 真正支持的 input/button/For。README 首要位置放一条可复制命令和当前支持清单；尚未实现的 on:submit、aria 等用单独未来语法页，不混入新手跟做路径。
- 验收：red：当前宏收 TodoApp 时在 main.rs:71/72/74 等不支持位置明确失败，且无起应用的最小数据路径；green：干净检出按 README 一条 cargo 命令打开窗口，空 Store=M/添加/删除/筛选都可重复执行，TodoStore 和 For 的实现不是未链接伪代码，CI 对此进行编译+真实交互 smoke。

### P1 输入、Enter 与 IME 的组合语义未落到事件数据模型和可验收流程

- 证据（目标设计）：Todo 在 main.rs:71-84 把输入、Enter、button type=submit 和 disabled 写成原生表单；没有 on:key、焦点规则或键盘操作说明。README:14-15 明确规定 IME 候选 Enter 不提交并保留 caret/selection/IME，但这里的 input 事件直接传播 String，submit callback 的 SubmitEvent 未在实现或事件类型中定义。当前 runtime events 只是 (&str, Box<dyn FnMut()>)，examples/macro_poc/runtime/src/lib.rs:103,138-147；实际 native PoC 只测 CJK glyph/词边界，examples/native_poc/src/lib.rs:21-59，不能证明输入行为。
- 失败场景：中文用户在新增框输“买牛奶”，用 Enter 选 IME 候选时既提交了尚未确认的文本、清空 draft，又丢了 composition；或 Enter 根本没反应，Tab 也不能到 Add。对初学者没有任何 API 或调试输出可以判断该由窗口、TextEdit 还是 form 处理。
- 建议：在首个窗口原型就定义最小的 NativeEvent 流：KeyDown/CommitText/Preedit/Submit，并公开 consumed/composing 判断；TextEdit 在 composing 时阻止 submit，确认后本次按键不再冒泡。为 input/button/checkbox 命名默认 Tab 顺序、Space/Enter 激活、disabled 跳焦、初始聚焦规则，并在开发模式打印 window/node/key/IME/caret/consumed。把这些规则写成一个最小中文输入 example，而不是仅在 README 注释中承诺。
- 验收：red：发起 on:input 会写入 draft，但输入框是否可键入、Enter 是否提交、IME 候选 Enter 是否保留完全不决定；green：中文“买牛奶”组合输入全过程 draft 逐阶段可追踪，第一下候选确认 Enter 仅 commit 不新增，第二下 Enter 才新增一次；Tab/Shift+Tab 顺序固定，禁用 Add 不进 Tab 序列， caret/selection 不因外部 draft 回写跳变，实测在 Windows CJK IME（随后 macOS/Linux）通过并附日志。

### P1 筛选、删除和空状态没有可预测的焦点与局部状态归宿

- 证据（有源码依据）：main.rs:112-125 同一个 visible Memo 同时驱动空态 Show 和 For，但没有说明空列表之前是否已有数据；删除/清理在130、170 直接 remove/clear_completed，无焦点接任策略。README:10 承认被筛掉的 UI owner 会销毁，持久焦点与局部输入状态需要 snapshot/restore 或 hide-store；docs/research/native-macro.md:97-100 只说重排保留状态。docs/research/runtime-and-rendering.md:61 仅列“cleanup focus ... references to deleted ViewIds”，没有说明随后焦点去哪、空表如何播报。
- 失败场景：用户正在第三项上操作，切到 Active 或删除该项后该 key 消失；屏幕阅读器仍报已删除按钮，键盘焦点回到窗口顶部/不可见行，之后按 Space 改了别的任务。筛选没有匹配项时“请添加”与“当前筛选暂无”的语义也不清，初学者无法辨空数据与筛选结果。
- 建议：先给 For/Show 一份新手可验证的生命周期规则：移除焦点项后按下一项、上一项、表头的固定优先级转移；筛选暂时隐藏行的局部编辑状态保留或明确丢弃，并与真正删除分开。开发日志发现 focus 转移要输出 old/new TodoId/ViewId 和原因；空态由 template 提供 filtered-empty 与 store-empty 两种文案或示例分支。重复 key 给开发期可视化诊断。
- 验收：red：For 的隐藏/删除会导致焦点节点悬空或读写焦点跳错；green：编辑行内容时切筛选再切回，若有编辑器则 draft/caret 按明确规定保留；删除第三项后焦点到下一项/末项/新增框的规则稳定，Clear completed 不留下隐藏焦点，空匹配始终有可感知空态。键盘与 AccessKit focus event 断言同一 TodoId，重复 key 报出具体值。

### P1 有无障碍名称写法却没有语义 role/live update/真实 AccessKit 链接的闭环

- 证据（目标设计）：Todo 为输入框/main.rs:74、筛选容器88、行复选框151、删除按钮169 写 aria:label，筛选使用 aria:pressed(93)，这是目标语法中少数值得保留的明确命名；但没有 main/title/列表项的 role、状态通知或 AccessKit mapping。实际宏对 aria:label/value/checked 落入 unsupported attribute，examples/macro_poc/macro/src/lib.rs:77-81，目前 Node 只有 tag/styles/events/content，examples/macro_poc/runtime/src/lib.rs:105-110，没有语义输出。docs/research/validation-and-roadmap.md:75-80 把 AccessKit 放在未来 Gate C。
- 失败场景：初学者照着写了 aria:label 便认为任务可被读屏操作，实际语义树尚无这些属性、值/勾选状态和 pressed 的输出；添加/过滤后计数与“No matching tasks”只是文本变化，读屏用户可能什么都听不到，Tab 停在哪里也无法确认。
- 建议：把有限但完整的语义作为输入示例验收，而非最后附加：明确 text 标题、form、toolbar/group、checkbox、list/listitem 的 typed role 与 accessible name/value/state/focus/action；新增、删除、筛选结果提供 live announcement。初次只需 Windows AccessKit+NVDA 冒烟 fixture，并把语义 dump 纳入调试信息。暂时不支持 aria: 时编译诊断应指向属性并说“请用语义组件/尚未实现”，避免让目标示例像已经生效。
- 验收：red：只有 aria 字符串截图/类名，无 operating-system accessibility 输出，不能证明初学者写的名字可见于读屏；green：从空店加一条“买牛奶”后，NVDA 等最小验收环境能读到新增输入名称/值/光标、Add disabled、筛选组、当前筛选 pressed、复选框 checked、删除名称及空态；删除/筛选通知焦点和列表数量变化，语义树上的节点与可见项一致。

**建议**：把下一里程碑缩成“中文输入新增一条任务、Tab 切换、筛选后维持焦点、删除后读屏可继续操作”的可执行最小窗口，不要先扩 Store/长列表/GPU 语法；每个交互缺口输出普通开发者可读的 key/IME/focus/语义事件日志，并给出该场景的 red/green 自动验收。

## E1：十年 Rust 宏与 DX 工程师

不确定性：本轮只读，没有重新运行 cargo test/clippy/trybuild，也未验证 rustc/RA 在实际 IDE 中的增量表现；因此失败场景由现有 PoC 源码、调用点及目标语法推导，不冒充实测。rstml 0.13.1 对组件 attributes、slots、fragment 的具体覆盖边界还需在后续独立 fixture 中验证。diagnostic 子字面量 span、renamed dependency、Builder 信号增量成本均明确待机器实测。

### 证据
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/todolist/main.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/todolist/README.md`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/macro_poc/macro/src/lib.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/macro_poc/runtime/src/lib.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/macro_poc/frontend/tests.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/macro_poc/frontend/tests/compile_fail/unknown_class.stderr`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/macro_poc/frontend/Cargo.toml`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/macro_poc/README.md`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/native-macro.md`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/validation-and-roadmap.md`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/style-system.md`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/desktop_poc/src/lib/project_signals.rs`

### P1 on:* 事件接口按名分派，无法承载 Todo 的 typed 负载

- 证据（有源码依据）：真实 lower 逻辑只识别三个事件名并统一生成 `.on(#event, #expression)`（`E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/macro_poc/macro/src/lib.rs:68-76`），而 runtime `Node::on` 单个 `events: Vec<(&'static str, Box<dyn FnMut()>)>` 存了全部类型，`fire` 只按字符串名找首个（`.../runtime/src/lib.rs:103-110,138-148`）。Todo 却要求 `on:submit |_: SubmitEvent|`、`on:input |value: String|`、无参 click、checkbox `on:change |checked: bool|`（`.../examples/todolist/main.rs:71,77,104,149-150`），事件载荷及受控 input 路由远不是当前签名。已有集成测试仅调用两个 `FnMut()` 并验证 `.fire("click")`（`.../macro_poc/frontend/tests.rs:24,38-41`），没有参数事件 fixture。
- 失败场景：同一控件既有表单提交又有点击、或多个输入源共用名称时，on() 存入不兼容闭包会报泛型/F-N trait 边界错误；即使编译器将来投影成动态 handler，fire(event: &str) 会命中同名首个，payload 丢失或干出错误命令。受控 TextInput 在 IME 输入仅靠 on:input 把 String 推给 signal，也不难与 native selection/change 抢同一路径。
- 建议：先写一套小但封闭的语法协议：每个 on:* 对应精确类型参数（Click Submit(String) 等）或适度 enum/traits，不把 SubmitEvent 与 String 逛路由成派生损耗；为原生 submit、clicks 与 checkbox change 分别生成 typed handler registry/方法，避免同名事件间共用没有类型保障的 Vec。文档明确原生 Form submit 冒泡及 IME composition 完成后的调度，必须提供 default cancel、prevent、once/重复处理语义与对应 compile/run fixture。
- 验收：red：新增 typed two-way input（text/checkbox/radio + input/change 参数）最小测试，on:click 多参数/SubmitEvent 与同名动态书签名 fixture 明确编译结论；green：各事件独立类型化通过，类型不符时约束错误指向具体表达式/事件属性，不依赖克隆适配，也不绕回未捕获错误。

### P1 目标属性/样式 IR 尚未落地，多 class 与 inline 会被直接覆盖

- 证据（有源码依据）：`lower_element` 只在 key == class 或 `on:` 下成功，其余 attribute 一律拒绝（`E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/macro_poc/macro/src/lib.rs:60-82`），intrinsic 列表仅 `view|text|button`（同文件 44-50）。其生成 `Node::styles(Vec<StyleRule>)` 直接替换整个 styles，后次 class 会覆盖前次（`.../runtime/src/lib.rs:122-125`）；StyleRule 只有 property/value/condition 枚举而非 priority/source（同文件 27-31）。与目标语法里 list count view、`aria:`、typed `style:*` Todo 调用（`.../examples/todolist/main.rs:61,74-82,88-104,127-134,146-170`）完全不是可烘干模板。Todo 明确 inline/class 同条件必须按 IR 养优先级（`.../examples/todolist/README.md:16`），不是 builder 调用顺序。
- 失败场景：先把 filter 的 `style:background/color` 集入 builder，再遇到同一个 class 处理，后序 `.styles` 赋值会清掉 inline；重复 `class`/动态 color 同样会出现“按行号覆盖”而非声明优先级。随后无 aria/model semantic bridge，focused/checked 状态只能以颜色表现，无障碍缺值；运行时在 TextEdit/IME 只有受控 value 而 caret 细节未绑定，容易全字段重设。即使扩展 tag/attribute 不含 IR，仍不可用。
- 建议：把 intrinsic 定义做成 typed descriptor（NodeKind、事件、状态、aria/style 字段与 native widget 选项），宏只做 AST→descriptor 常量/绑定。样式先 append/merge declarations，最后由 StyleResolution 按 origin/condition/source 计算，而不是赋值覆盖。明确同一节点多个属性源的 diagnostics；ARIA 字段和 checkbox/input typed widgets 要在 target syntax 内见调用可见签名。
- 验收：red：todolist 的 disabled/checked/aria:label/aria:pressed/style:color 片段在当前宏 fixture 获得属性和 span 对应 error（当前不支持）；green：同类型 builder/IR 驱动这些属性及 label、placeholder、style targets、dynamic binding，错误不一致时再有诊断，属性顺序不决定覆盖值；测样式条件优先级/静态初值/动态更新。

### P1 生成代码绝对依赖 runtime 原名，重命名 facade 即失效

- 证据（有源码依据）：生成的初始化直接写 `::zenui_macro_runtime::Node::view`，content 与 StyleRule 也硬编码同一 crate（`E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/macro_poc/macro/src/lib.rs:111-116,126-128,193-195,234-238`）。公开 facade 目前只是 `pub use zenui_macro_processor::view; pub use zenui_macro_runtime as runtime;`（`.../frontend/src/lib.rs:1-3`），不存在 `__private` 或用来稳定侦接的入口。现有 fixture 依赖可以原名找到 runtime（`.../macro_poc/Cargo.toml`、`.../frontend/Cargo.toml:7-9`）；README 也明确说 renamed dependency 尚未解决而非已验证（`.../macro_poc/README.md:27`）。
- 失败场景：真实 app 因 workspace 名称冲突或同时承接不同版本而 `zenui = { package="zenui", version=... }` 重命名为 `zen` 后，用户只 `use zen::prelude::*` 写 view!，resolver 找不到由 processor 写掉的内部 crate 名，正确 RSX 仍 compile error。该类破坏波及普通消费者，无法用 IDE 隐藏。
- 建议：让处理器返回由公开 facade root 到私有支持模块 codegen 的路径（例如 `zen::__private::{Node,Content}`，processor  facade 路径用稳定机制）或借助 cargo package resolution；不要让 output 定位 processor peer crate。文档承诺用户的 primitive builder 都从同一 facade 入口重导出，避免用户手动知道内部 runtime 包名；测试独立于 workspace 同名导入的三个 fixture：原始名、改名、加同包不同版本。
- 验收：red：第三方 crate fixture 在 Cargo.toml 将公开 facade 改名为 `renamed_zenui`（且不带原名直接依赖），只导入 facade 即触发：当前 macro 展开引用 `::zenui_macro_runtime` 会编译失败；green：两者改名后无需额外 `extern crate as` 或手动 pinned import 即编译，2 个不同名称 consumer workspace+trybuild 通过；用 `cargo package` 风格测试验证 facade 源文件路径与预处理器边界一致。

### P0 以 closure 形状猜语义与 move 捕获规范不闭环，Todo 已经踩到所有权边界

- 证据（有源码依据）：Todo 以 `TodoRow` 同时放 Copy handle 与 owned `String` 并 derive Copy（`E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/todolist/main.rs:25-31`），但函数内又 `let title=todo.title` 后多字段使用（`:139-143,151,165,169`）。`For` 同模板要求 `|todo: &TodoRow|` key 和 `move |todo: TodoRow|` child callback、对数组/source 混用 `each={FILTERS}` 和 `move || visible.get()` (`:89-90,118-123`)；另 `add_todo` 是捕获 Copy handle 的 closure 本体，move 进 submit 后已无同一份再供 Add button 等复用（`:52-58,71-85`）。native-macro 又声明数据位 `{move || expr}` 是唯一绑定、事件是普通 FnOnce/closure（`.../docs/research/native-macro.md:44-59,122-127`），但没给可形成 trait bound 的 props/For/child factory/slots API。
- 失败场景：只要实现是函数转 builder，单凭 Copy 信号无法规避 move 语义：submit closure 持有真正命令 closure，事件系统闭包值若 FnOnce 再尝试第二次使用就 E0382；TodoRow 带 String 连 Copy derive 本身都难兑现，未来 row 重构为 owned 值后 move 捕获的 filter/title 责备会在多个绑定与事件间传播。For key 引用和 renderer row move 之间错用借用还涉及 owner/lifetime，光 `move` 或宏魔法都无法自动 patch。
- 建议：规范 4 个位置而不是统一 closure 元素：一次性 props、动态（getter）、event callback、For renderer 中数据共享与 owner 传递。不要假定 `FnOnce` 或 kind 可凭语法猜测；给 binding、listener、child factory 分别显式 wrapper/newtype trait，支持 FnMut/Fn 的条目回调。Todo 示例可让 `add_todo` 复制 Copy store handle 每次构造命令，而非复用 closure 本体；每组件展示派生的 builder 等价代码，让 RA/clippy 诊断走 Rust 字段/参数。对 title 等只读标题将非 Copy 字段作为 initial prop 移到一次节点，借读的 signal 只用于有动态更新需要字段；若需多字段响应 title，明确 Store 字段 signal 与 cloning 成本，不用 derive Copy 神奇地平息所有 move。
- 验收：red：现在 Todo 里 `For each={FILTERS}`、复制 callback 后 add_todo 再被 submit，以及 filter/entry_filter 三 closure 和同一 todo.title 三字段进入例子，列每个命名冲突及重复 Copy 字段；green：props/closure/param receiver 合法、ReadOnly title 只转移一次，IDE rename 不变量；提供以 Signal/Store handle 为唯一捕获的等价 builder 示例；CI 有 RA diagnostics/rename+宏 expansion 性能测量清单，而非只靠友好文案。

### P2 诊断仍停留整串 class + byte offset，缺少可跳转到具体错误与处理建议的 span

- 证据（有源码依据）：`compile_class_literal` 函数输入从 `expression(attribute)?.span()` 取得整体字面量 span，resolver 返回 offset 后被压入普通字符串 `format!("unsupported ... byte {}")` 并 `syn::Error::new(span, ...)`（`E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/macro_poc/macro/src/lib.rs:204-213`）。现有预期诊断正是一个覆盖 `"flex p--2"` 的 9 字符 intact attribute（`.../frontend/tests/compile_fail/unknown_class.stderr:1-5`），unknown 错误同时没有 code/help。目标 style 文档却把字段颜色/separator 和 source maps 说成 parser 供给 IDE/trybuild/builder 共用（`.../docs/research/style-system.md:139-158`），而 README 承认精确 UTF-8 内部诊断尚未实现（`.../macro_poc/README.md:28`）。目前 crate 仅编译 class 时经受 runtime 的 per-token evaluator，普通 root 多节点/关联表达式的错误更直接挂靠 call_site（`.../macro/src/lib.rs:27-39`）。
- 失败场景：设计稿一个 class 写十几二十个 utility，在第三处 `focus-visible:ring-2` 拼错时，用户只能看到整串波浪线并计算 byte，若 byte 因 unicode 或转义不同则不指到任何具体 source；同一宏错误多数在编译器膨胀，macro expansion/code action 导航进入 `view!` 而不是输入。宏阶段增加 For/Props/intrinsic 后，这类黑框诊断会把 ownership 错化成模板大段的 syn error，用户无法区分 Zenui 与 Rust。
- 建议：callee 返回包含 source literal/原始 offset/property/suggestion 的结构化 ClassError，processor 保留每个 Literal 的 subspan/source map（注明 proc_macro span API 可用条件下 fallback 而非 offset 自动定位）；将 parser 错误码迁移为稳定 `syn::Error::combine` 多错反馈。把 vendor raw CSS scanner 与 macro 的表达字段错误输出对接，生成 diagnostics 时间序文档与 IDE/check diagnostics fixture，明确“无法精确 span 时退化为 literal 整体”的 fallback，不伪造比源更细的错误。
- 验收：red：当前错误基准可见整段 class 字面量而非 p--2；diagnostic fixture 再加同一行两个 class attribute、字符串转义和 unicode 前缀、unknown class 变体；green：错误指向具体子 token/修改建议且 host/caller span 合法，同错误稳定排序；RA 可在 token 源范围定位/跳转，builder 共享 parser 同一错误码。

**建议**：在扩展任何 UI 功能前，先落定一页可编译的“核心类型/所有权语法契约”：一次性 Props vs 动态 binding vs typed event vs For renderer 各自独立 trait，明确 Signal/Store Copy handle 的 Arena 生命周期，并用 TodoList 做 contract test。然后用公开 facade 的 `__private` 路径生成代码、独立 package trybuild 验证 renamed dependency，生成 typed StyleIR 字段而非 `.styles` 覆盖。当前最小宏只能证实 rstml 转写，那些跨 ABI/事件/props/For 的目标 API 仍不可当验证完成。

## E3：十年桌面应用与质量工程师

不确定性：本轮未运行任何测试程序，也没有修改文件；所有结论来自逐行读源码。validation-and-roadmap.md 的历史实测记录仅作项目自述，未据以把任何发现标成“已实测”。因伤仅为源码分析，跨平台故障场景和性能预算都需后续实测标定；Storage trait 的测试 memory stub 不等同真实磁盘/SSD，性能 acceptance 应在真实 app artifact 中执行。对 docs 与 PoC/目标语法的差距只描述为“易被掩盖/测试未能证明”，不把 PoC minimal inspection records 当作完整 runtime。

### 证据
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/todolist/main.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/todolist/README.md`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/desktop_poc/src/lib/project_signals.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/macro_poc/frontend/tests.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/macro_poc/runtime/src/lib.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/native_poc/Cargo.toml`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/native_poc/src/lib.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/native_poc/src/gpu.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/native-macro.md`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/runtime-and-rendering.md`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/validation-and-roadmap.md`

### P1 失效 handle 静默读写破坏异步错误处理

- 证据（有源码依据）：E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/desktop_poc/src/lib/project_signals.rs:47-55 中 set 在失效时返回 ()、且无 generation；lines 27-45 read 最后总是执行取值；lines 319-335 测试只验证 updates 为空，就接受失效写不报错。docs/research/native-macro.md:68 目标明确“销毁后返回明确 handle invalid 错误”，差距未纳入测试。
- 失败场景：resource/async completion 比 For/Show 销毁晚返回，仍然持着 Copy handle；处理函数读旧值，再写回失败也没人知道，上层可能提示“保存成功”。当前测试只断言无 patch，不等同取消被验证或业务错误可观察。
- 建议：在 reactive API 规定 invalid read/write typed Error；production Store/Resource 增加 OwnerId + generation，不把 Rc 存活当有效。dispose 时依赖与异步任务同时取消，保存/删除命令返回是否真正执行的结果；开发模式诊断携带来源 span/owner。当前 PoC 应有一个带 TODO 的负向测试，而不是沉默的合法化。
- 验收：Red：在任何分支已删、异步回调过期、handle 失效时 binding/patch 不得推进 partial 状态；传播 Result 或开发诊断并取消过期 commit，最后一版本 halt scope。Green：在 Owner drop 前后准时交换 filter/draft/source，效果只作用存活 scope，1000 次 interleave 没有 panic、静默成功和无申请异步更新；测试进入 Gate A。

### P1 Todo 的 For 过滤未规定焦点、编辑和滚动生存规则

- 证据（目标设计）：examples/todolist/README.md:10 明确过滤可释 UI scope、持久焦点保留需 snapshot/恢复或 hide-store；main.rs:118-125 按 visible.get 动态删除 <For> 行，lines 149-150 空格切 checked 会令 completed 离开 Active 筛。docs/research/runtime-and-rendering.md:49、61 仅描述愿景，没有可测转移优先级。
- 失败场景：键盘用户在 Active list 的 checkbox 按 Space，行立即因过滤消失；焦点、selection/IME composition 若跟 owner 一并释放，第二次 Space 随机落根节点/另一行，scroll anchor 也突跳，NVDA 读出另一项。
- 建议：把 focus intent 和 key 绑定 update transaction：row removed 选 direction-aware next visible row，无 row 回 filter/add，reorder 保留 ViewId/caret；真实数据删除作废 restore，UI filtered 退出保留局部 editor snapshot 有 TTL/owner，异步 generation 验证后才 restore。定义 scroll reveal/anchor、pointer-capture、IME composition commit/cancel 规则以及 AccessKit action 回调。
- 验收：Red：Active 中 Space 修改 focused checkbox 后 focused/caret 是 deleted/none；filtering 后 off-screen 前 focuser 没有逻辑目标；restore 数据时焦点恢复但 IME composition 被提交的问题能重现。Green：焦点掉到下一个可见行（没有则 filter 控件），assistive 输出 ViewId/focus 与 render 同步；切换 Filter/undo/reorder 保持业务 key、局部 editor/caret 规则，IME commit 不被 reset。单元 + 真窗口集成进入 Gate A 而非后置 C。

### P1 桌面验证与性能回归门缺少真实驱动和可执行预算

- 证据（有源码依据）：examples/native_poc/src/lib.rs:1 明示不是 working window，Cargo.toml:9-17 无 winit/accesskit，gpu.rs:128-134 只 headless 且明确排除 window/IME/a11y。macro_poc/frontend/tests.rs:21-44 只是手工 evaluate_content() 和 fire(click)。docs/research/validation-and-roadmap.md:47-55、75-88 认可 Windows 单机和 Gate C/D 缺口，但 88 没有阈值或数据格式/测试入口。
- 失败场景：发布前 macOS 的 Cmd 键、Wayland scroll 缩放、Windows 中文 IME candidate Enter、DPI 变化中的 caret/a11y 坐标可错而现有 tests 全绿；把 Gate D 性能推到 3D 阶段还会导致 10k/100k 行 update、mount/unmount 或 idle 空转问题只有客户遇到。
- 建议：把最小 Todo 垂直 slice 从 Gate C 前进到 Gate A 早期，做可编程 native-event driver（key text-commit、IME lifecycle、wheel/capture、DPI）+ AccessKit snapshot/state action + geometry/hit 一致性断言。CI 分纯单元、CPU deterministic layout/pixel snapshot、操作系统 native integration、实机 GPU；跑 Windows/macOS/Linux（Wayland/X11 分流）并固定字体、版本、seed。性能矩阵现在写 workload 与 budget：单字段 patch 数、1000→10000/100000 list、layout/text/memory、frame p95/p99、idle wakeups、heap 回到 baseline；refs/scene key failure 的可重复 debug trace。
- 验收：Red：仅跑库测试和 readback 就能宣称窗口感官/行为通过；当前源码没有 window/IME/scroll/a11y 集成断言。Green：Win+Mac+Linux 真实 window driver 通过，输入/scroll/focus/accessibility 与 DPI/resize 通过矩阵；固定字体与容差的 CPU diff 以像素/generation 防止遮蔽 bug，GPU image/latency/idle/内存 budget 以可下载 JSON+图块为门槛；IME、窗口 close/resource-failure 标清 automated/manual 结果，失败 job 阻断发布。文档历史命令不算实测通过。

### P1 Todo 目标缺少持久化、异步乱序及失败恢复验收

- 证据（目标设计）：examples/todolist/README.md:24 完全省略应用持久化/文件对话框以及 async resource generation；示例 main.rs:52-58 insert、149-150 completed.set、167-171 remove 都是无结果命令。docs/research/runtime-and-rendering.md:36、native-macro.md:167-185 只有资源 generation 设计，没有 durable command/read profile。
- 失败场景：启动 load 老快照时用户新增一条，旧 async load 姗姗来迟再整表 apply 会吞掉新增；程序 close/刷新恰逢 save，退出丢操作。损坏 JSON/权限拒绝/磁盘满若示例层没有 Result policy，应用工程师只能用 unwrap、伪造成功或丢失数据。
- 建议：补一条声明【目标设计】但可独立单测的 TodoRepository：snapshot(version, rows) + load Migrations + save atomic（temp/flush/rename，平台 capability 明示）+ Result diagnostics。所有 load/save 带 owner、generation；新本地 mutation 触发新的 epoch，旧 load 不覆盖，旧 save completion 不发布成功；close/dirty 等策略可以手动 flush/cancel 但必须明示。分离 last-known-good、recovery、unsupported-version，不把 stale row handles 恢复成新行。
- 验收：Red：load generation 1 被 generation 2 覆盖，或构造中途退出造成 loss/option unwrap/path permission panic，当前没有示例或集成断言能做 gate。Green：cancel/stale load 不再覆盖本地新增；restart 精确 roundtrip Unicode/完成的行、filter/draft policy 明确；migration v1/v2、corrupt/truncated file、permission/disk full、worker panic、重覆删除都有 Result + recoverable UI label，且无 raw backend error。stored identity/caret 恢复通过 generation 校验，临时目录故障注入 + 真文件 manual matrix。

### P2 TodoRow 的目标 Copy 模型存在内部类型矛盾，应用契约无法成为回归夹具

- 证据（目标设计）：examples/todolist/main.rs:26-31 对 TodoRow derive Copy、但 title: String 非法；lines 141-143 又从 todo move 出 title。README.md:7-10 文档的一面是 Copy row handle 稳定 identity，另一面是静态 read-only String 配置值，无法作为可作为公共契约类型 fixture 编译。根 Cargo.toml:6 dependencies 为空，故主 crate 没有 contract check。
- 失败场景：框架实现者照示例把 TodoRow 做成真的 Copy 或把 String handle 再次伪造 Clone copy，导致生成代码无法通过 rustc；或为了绕开编译把 title clone 到每行 projection，形成每次 filter O(n) 复制、Store 删除后 title 仍存活的分离语义，这不是单纯目前示例不可跑。
- 建议：在 Gate A 做一个不伪造运行引擎的 public app-facing contract crate/fixture 把 compile contract 与 command semantics 区分。TodoRow 若必须 Copy，title 要改成 Store 内不可变 text handle（或明确共享 Id），get/with 仍然 owner+generation 校验；title 若可 String 就不 derive Copy 并清楚 clone/snaphot 代价。运行示例 future 自动纳入 workspace CI：compile API + trybuild invalid stale read + clone two handles shares id/model，drop store 后 access typed error。
- 验收：Red：编译既显示 E0204 derive Copy String，也显示 move out Copy 类型，且主 crate 只靠 println 无任何示例 check。Green：row id/title read handle/completed 都可复制但不复制数据；target example cargo check + add/toggle/filter/remove BDD 测试通过，删除 commit 后 copied handle 明确 invalid 或 None，不会指向 other TodoId generation。main.rs 的文件头继续保留真实 runtime capability 的 statement，不返回“可编译即功能完成”。

**建议**：把“可驱动、可断言的最小 Todo 真实窗口”作为 Gate A 的硬门，而不是等 Gate C/D：一次 Space 完成 Active 行后行被过滤，焦点与 accessible node 转向可预期目标，stale async/handle 必须可见失败，持久化必须可恢复。这个竖切片先通过输入、focus/IME、For/scroll、owner invalid error 和 service 失败注入，再在 Win/Mac/Linux 扩 pixel/performance gate；宏 surface 或 Tailwind coverage 增长不等于应用可交付。

## U1：Rust 初学者；只评是否能循序渐进看懂、照做和根据错误修复，不承担底层架构审批。

不确定性：本轮全程只读，没有执行 cargo、Todo 或 trybuild，因此没有“本次已实测”项；签入的 stderr 和 validation-and-roadmap.md 的既往通过记录只当源码/文档证据，不冒称本次运行。Todo 的 Signal/Memo/Store/For/props/窗口生命周期均未在这些文件中实现，最终句柄命名、编译行为、运行时交互和新诊断文案都待真实实现后验证。未评估 GPU、布局、IME、无障碍及底层 Owner 设计。

### 证据
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\Cargo.toml`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\todolist\main.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\todolist\README.md`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\macro_poc\README.md`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\macro_poc\frontend\tests.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\desktop_poc\src\lib\project_signals.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\macro_poc\frontend\tests\compile_fail.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\macro_poc\frontend\tests\compile_fail\html_element.stderr`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\macro_poc\frontend\tests\compile_fail\unknown_class.stderr`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\macro_poc\frontend\tests\compile_fail\script_event.stderr`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\docs\research\native-macro.md`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\docs\research\validation-and-roadmap.md`

### P1 先修 Copy + String 的基础所有权矛盾

- 证据（有源码依据）：E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\todolist\main.rs:26-31 给含 title: String 的 TodoRow 派生 Clone, Copy；普通 String 不能 Copy，这不是未来宏能改变的规则。E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\todolist\README.md:7-9 又强调只复制小 handle、String 要 with/clone，更容易让初学者以为 Rust 的 String 在这里也自动变成句柄。此处判断依据是源码和标准 Rust 类型规则，不是本次编译结果。
- 失败场景：初学者照抄 TodoRow 定义到自己的 struct，一编译就遇到 String: Copy 不满足；他会怀疑 derive、trait bound 或 zenui 宏，而不是“字段本身是 String”，第一课就卡住。
- 建议：让轻量行模型名实一致：title 改为框架承诺的可 Copy 只读文本句柄，读取时明确使用 with；或者取消 TodoRow 的 Copy 并同步改 For 契约。二选一，不能保留 String 却宣称整行可复制。用“行李牌/票据只复制号码，不复制货物”解释 Signal 与 String 的区别。
- 验收：red：保留 main.rs:26-30 的 String 字段并派生 Copy，最小编译检查必须失败。green：在目标 Todo 示例的 compile-pass/doctest 中断言 TodoRow: Copy 能成立，整例 cargo check 通过；教程同时展示“复制 TodoRow 只复制小句柄”，并有测试证明修改标题数据不需要克隆整行文本。

### P1 缺真正可照做的入门阶梯，而不是再补一篇总说明

- 证据（有源码依据）：E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\todolist\main.rs:1-7 明说不能运行；40-58 的 TodoApp 直接接收一个已经存在的 Store，没有启动页展示 Store 如何创建、应用如何挂到窗口。E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\Cargo.toml:1-7 根清单无依赖；E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\macro_poc\README.md:3-11 又限定探针只支持少量 view/text/button/class/on:click，且 25 行说 Rc<Cell> 只是测试见证。初学者没有 20 行可运行起点。
- 失败场景：只学过变量/struct/trait 的读者把 main.rs 当 quickstart，第 41 行就会问“todos 从哪来、main 在哪、为什么点不了”；复制文件后没有一步能成功，便无法分辨是自己写错还是示例本来就未实现。
- 建议：Todo 保留为最后的目标语法预览，另做四课：静态 view/text（只讲静态界面）→ 单 Signal 计数器（只讲 get/set 与更新）→ 文本输入（再讲 with、move 和受控值）→ TodoList（最后才讲 Store、Memo、For）。第一课就给完整包/构建入口和成功输出；每课只引入一个新框架概念，暂不要求读者看 Rc/RefCell/Weak 的 PoC。
- 验收：red：当前只有不可编译的完整 Todo 预览，现有宏探针也不能替代第一课。green：四个依次递进的小例子分别有清晰构建入口，逐一通过其文档所列 cargo test/run；第一课不要求生命周期/RefCell/move capture，智能指针只作为内部实现；输入课可以演示从空文本到非空文本再清空，Todo 课才有 Store/For，并明确每课只新增的概念清单。

### P1 先拆清视图五段和闭包时机，再引入两种 For 嵌套写法

- 证据（有源码依据）：逐段白话应是：E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\todolist\main.rs:15-38 先定义三种筛选、Todo 编号、每行数据和整套数据服务；42-58 建草稿/筛选状态，派生可见行及计数，add_todo 负责去空格、插入、清空；60-87 画标题、计数和输入表单；88-109 用固定三个枚举项生成筛选按钮；112-135 分别处理空列表、可见 Todo 行、清除已完成；139-176 子组件只负责一行的勾选、文字样式和删除。难点不是业务，而是 main.rs:47-58、65、71-107、118-125 同时要求理解 move capture、get 与 with(Clone::clone)、Memo、两种 For、回调参数和 key。
- 失败场景：初学者看到同一文件里的 key={|entry| entry.0}、child={move |(entry_filter,label)| ...}、key={|todo:&TodoRow|...}、child={move |todo:TodoRow|...}，很难判断竖线、move、引用各起什么作用；照改时容易删掉 move 或给每份数据乱 clone，却仍不知道哪一步只是“事件函数”而非“自动刷新函数”。
- 建议：基础 Todo 版本先直接写三个筛选按钮，不在入门例里用 For 生成固定三按钮，只保留真正列表那一个 For。把 value、disabled、点击处理分别命名并在旁注释“只执行一次/每次值变时计算/用户点击才执行”；将嵌套闭包工厂、Memo 共享订户比较留到进阶版。README 只解释这五段，不在同页要求先吸收 Owner disposal、generation、hide-store、IME 和 StyleIR。
- 验收：red：当前读者只有读懂 main.rs:42-125 所有闭包形态后才能理解一个 Todo。green：基础版每个区块都能用不超过三句话说明“数据在哪里、何时读、何时执行”，源码逐段注释与之对应；宏测试继续断言 mount 不执行事件、显式求值只读一次，新增行为测试覆盖输入→新增→清空草稿、勾选不重建标题、筛选后删除正确行。

### P2 错误测试只防非法 Web 写法，尚未覆盖初学者最常犯的框架错误

- 证据（有源码依据）：E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\macro_poc\frontend\tests\compile_fail.rs:1-6 只通配现有三个 case；html_element.stderr:1-6 和 script_event.stderr:1-6 是已签入的预期诊断快照，只覆盖错误标签/脚本字符串。unknown_class.stderr:1-6 虽有 token 与 byte 5，但高亮整串且没有修复建议；E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\macro_poc\README.md:27-28 也承认字面量内精确 span 尚待实现。E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\docs\research\native-macro.md:46-68；84-99 提到 move 和 For key 这些核心约定，却未见相应 compile-fail。新错误最终长什么样待验证。
- 失败场景：初学者漏写动态绑定外的 move、For 不写 key，或把 class 拼成一个接近合法值时，不能从现有测试证据确认 Zenui 会指出对应属性和给出一行修法；他可能只拿到普通 Rust 捕获错误或整个 class 字符串报错，无法自己修。
- 建议：除现有三项外，首批 pin 三类初学者 fixture：绑定闭包漏 move、For 漏 key、class 近邻拼写错误。诊断必须包含“这里为何错”和 help，不只重复 token；class 要精确到具体 token/UTF-8 span。String 的 get/with 误用则再单独实测 rustc 输出，若原始 E0277 不足，应在教程紧挨示例解释，不虚称宏能识别所有类型错误。
- 验收：red：当前 compile-fail 集合只有 html_element 与 script_event，证明不了核心教学错误可修复。green：trybuild 对 binding_without_move、for_without_key、unknown_class_near_miss 均有稳定 stderr；缺失 move 给“这是持续跟随值的绑定闭包，把外部状态搬进闭包”及修复示例，缺 key 指向 For，class 拼写指出具体 token 和相近合法值；测试入口失败时 stderr diff 必须变红。

**建议**：把“可编译的轻量 TodoRow + 真正可运行的小 Counter/Input 阶梯 + 初学者错误 fixture”设为 Gate A 的教学子门槛，再把完整 Todo 当进阶综合例。先让新手每一步都有可验证成功，再讲 Memo/For/Owner，而不是靠更长的总说明补足体验。

## A2：响应式系统与 Rust 所有权架构师（第一轮只读评审）

不确定性：本次仅静态阅读，没有修改文件、启动工作流或重跑编译/测试，因此没有“已实测”条目；validation-and-roadmap.md 中的通过记录只作为仓库自述，不算本次复验。根 Cargo.toml 无依赖，Todo 明确不可运行。前两条及第五条是对当前 desktop_poc 公共 API 的源码级判断，不能直接外推成未来 production arena 的漏洞；Memo、Store、For、Resource 的缺失则按 Gate A 设计阻断项评审。未审计 GPU、布局、样式实现，也不据这些探针推断无神安全内存问题、跨线程数据竞争或性能量级。

### 证据
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/desktop_poc/src/lib/project_signals.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/todolist/main.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/todolist/README.md`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/runtime-and-rendering.md`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/native-macro.md`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/validation-and-roadmap.md`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/CONCLUSION.md`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/stack-decisions.md`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/Cargo.toml`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/desktop_poc/src/lib.rs`

### P1 Copy Signal 在销毁后仍能读旧值、写入静默丢失

- 证据（有源码依据）：E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/desktop_poc/src/lib/project_signals.rs:27-55：`with` 只把有效性用于决定是否登记依赖，随后仍在第44行读取 `self.value`；`set` 在失效时第48-50行直接 return。E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/native-macro.md:68 要求销毁后明确 invalid 错误；E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/todolist/README.md:13 要求 generation 与失效读写规则，反对 stale handle 静默操作。
- 失败场景：Owner dispose/drop 后，事件闭包或资源回调仍持有复制出的 Signal；`with` 返回已冻结的旧业务值，`set(true)` 却静默无效。Todo 行删除后，旧复选框句柄可能继续展示旧 completed，命令调用方却无法区分成功与失效；未来 arena 复用 slot 时若不补世代，目标文档所担心的误读写新行也无可执行防线。
- 建议：公共 handle 使用 `(runtime/owner identity, typed slot, generation)`，把有效性和值存取统一放进 arena 的 checked access；提供 `try_with/try_set -> Result<_, InvalidHandle>`，错误含 kind/index/generation/owner。若保留便于绑定的 infallible 方法，只能采用明确的诊断/panic 策略，不得回退旧值或无声 no-op；Store remove 独立定义缺失 key 的 Result/diagnostic。
- 验收：RED：创建 signal，保留 handle，dispose/drop Owner 后 `try_with`/`try_set`；当前 `try_with` 尚可读到旧值，`set` 返回单位且静默忽略。GREEN：两者均返回 `InvalidHandle`，保留的原值不再可读、不可写，slot 复用后旧 handle 仍失败；Store 对不存在/已删除 key 的 remove 同样可断言，且在 UI owner 与 row-model owner 各自 dispose 时有不同、明确的错误域。

### P1 flush 可被计算期自写永久驱动，缺少 phase 与循环故障边界

- 证据（有源码依据）：E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/desktop_poc/src/lib/project_signals.rs:128-160：flush 是无预算 loop，先弹出 observer、detach 后直接执行任意 sink；E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/desktop_poc/src/lib/project_signals.rs:47-55 允许 sink 内同步 set 并立即 invalidate，没有 phase/running observer/重入检查；E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/runtime-and-rendering.md:35,52 已要求检查重入及 Overrun depth，但 PoC 尚无此保护。
- 失败场景：一个公开 bind 闭包先 `count.with(|v| *v)`，再 `count.set(n+1)`。第一次读取已重新建立 count→自身 sink 边，set 把刚弹出的 sink 放回 pending；flush 每轮都产生下一轮，bind 的初次 flush 即可能永久占用 UI 线程。若在 `with` 的读借用尚存活时 set，还会直接暴露 RefCell panic，而非约定的阶段诊断。
- 建议：引入显式 `Outside/Mark/Compute/Commit/Disposing` phase；字段 RenderBinding 的 Compute 段禁止用户写入，事件/异步写入只从 Outside 或命令队列进入。Memo 内部标脏与提交走独立受控路径，保留 running observer 并检测自环；运行时加上确定性 evaluation budget，报告 `WriteDuringCompute` 或 `ReactiveCycle {nodes}`，并用 guard 保证错误/panic 后 phase、stack 与临时队列可恢复。
- 验收：RED：子进程/超时保护下安装“读 count 后 count.set(count+1)”的 bind，当前 bind→flush 不返回。GREEN：同步在重入点或 flush 返回结构化 `WriteDuringCompute`，执行次数有限，已写值按所选合同拒绝或回滚，诊断不再只是 RefCell BorrowMutError；另加 a↔b Memo cycle、长链更新、panic 后 phase 复位测试，均不会死循环或污染下一帧。

### P1 嵌套 batch 泄露中间状态，Memo/拓扑稳定化尚无可执行合同

- 证据（有源码依据）：E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/desktop_poc/src/lib/project_signals.rs:163-169：batch 无嵌套深度，任何内层 batch 都直接 flush；E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/desktop_poc/src/lib/project_signals.rs:58-65 只有 source→sink 边，没有 Memo 层或拓扑状态。对照 E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/runtime-and-rendering.md:30,34 的 Memo equality、拓扑提交目标；E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui docs/research/validation-and-roadmap.md:50 明确 Memo fairness 尚未实现。
- 失败场景：Todo 的应用命令外层 batch 调用 Store 内部 batch：Store 已完成一半时内层 flush 计算 visible/count/binding，外层随后清 draft 或继续修改筛选源，再次 flush。订阅者可观察到中间集合和计数，重复求值；未来加入 Memo 后，diamond 的左右分支还可能分别推动下游，产生 glitch，而不只是多做一次计算。
- 建议：batch 改为 RAII 维护的 depth transaction，最外层离开时才进入 Mark→拓扑 Memo stabilize→Binding commit；嵌套批不得消费 pending，显式 flush 在批内按同一事务规则处理。Memo 用 dirty/version 与拓扑层级，沿 reverse dependency 标脏，按最后 source value 求值；equality 仅阻断下游无效通知，不让 Effect 自己猜去重。所有结构清理须在对应 view generation 校验后提交。
- 验收：RED：初始 a=b=false，绑定计算 a&&b；外层 batch 设 a=true，内层 batch 设 b=true，随后外层设 b=false。当前内层产一次 true、外层再产一次 false，违背一组写入只稳定一次。GREEN：内层不提交且返回空/延迟 token，外层只计算一次并提交最后值 false；显式嵌套 flush 按合同延后或报错。另以 diamond、1000 层 Memo 链、同 source 多次写验证每轮稳定后每个 Memo/binding 的必要求值次数，a 的中间态不可达 binding。

### P1 For 行、模型与异步任务必须分离三代生命周期并迟到拒写

- 证据（目标设计）：E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/todolist/main.rs:118-125 仅以 TodoRow.id 建 For key，第149-170行闭包保留 Copy 的 completed 与裸 TodoId；E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/todolist/README.md:9-13 区分过滤释放 UI owner与实际删除 row model，并承认 generation 待 runtime 定义。E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/desktop_poc/src/lib/project_signals.rs:86-114 只有单 Owner 的 bool liveness 与递增 usize，没有子 Owner/世代/取消注册；E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/runtime-and-rendering.md:36,58 要求 SyncSignal 与 generation 验证，E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/validation-and-roadmap.md:50 承认 Store/For/resource generation 未实现。
- 失败场景：id=7 的行 checkbox 点击已进入原生命令队列，随后 filter 切换卸载该行；若命令只验证仍存活的 Store row，不可见旧 UI 动作仍会改 completed。id=7 重新出现时 UI scope 已换代，真正删除后旧 Resource/worker 结果又可能回写；若模型 slot 复用，裸 TodoId 或 Copy Signal 可能指向新行。Simple key map 本身无法区分 reorder、filter out、actual delete、stale async completion。
- 建议：把状态机写成可测接口：For `Mounted{key,view_gen,row_gen,owner}` 在 reorder 完全不变，filter out 转 `UiDetached`（dispose UI owner/事件/焦点，保留 row model），重现创建新 view_gen；实际删除转 `ModelDisposed` 并使 row signals 失效。命令同时校验 ViewId+view_gen 与模型 slot+row_gen；Resource 用 `Idle→Pending{owner,key,generation}→Ready/Cancelled/Disposed`，只在 UI 线程完成原子 compare-and-commit。普通 UI handle 保持 !Send/!Sync，SyncSignal/worker bridge 仅发送 owned Send 数据和 ticket，不直接跨线程访问 arena。
- 验收：RED：现有 PoC 无法表达下列状态矩阵，任何测试若仅对 Copy id 调 remove/set，都无法证明身份安全。GREEN：(1) A/B reorder 后 owner/root/focus 对象身份不变；(2) filter out/in 后 UI gen1 的迟到 click 被拒，row model 仍在，UI gen2 可正常操作；(3) model delete 后老 completed 的 try_get/try_set 失败，新 slot generation 不受旧回调影响；(4) key 改变和 owner dispose 都使旧 worker completion 不提交；(5) 重复 key 在修改树前报错；上述序列循环 10^4 后 graph、model、focus、timer、resource 计数回到基线。

### P2 动态依赖解绑留下空订阅桶，造成长命 Owner 内存与指标漂移

- 证据（有源码依据）：E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/desktop_poc/src/lib/project_signals.rs:75-83：detach 只从 BTreeSet 删除 observer，不在集合为空时移除 source entry；E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/desktop_poc/src/lib/project_signals.rs:183-185 的 subscriber_count 直接返回 map.len。现有 E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/desktop_poc/src/lib/project_signals.rs:258-291 仅验证 inactive source 不触发 patch，没有检查桶数量或长期增长。
- 失败场景：长命根 Owner 中条件绑定依次接触大量动态 signal。每次切走的 source 都留下空 BTreeSet，后续 set 虽不再错误唤醒 sink，但 subscribers 的 key 数单调增长，内存不能在该 Owner 存活期回收；调试计数把幽灵桶算成订阅，也会掩盖真实卸载泄漏。
- 建议：detach 在 observer 删除后回收空 source entry（可在借用结束后 remove，或结束后 retain 非空）；将订阅计数定义为活跃边数/活跃源数并提供两个明确指标，不能只报容器 len。为 dispose、Show/For 分支 churn 建统一 graph snapshot，断言 nodes、edges、buckets、cleanups 四类计数。
- 验收：RED：复用 changing_dependencies_removes_inactive_edges 的路径，switch 从 false 切到 true 后，旧 b 对应 subscribers entry 是空集但 `subscriber_count()==3`（switch、a、b 三个 key）；继续接触新 sources，map 基数持续增加。GREEN：同点为2个有效 source subscription，断开最后 observer 立即删除 source key；在单长命 Owner 中交替挂载/卸载至少10^4个不同动态源后，subscriber bucket 数等于活跃订阅源数，detach/dispose 后归零。

**建议**：先在不依赖宏/GPU 的 reactive core 中落地两套合同：`Mark→Memo 拓扑稳定化→Binding commit→cleanup`，以及 `For UI owner／Store row model／Resource generation` 状态矩阵；第一步先封住 Compute 期写入并修复嵌套 batch 原子性。上述 red/green 测试未通过前，不应把 Todo 目标语法或现有基础 sink 探针扩大解释为 Memo、Store、For、异步生命周期已可用。

## A3：图形、窗口与平台架构师

不确定性：我未在本会话中再次运行 cargo run/test；"已实测" 仅引用 docs/research/validation-and-roadmap.md 与本仓库对应 PoC 源码明示的本地验证范围。本机之外（macOS/Metal、Linux Wayland/X11、sRGB/BGRA surface、Intel iGPU、clipboard/DragDrop/IME/AccessKit 实战、真实 glTF/mesh/PBR、跨窗口 device-loss）均无源码/实测支撑，结论属【目标设计】或【待验证】。vello_gpu 0.4 TextureBindings 对 Rgba8Unorm 之要求、glifo atlas 限制、Classic 0.12 STORAGE_BINDING 在 macOS Metal swapchain 上的兼容性需在引入 zenui-render-adapter 时直接对照该版本源码核对。docs/research/stack-decisions.md、docs/research/CONCLUSION.md 也已经交叉阅读但未列入 evidenceRead 上限。

### 证据
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\todolist\main.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\todolist\README.md`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\desktop_poc\src\main.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\desktop_poc\src\compositor.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\desktop_poc\src\compositor.wgsl`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\desktop_poc\src\vello_layer.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\native_poc\src\gpu.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\native_poc\src\lib.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\desktop_poc\Cargo.toml`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\native_poc\Cargo.toml`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\docs\research\runtime-and-rendering.md`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\docs\research\validation-and-roadmap.md`

### P0 桌面窗口/winit/IME/AccessKit 完全缺位，headless PASS 不能算桌面能力

- 证据（有源码依据）：examples/desktop_poc/src/main.rs:189-199 仅默认 Instance.request_adapter + request_device，无 winit / Window / SurfaceConfiguration；examples/desktop_poc/Cargo.toml:9-12 仅 vello/wgpu/pollster。examples/native_poc/Cargo.toml:9-17 同样无 winit/accesskit/accesskit_winit；native_poc/src/main.rs 仅 zenui_native_poc::run()。docs/research/CONCLUSION.md:70 与 docs/research/validation-and-roadmap.md:49、76-81 都把 winit 真实 surface、DPI/resize、device loss、IME、AccessKit、Windows/macOS/Linux 实机验收挂在 Gate C。docs/research/stack-decisions.md:18、31 把 winit 0.30.13 / accesskit_winit 0.34.2 与 zenui-platform 列为待建模块；docs/research/runtime-and-rendering.md:17-20 仅画出 WindowShell 目标拓扑。全局 grep winit|accesskit|WindowEvent|configure\(|get_current_texture|set_ime_allowed|Ime::|clipboard 在 examples/**/*.rs 中只命中 request_device 无关行，证明窗口/IME/剪贴板路径完全缺失。
- 失败场景：把 'Sparse-strips GPU + External Texture + CJK' 的 headless PASS 当作可桌面交付：接入 winit 后第一次 SurfaceError::Lost/Outdated、跨显示器拖动 scale_factor 突变、IME candidate 框 logical↔physical 单位换算、device lost 时 in-flight TextureBinding/Font atlas 仍引用旧资源，都会出现文字拉伸、IME 错位、闪退黑屏。现有 PoC 不会触发这些路径也无结果存根。
- 建议：先把 PoC 在文档里的'已实测=单 Instance/单默认 adapter 的 headless Rgba8Unorm 纹理'与'未启=WindowShell'二分写清。引入独立 Gate C0：真实 winit 窗口 + per-monitor scale_factor + SurfaceError/DeviceLost 处理；共享 Device/Queue，每窗口独立 Surface；缓存按 docs/research/runtime-and-rendering.md:84 的 DPI epoch 失效；任何后端 unsupported 走 docs/research/stack-decisions.md:53 的 Result 路径。Gate C 勾选不得以 headless PASS 代替。
- 验收：Red：grep winit::|accesskit|WindowEvent|configure\(|get_current_texture|Ime::|set_ime_allowed 在 src 与 examples/**/*.rs 均无命中；desktop/native PoC 跑完不创建任何真实 OS 窗口。Green：扩展 desktop_poc 引入 winit 0.30.13 + accesskit_winit 0.34.2，跑通 2 个独立 winit 窗口共享 1 个 Device/Queue、各自 SurfaceConfiguration；两次拖动窗口跨 scale_factor∈{1.0,2.0} 时物理尺寸/scale 与字体 epoch 正确更新；拖动 IME 候选框不越窗口；注入 device lost 后能重建 swapchain 继续绘制；断言挂在真实 WindowEvent 路径上（不再 headless）。

### P0 Classic 与 sparse-strips 的 alpha/format 约定相反，adapter/能力矩阵未落地

- 证据（有源码依据）：examples/desktop_poc/src/compositor.wgsl:11-19 注释明确 Classic v0.12 fine.wgsl 会 un-premultiply，shader 自行 straight-alpha source-over；examples/native_poc/src/gpu.rs:100-127 验证的是 sparse-strips premultiplied + CPU reference [128,0,0,128]。两端 PoC 都把目标写死 Rgba8Unorm（desktop main.rs:6，native gpu.rs:47、170），desktop 还叠加 STORAGE_BINDING（main.rs:41）。两边都 request_adapter(&Default::default())（main.rs:189、gpu.rs:18）+ request_device(&wgpu::DeviceDescriptor::default())（main.rs:198、gpu.rs:26），无 features()/limits()/capabilities 检查，无 adapter 选择策略。docs/research/stack-decisions.md:15、51 与 docs/research/runtime-and-rendering.md:94 提了 capability gate 但没代码；docs/research/style-system.md:73-74 自承 'Vello Rgba8Unorm 不等于 sRGB color space 已解决'；docs/research/runtime-and-rendering.md:133 知道可能要 clone matching texture view formats，PoC 未示范。docs/research/runtime-and-rendering.md:145、158 明确两条线 alpha 约定不同。
- 失败场景：接入真实 swapchain：Windows DX12 多为 Bgra8UnormSrgb，macOS Metal 为 Bgra8UnormSrgb 且 STORAGE_BINDING 兼容性更严，sparse-strips 在 BGRA 通道下红色矩形变蓝、绿色占位方框反相；Classic compute 需要 Rgba8Unorm+STORAGE_BINDING，在 sRGB/普通 swapchain 上不可用。Intel 集显或 GL backend 上 atlas 尺寸超 limit 时 wgpu 直接 validation panic，无法走 docs/research/stack-decisions.md:53 要求的 unsupported=Result。
- 建议：引入独立 zenui-render-adapter 模块，启动时依 adapter.features()/limits() 与 surface.get_capabilities(&adapter) 算出 formats/alpha_modes/STORAGE_BINDING 兼容性/glifo atlas 上限等能力；附 capability matrix（backend × format × external texture × mask/filter × clip × sRGB）。Classic straight-alpha shader 与 sparse-strips premultiplied path 拆两个 backend 适配器，外部纹理边界按 docs/research/runtime-and-rendering.md:145 的 {format, color_space, alpha, frame_epoch} 元数据 bundle 传入；禁止生产端复用 compositor.wgsl。
- 验收：Red：grep 'request_adapter\(&Default::default' 命中且无 Rgba8UnormSrgb/Bgra8UnormSrgb/limits/features 分支判定。Green：定义 RenderCapabilities { supported_formats, required_features, required_limits, alpha_mode, external_texture_format }；同 SceneRenderer 在 headless Rgba8Unorm 与真实 Rgba8Bgra8UnormSrgb+AlphaMode 上各跑一次并给色彩断言；adapter 按 power_preference + compatible_surface + backend 白名单挑选；CI 覆盖 Vulkan/DX12/Metal/GL 各 1 台真机；把 compositor.wgsl 与 vello_layer.rs 显式标注 probe-only。

### P1 实机/平台能力矩阵没有被参数化，headless 单平台覆盖被当成普适正确性

- 证据（有源码依据）：native_poc/src/gpu.rs:11-13 WIDTH=128 HEIGHT=64 ROW=512 写死，无动态 resize；native_poc/src/lib.rs:20-71 4 个 lib test 全部 CPU 端数值断言，不触真实 swapchain；gpu.rs:139-145 render_options 把 TargetInit 设为透明 Clear；gpu.rs:106-127 用同一份 RenderTarget + Rgba8Unorm 做半透明对照，没有跨 backend/scale/format 矩阵。desktop_poc/src/main.rs:4-6 WIDTH=64 HEIGHT=64 写死。docs/research/validation-and-roadmap.md:3 自承环境 = Win11 + RTX 4060 Laptop + Vulkan；第 49、53 行承认 macOS/Metal、Linux Vulkan/GL、multi-monitor DPI 全部未验收。glifo atlas 'experimental'(validation-and-roadmap.md:42) 与 ICU4X 模型缺失在 lib.rs:46-60 的修复仅在 Windows 单机上完成。docs/research/runtime-and-rendering.md:84 强调 per-monitor DPI 单独作为 epoch；docs/research/style-system.md:73-74、docs/research/runtime-and-rendering.md:133 把 P3/HDR/sRGB 标为后续 capability。
- 失败场景：把 native_poc 4 个 PASS 误写成'跨平台文本/alpha correctness 已达'：macOS Metal 强制 sRGB swapchain 与 atlas 格式不兼容；Wayland 没有全局 pointer 与 per-monitor DPI 变化多端；Intel iGPU max_texture_dimension_2d 通常更小。PoC 之外，atlas 超限立即 panic、半透明像素偏离 reference 不被 CI 捕获；ICU4X 数据若打包缺失又会退化为 No segmentation model 警告，无任何平台矩阵覆盖这些失败模式。
- 建议：把'已实测'范围显式锁到该基线，parametric 测试覆盖格式、alpha Mode、scale、texture size 边界；CI 引入 platform/adapter 矩阵至少 4 平台对；未通过项按 docs/research/stack-decisions.md:53 标'unavailable'，不再造 'PASS' 文案；把 glifo atlas 'experimental' 升级为 build/runtime 门槛。
- 验收：Red：lib.rs 把 4 个 [已实测] 测试标写明但未约束 adapter/platform；CI 无平台矩阵配置；gpu.rs 头仅一处宏常量。Green：在 native_poc/lib.rs 与 gpu.rs 上方注明 '仅 64×64/128×64 headless Win11 NVIDIA/Vulkan 已实测，其余 [待验证]'；参数化测试覆盖 Bgra8/Rgba16Float、透明/不透明 alpha Mode、scale∈{1.0,1.25,1.5,2.0}、max_texture_dimension_2d 边界与 1×1；CI 矩阵 Windows(DX12+Vulkan)、macOS(Metal)、Linux(Vulkan/X11 与 Wayland、GL) 各 1 台真机跑同一种子并保存像素基线 hash。

### P1 离屏 3D 目前只有绿色占位框，无 mesh/depth/lifetime 实路证据

- 证据（有源码依据）：native_poc/src/gpu.rs:28-33 external 纹理 usage=TEXTURE_BINDING|RENDER_ATTACHMENT，record_clear_viewport(gpu.rs:176-198) 仅 Clear (0,1,0,1) 提交；gpu.rs:64-65 用 scene.fill_rect 在 (4,4)-(36,36) 画占位方块；gpu.rs:52-55 仅 create_depth_texture_view 供 sparse-strips 内部裁剪用，没有独立 offscreen color/depth 双缓冲或 mesh 渲染；两个 PoC 均无 vertex/index buffer、相机、uniform、PBR。docs/research/CONCLUSION.md:73 与 docs/research/validation-and-roadmap.md:55、85-87 明确 '当前外部纹理测试是绿色占位'。docs/research/runtime-and-rendering.md:128-167 给出 ModelViewer/TextureId/Camera/generation 但属目标设计；examples/todolist/main.rs:1 与 examples/todolist/README.md:3 自承整体不能编译。
- 失败场景：把 external TextureView + TextureBindings 通路误读为'离屏 3D 已就绪'：接真实 glTF viewer 时缺独立 depth-stencil attachment/vertex/index buffer/per-object uniform/generation 校验。用户关闭 ModelViewer，worker 才把 texture 推入 TextureBindings，shader 仍引用上一世代 TextureView→validation error 或渲染过期帧；多 viewport 同框时共享 Vello 内部 depth_view（docs/research/runtime-and-rendering.md:148 已禁）会让 3D 物体被半透明 UI overlay 错误遮挡。
- 建议：把'Green Square 占位'写进文档标题；新增独立 zenui-3d probe：3D 离屏 color+Depth32Float own target，与 UI 同尺寸/scale，经 TextureBindings 汇入；资源 owner+generation 持有，dispose 时清理注册与 in-flight 引用；显式禁止把 Vello 内部 depth 当 3D depth；完成 Gate D（validation-and-roadmap.md:85-88）前不接受'支持 glTF viewport'文案。
- 验收：Red：grep 没有 vertex/index buffer 与独立 depth_stencil_attachment 配对出现的代码。Green：新增 3D probe 创建独立 Depth32Float 离屏 + 匹配 size/scale 的 Rgba8Unorm color，经 TextureBindings 注入 UI；同一帧 UI underlay + 真实 mesh + 半透明 overlay 在 scissor viewport 内合成；2/4/8 个 viewport 实例并跑，owner 释放后 TextureId 注册即清，所有 in-flight 引用 generation 校验；CI 像素断言覆盖背面剔除/深度 Z-test；model unload 后原 TextureId 不再被引用。

**建议**：先把 headless 的'已实测'范围在 docs + 源码注释里精确锁到 (Win11 + NVIDIA/Vulkan + Rgba8Unorm + 64×64/128×64)，把 winit 真实窗口/per-monitor DPI/SurfaceError/DeviceLost/AccessKit/IME 接通落成独立的 Gate C0；随后抽出 zenui-render-adapter，按 adapter.features()/limits()/surface caps 维护 backend × format × alpha_mode × 限值的显式能力矩阵，所有 unsupported 路径走 Result，headless PASS 不再被引用为桌面或跨平台正确性证据。

## X1：UI 框架爱好者与技术选型评审者

不确定性：本轮仅只读源码与设计文档，没有执行任何编译、测试或工作流；因此不对现有 PoC 的测试结果作独立实测背书。第 1 条的 Copy/String 冲突可由 Rust 类型规则直接判定；其余为明确标注的目标设计风险，而非声称当前代码已复现失败。对 SolidJS、Leptos、Flutter、Slint 的提及仅用于说明可采用的机制边界，不代替其当前版本源码审计。

### 证据
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\todolist\main.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\todolist\README.md`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\desktop_poc\src\lib\project_signals.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\macro_poc\runtime\src\lib.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\macro_poc\frontend\tests.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\native_poc\src\text.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\docs\research\native-macro.md`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\docs\research\style-system.md`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\docs\research\runtime-and-rendering.md`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\docs\research\validation-and-roadmap.md`

### P1 TodoRow 的 Copy 契约与 String 字段直接矛盾

- 证据（有源码依据）：E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\todolist\main.rs:26-31 同时写 #[derive(Clone, Copy)] 和 title: String；README.md:9 又称 builder 接收“可复制 handle 快照”。String 不实现 Copy，这不是宏能力未实现，而是该数据契约自身违反 Rust 类型规则。
- 失败场景：即使未来补齐 zenui::prelude、#[component] 与 view!，该目标应用仍在 TodoRow 的 derive 处以 E0204 失败；“复制 handle 而不复制文本”的核心开发体验无法按示例成立。
- 建议：把稳定身份和字段投影分开：优先让 TodoRow 只保存 id、title: Signal<String> 与 completed，标题文本用 move || title.with(Clone::clone) 绑定；或者只对不可变标题提供明确的 Store 读接口。可借鉴 Store/Leptos 式字段投影原则，但不要引入额外 DSL 或把整个 Store 都变成逐字段信号。若确实返回 String 值则删除 Copy 并明确 clone 成本。
- 验收：red：用当前 TodoRow 定义做最小类型检查，应得到 String 无法 Copy 的 E0204。green：扩展 examples/macro_poc 的 trybuild，合法 TodoRow 能在组件、For builder 与多个事件闭包中传递；不复制标题底层数据，标题读取结果正确，Store row 删除后失效句柄按约定报错。

### P1 行字段直接写入与 Store 查询之间缺少可执行的依赖契约

- 证据（目标设计）：E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\todolist\main.rs:47-49 计算 filtered/remaining/has_completed，:141 取出行内 completed，:149-150 却直接 completed.set(checked)。README.md:12 只说 Memo 缓存与共享订户，其“生产 runtime 验证”尚未完成；真实探针 E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\desktop_poc\src\lib\project_signals.rs:58-159 只有普通 signal/sink，没有 Store、Memo、跨 Owner 字段订阅。
- 失败场景：勾选一项后，行颜色和删除线变化，但 remaining 仍为旧值、Clear completed 仍隐藏、Active 筛选仍保留已完成项。若为避免陈旧而让 Store 每次整体替换 Vec，又会使所有行订阅失效，偏离细粒度更新。
- 建议：明确 Store 查询是 tracked projection：remaining/has_completed/filtered 必须读取相关行字段 signal（或用等价的行字段索引通知），而不是读取无订阅的缓存快照；可提供 store.set_completed(id, value) 做世代校验和统一命令入口。借 Solid 的 dependency tracking，只在 tracked 计算域执行查询读取，不能照搬 JS GC 或假定普通 Store 方法自动建边。
- 验收：red：添加一项后勾选，若只更新行绘制而 remaining、has_completed 或筛选结果不变即失败。green：勾选/取消精确更新计数、清空按钮与匹配行；TodoApp 不重新 mount，其他行的标题/编辑器不重建，Memo 对每个 sink 批次不重复计算，Store row model 的生命周期独立于 For 的 UI owner。

### P1 过滤后的 For 会反复卸载行，不能承担普通筛选的状态保留

- 证据（目标设计）：E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\todolist\main.rs:112-125 把 visible.get() 同时用于空态 Show 和列表 For；README.md:10 明确过滤消失会释放 UI scope，出现相同业务 key 时再重建，局部状态需要另行 snapshot/hide-store。native-macro.md:97-100 规定 key 删除即销毁 owner，For 重排才保留行节点。
- 失败场景：用户在 Active 与 All 间切换，未变化且仍存在的业务行被卸载再挂载；未来行内编辑草稿、未提交 IME 内容、滚动内嵌状态都会丢失或跳变，文字 layout、订阅和节点分配也会反复发生。简单 key 并不能解决，因为过滤后的数组确实删掉了该 key。
- 建议：分开 keyed reconciliation、条件展示与回收：默认对稳定排序 ID 集运行 For，过滤通过 retained-visibility/隐藏占位改变布局、命中和 AccessKit 可见性，而不是销毁行 owner；真正删除才释放。过滤时隐藏焦点节点要确定性迁移，若承诺恢复草稿则按 TodoId 明确 snapshot 策略；大行表用有上限的虚拟化缓存，不能无限保留。正确吸收 Solid For/Show 的分工，不把 For 误当过滤缓存。
- 验收：red：反复切换 All/Active/Done，若同一未删除 TodoId 的 mount 次数或 Parley layout 构建次数持续增长即失败。green：重排保持节点、selection 与 IME；隐藏行不再命中/被读屏，焦点进入可见删除按钮；恢复策略有断言，真实删除后 owner/subscriber 计数回落。

### P1 受控输入只有字符串回写，不足以保证 caret、选择区与 IME

- 证据（目标设计）：E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\todolist\main.rs:72-82 的 value 依赖 draft，而 :77 的 on:input 每次都 draft.set(value)。README.md:15 已要求仅外部逻辑值改变时更新 editor，但没有给出本地输入来源、相等比较和编辑器状态协议。真实文字探针 E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\native_poc\src\text.rs:12-23 仅做 Parley shaping/换行，:39-64 仅 glyph 绘制，不包含编辑状态。
- 失败场景：中文拼音 composition 中输入或提交候选时，on:input 更新 draft，value binding 又把同一字符串作为外部值写回，导致 composing range 被取消、caret 跳到末尾或选择区丢失；不能再用 on:submit 特判 Enter 来掩盖编辑器回写问题。
- 建议：采用 TextEditState { value, selection, composition } 作为 widget 本地权威，区分 InputOrigin::Local/External；本地提交不发起等值回写，外部 draft.set 才按明示 selection 策略投影。可借 Flutter TextEditingController 将 text/selection/composing 一起处理的状态边界，但不要复制其 ChangeNotifier 或整棵 widget rebuild。明确 UTF-8、UTF-16/native IME 和 AccessKit character index 的转换，不能只复用 Parley glyph offset。
- 验收：red：模拟中文 composition、候选提交、混合 emoji 的中部插入，若 composition 被取消或 caret/selection 漂移即失败。green：本地 on:input 不产生 value 外部 patch；外部清空/替换只应用一次；composition 期间 Enter 不 submit，caret、选区与 AccessKit 文本范围一致，真实 IME 与 automated reducer 测试均通过。

### P2 CSS-first 与完整优先级引擎先于核心控件扩张，形成第二套浏览器兼容工程

- 证据（目标设计）：E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\docs\research\style-system.md:23-32 已设 origin/layer/variant precedence，:56-67 又定义多级覆盖且精确规则待 fixtures；:82-105 再规划 @theme/@utility、外部扫描与 rebuild。validation-and-roadmap.md:49-55 同时承认窗口输入、IME、结构 runtime 及真实编辑器均未验收。
- 失败场景：同一 selected/hover/dark/inline 样式在 Rust 宏、CSS-first 扫描器、native resolver 中得到不同优先级或主题未触发重建；团队先承担 CSS 编译器兼容与工具链维护，却无法让 Todo 的输入和列表稳定工作。
- 建议：Gate A/C 只支持 Todo 所需静态 literal utility、固定主题和 typed style:*，先统一宏与 typed builder 的 StyleIR 和诊断；@utility/@custom-variant 在外部文件追踪、差分和控件垂直切片通过后再开。借 Slint 的有限 typed 属性及诊断经验适用于原生桌面稳定 API，不应复制其 DSL、浏览器 selector/cascade、preflight 或 JS plugin。
- 验收：red：同一 declaration 经手写 builder 与 view! 得到不同字段，或外部 theme 改变却不触发对应 patch/重建即失败。green：Todo 全量静态类和动态 style 有 golden 与优先级测试；theme 只 dirty 使用 token 的字段；不在 Gate A 引入 CSS selector/JS plugin 也能完成桌面 Todo 垂直切片，unsupported 输入有精确定位诊断。

**建议**：先用一个真实可编译、可交互的 Todo 垂直切片锁定 Store 字段订阅、For/Show/retained-visibility 和 TextEdit 状态协议，再扩 CSS-first、3D 等外围能力。当前最具区分度的机制不是 Tailwind 数量，而是字段更新不重建组件、筛选不误删行身份、输入不破坏 IME；这三项必须成为发布 API 的硬验收。

## S: 系统性怀疑论者与对抗评审主持人

不确定性：(1) 我没有运行任何测试或渲染探针：以上“已实测”行均来自读到的源码与 validation-and-roadmap.md 自报的执行结果，不是我本机实际跑出。PASS/PASS 输出我是当“文档声明的实测”对待，未独立复现。 (2) native_poc 与 desktop_poc manifests 没有 Cargo.lock 入 git 难以证实，未逐字检查。 (3) 我没有逐行审阅 macro_poc/frontend/tests/compile_fail/*.stderr 文件，所以断言的“未知 mx-auto 会报错”依据的是 macro/src/lib.rs:47 与 runtime/src/lib.rs:91 的代码路径推断，可能在 trybuild 校验文案中有出入；建议主控方跑一次 trybuild 确认。 (4) vello_gpu 0.4.0 + parley 0.13.0 的具体 cache/panic 行为我没有读上游 source，所有 mask/filter panic 引用都来自 validation-and-roadmap.md:54 的文档自陈。 (5) desktop_poc/src/main.rs、compositor.rs、vello_layer.rs 我没读，Classic interop 的精确边界以 validation-and-roadmap.md:13 与文档结论为准。

### 证据
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\Cargo.toml`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\todolist\main.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\todolist\README.md`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\macro_poc\runtime\src\lib.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\macro_poc\macro\src\lib.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\desktop_poc\src\lib\project_signals.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\native_poc\src\gpu.rs`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\native_poc\Cargo.toml`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\examples\desktop_poc\Cargo.toml`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\docs\research\CONCLUSION.md`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\docs\research\validation-and-roadmap.md`
- `E:\code\mine\zenui\.delta\worktrees\ws200twab8j5\zenui\docs\research\tailwind-coverage.md`

### P0 “Copy handle 即人体工学”这一核心承诺在真实运行时上是空的：当前 PoC 的 Signal<T> 是 Clone 但非 Copy，并且携带 Rc<RefCell<T>>、Weak 等运行时身份

- 证据（有源码依据）：examples/todolist/main.rs:26-31 声明 `#[derive(Clone, Copy)] pub struct TodoRow { id: TodoId, title: String, completed: Signal<bool> }`，main.rs:38 又把 `#[derive(Clone, Copy)] pub struct TodoStore(TodoStoreId)` 设为目标。但 examples/desktop_poc/src/lib/project_signals.rs:17-22 是真实 PoC：`pub struct Signal<T> { id: usize, value: Rc<RefCell<T>>, valid: Weak<Cell<bool>>, graph: Weak<RefCell<Graph>> }`。该结构因含 `Rc`/`Weak`/`RefCell` *不允许 derive Copy*，且克隆它会复制 Rc 所有权（不是单一 ID），信号读写都依赖 `valid.upgrade()` 和 `graph.upgrade()`（26-46 行）。Owner 用 `Rc<RefCell<Graph>>`（86-91 行）实现单线程 arena，没有 generation，dispose（171-181 行）只是把 valid 设 false 而不释放 value Rc。
- 失败场景：把 todo 的 `completed: Signal<bool>`、`store: TodoStore` 按文档说的当成 ID handle Copy 给多个按钮 move 闭包时，运行时实际是 Rc 强引用计数+1；Owner dispose 不会释放已捕获的 Rc，导致 store/graph Rc 被闭包持有到应用退出，`Signal<T>` 自身的 PI validity 检查只能靠 `valid.upgrade()` 判断，弱 ref 失效后 `set` 静默返回（48-50 行 `if !self.valid.upgrade().is_some_and(...) { return; }`）。在等待新世代 row 替换时，旧 handle copy 持有的 Rc 让旧 value 仍被读——和 README.md:13 承诺的“不赞成 stale handle 继续默默读取/修改别人创建的一代新 row”直接冲突。
- 建议：把“Copy handle 不触发数据克隆”拆成可验证契约：(a) 真实 Signal<T>/TodoStore 只能是单字段 Copy ID（典型 `u32` slot + `u32` generation），不含 Rc；(b) Owner 持有到数据表格的独占引用（`Rc<RefCell<StoreInner>>` 但不是 `Signal<T>` 自身的字段）；(c) Signal 是 `#[derive(Clone, Copy)] { slot: u32, gen: u32, owner_id: u32 }`；get/set 必须 generation check + 错误语义，不能静默 return；(d) dispose 用 generation 释放数据而不是只把 valid 关掉。改造后才能命名 TodoRow/TodoStore 为 Copy。
- 验收：RED：examples/desktop_poc/src/lib/project_signals.rs 中 Signal 诊断为 `RefCell`/`Rc` 字段，derive(Copy) 报 E0204；同时旧 generation handle 在 Owner recycled slot 后仍能读到新值（无 generation 字段）。GREEN：同一个文件改造后 Signal<T> == `Copy`，新增 `generation` 字段；新增单元测试：dispose+recreate 后旧 handle 做任何 with/set 必须返回 `Err(Stale)` 或 panic（dev），TestRow/Store 都能 `#[derive(Copy)]` 通过编译、跨 closure 复制不增加任何 Rc 强计数（用 `Rc::strong_count` 断言）。`cargo test --manifest-path examples/desktop_poc/Cargo.toml --lib` 通过。

### P0 class utility 在 PoC 中是 5 个硬编码 token，TodoList 使用了 30+ 个 class；“Tailwind 覆盖率高”目前全为设计标定

- 证据（有源码依据）：examples/macro_poc/runtime/src/lib.rs:48-96 `compile_classes` 仅匹配 5 个字面字符串：`flex`/`flex-row`/`flex-col`/`bg-blue-600`/`bg-blue-800`/`w-72`，并对其它 token 一律 `Err(error(token, offset))`；Property 枚举（7-11 行）只有 DisplayState/Background/Width 三种，Condition（13-17 行）只有 Always/Hover/Pressed，Value（18-25 行）只有 Flex/Row/Column/Rgba/LogicalPoints。但 examples/todolist/main.rs:61 使用 `mx-auto flex w-full max-w-xl flex-col gap-6 bg-zinc-50 p-6 text-zinc-950`，第 73、81、92、113、129、146、153、168 行涉及 `rounded-md border focus-visible:ring-2 hover:bg-blue-500 disabled:opacity-50 aria:pressed style:text-decoration-line` 等 30+ 个 class/variant/property。tailwind-coverage.md 全篇是 N/D/R/X 设计矩阵，没有 fixture manifest 也没有 generated percent。
- 失败场景：用户按文档复制 examples/todolist/main.rs:61 的 `mx-auto flex w-full max-w-xl flex-col gap-6 bg-zinc-50 p-6 text-zinc-950` 进真实宏，宏会调 `compile_class_literal` → `compile_classes`，第一个 token `mx-auto` 在第 91 行直接命中 `_ => return Err(error(token, offset))`，宏报错：“unsupported native class `mx-auto` at byte 0”。整个 TodoList 在 PoC 上根本不能渲染，连最外层 view 都过不去，更不谈 `style:text-decoration-line={... TextDecoration::LineThrough}`（宏根本没有接受 style: 前缀属性，macro/src/lib.rs:77-81 行 reject 除 class/on:* 外所有 attribute）。
- 建议：把“Tailwind 覆盖率高”从设计目标剥离为不可提前声称的能力。短期：(a) 在 docs/research/CONCLUSION.md、README、示例注释中删掉任何暗示“class 直接用 Tailwind 字面量就能跑”的措辞；(b) 把 compile_classes 的覆盖列表真实数（当前 6 个）写在 macro_poc README 头一行；(c) 长期建真实 Tailwind v4 pinned-commit manifest，生成 candidate list，每个 candidate 在 fixture 里有 expect-Native/expect-Diff/expect-Reject，未实现的一律 compile_error；用 trybuild 锁定。
- 验收：RED：把 examples/todolist/main.rs 第 61 行复制进 macro_poc 现有 integration 测试，断言编译失败的 stderr 至少包含 'unsupported native class `mx-auto`'。GREEN：pinned Tailwind v4 manifest + 自动生成的 candidate tables（JSON 文件提交进 docs/research/fixtures/）+ 一组 trybuild cases：N 级 token 通过并断言 StyleRule、D 级 token 有差异警告、X 级 token 报 compile_error；覆盖率报告由脚本生成，不带手写百分数。

### P0 宏实现了 view/text/button 三种标签，事件只有 click/change/input，TodoList 关键标签 <form>/<main>/<article>/<input type="checkbox">/<For>/<Show> 全部不支持

- 证据（有源码依据）：examples/macro_poc/macro/src/lib.rs:46 写死 `if !matches!(tag.as_str(), "view" | "text" | "button")`，69 行写死 `if !matches!(event, "click" | "change" | "input")`，79 行 `probe implements only class and on:click/change/input`。但 TodoList main.rs:61 要求 `<main>`、71 行 `<form on:submit>`、72 行 `<input type="checkbox" aria:label placeholder value on:input>`、79 行 `<button type="submit" disabled={...}>`、89 行 `<For each={FILTERS}>`、112 行 `<Show when={...}>`、140 行 `#[component] fn TodoItem(todo: TodoRow, store: TodoStore)`、146 行 `<article>`、149 行 `type="checkbox" checked={...}`、159 行 `style:text-decoration-line={... TextDecoration::LineThrough}`。宏根本没有 `For`/`Show`/`component`/`aria:*`/`type`/`placeholder`/`disabled`/`checked`/`style:*` 任何处理路径。
- 失败场景：一位评审者看到 CONCLUSION.md 第 59 行“2 个宏 integration tests：rstml→StyleRule/typed record 转写”，误以为宏系统已具有 TodoList confirmation，实际编译 main.rs 时在 outermost `<main>` 上就命中第 47 行 `"probe accepts only native view/text/button"` compile_error。即便把 `<main>` 改成 `<view>`，仍然卡在 `type=` 属性、`<form>`、`<For>`、`<Show>`、`<input>`、`on:submit`、`aria:label`、`style:text-decoration-line`、`#[component]`、`key={|todo: &TodoRow| todo.id}` 每一层。这意味着 TodoList 不是“接近完成的目标语法”，而是“需要一次完整重写的语法”。
- 建议：不要把 todolist/main.rs 描述成“完整应用语法预览”（README.md:3 现状）。把它分类成“目标语法的 aspirational catalog”，并在文件头列一个未实现特性清单（每个特性指回宏的真实检查行号）。短期：把 `<view>/<text>/<button>` + `class` + `on:click/change/input` 对应的最小 todolist 单独抽出来作为 runnable skeleton；将 `<For>`、`<Show>`、`#[component]`、`style:*`、`aria:*`、`type`、`disabled`、`checked` 分别建立跟踪 issue；宏扩展每加一个，去掉一行 issue。
- 验收：RED：尝试在 macro_poc/frontend/tests/ 下加一个 integration test 直接使用 `<main>`、`<For>`、`<Show>` 各一例，三例全部被 rstml/probe 拒绝而产生 compile_fail；这被当作证据而不是失败。GREEN：宏端 `matches!(tag.as_str(), ...)` 列表扩到显式包含 `<form>`、`<main>`、`<article>`、`<input>`、`<For>`、`<Show>`，事件接受 `submit`，attribute 接受 `type`、`aria:label`、`placeholder`、`disabled`、`checked`、`style:*`，`#[component]` 真正消费；trybuild 通过 + 一个最小 todolist 在 PoC runtime 上 `cargo run` 成功，并保留 view/text/button class/on:click 三个原始 integration test 全部通过。

### P1 “vello_gpu 拼起来即可”在 PoC 中只验了 128x64 的单次 fill_rect + 一个外部纹理 + 半透明 alpha；深度、layer、mask、filter、clip、scroll、IME、device loss 全是黑洞

- 证据（有源码依据）：examples/native_poc/src/gpu.rs:11-13 WIDTH=128 HEIGHT=64 ROW=512，整个渲染只有 4 个 fill_rect（62、65、70、107 行）+ 一个 external texture paint（64 行）+ 一次 Parley glyph draw（67 行）；只有一个 depth_view（52-55 行）；texture format 唯一 `Rgba8Unorm`（47 行）；只在一帧提交与 readback；外部 texture 是手工新建的 128x64 纯色清屏纹理（176-198 行），label 直接写“external opaque viewport placeholder”；代码末尾 133 行自己承认 `no glTF/window/IME/a11y/platform-font/platform-GPU/mask/filter/performance claim`。validation-and-roadmap.md:54 写道“全新 sparse-strips 只是正确性开始。GPU mask/complex filter 会 panic 的官方风险仍在；分层 clip/mask/opacity、glifo 实用缓存、color profiles 和 atlas lifecycle 未验”，73 行承认“3D 验证目前是带 TextureView 的绿色占位而非真实 mesh”。
- 失败场景：把 PoC 的 interop 证成直接推到产品：“接着把 Vello Scene 叠加到 glTF mesh 上方的真实窗口”时，会遭遇：(a) mask/filter 在 vello_gpu 0.4 panic（上游已知，roadmap 自己承认）；(b) 默认无深度测试场景，2D 文本/矩形与 3D mesh 的 blend order 与 depth interaction 完全没有验证；(c) 外部绿方块不是 glTF，没有 camera/generation/async lifecycle；(d) id 合唱中 per-row dispose 时 scene+texture binding 释放顺序没有 spec，会产生 GPU use-after-free（`vello_gpu::Resources` 文档要求 encoder 不能引用已释放的 binding，gpu.rs:43-51 行只在同一次 new() 里构造一次）；(e) device loss/resize/format 改变均无处理路径。
- 建议：不要把 native_poc manifest 当成“渲染已通”。短期：(a) 在 CONCLUSION.md 把 native_poc 描述降级为“headless 128x64 单目标一次提交的证明，仅为 OK 是 OK”；(b) 把“Vello 拼起来即可”从任何文档、结论、roadmap milestone 中删除；(c) 提出一条 explicit Gate C-：跨代数场景测试——同帧 2D overlay 覆盖 glTF mesh，depth target 共享，resize + device loss + mask + 半透明 group 全部测试一遍，所有 panic path 显式返回 Result。
- 验收：RED：现行 native_poc 仅在 128x64 + 固定 Rgba8Unorm + 一次 fill_rect 下断言通过；任何 clip/mask/filter/外部 PBR mesh/depth 共享用例都不存在。GREEN：native_poc 新增 (1) 一个真正的 mesh 加载（即使 triangle-subset）渲染到独立 color+depth target；(2) 至少一个 mask、一个 clip、一个 group opacity 用例；(3) resize（改变 RenderTargetConfig width/height）后再 render 仍然可断言像素正确；(4) device loss 注入（强制 wgpu Device 失败后每一个 render/resize 调用返回 `Err(..)` 而非 panic）；(5) CI 至少在 NVIDIA Vulkan 及 CPU fallback 双 backend 通过。

### P1 duration/token 版本耦合+synchronization 风险：PoC 五处依赖全是精确 pin，CONCLUSION 把“固定版本”当稳定，但 rstml/syn/parley/vello_gpu 三处存在隐式熔点

- 证据（有源码依据）：examples/native_poc/Cargo.toml 全=`=` pin：vello_gpu 0.4.0 / vello_cpu 0.4.0 / glifo 0.5.0 / parley 0.13.0 (+complex-scripts)/ parley_engine 0.13.0 / taffy 0.15.1 / wgpu 30.0.2。examples/desktop_poc/Cargo.toml 同时 pin vello = 0.12.0 + wgpu 30.0.2（另一个不同时代 vello）。examples/macro_poc/macro/Cargo.toml pin rstml 0.13.1 + syn version "3"。validation-and-roadmap.md:41 自己写道“rstml 0.13.1 使用 syn 3，把 syn 2 AST 混入产生 incompatible error”，42 行“默认 atlas_cache(false) 避免把其标注“实验”的 cache 作为基础能力”。CONCLUSION.md:9 把整套版本冻结成“生产候选版本固定为…”。但没有 workspace-root Cargo.toml 来共享 version、没有 lockfile，没有 CI 跨 triples。
- 失败场景：把 examples 合成真正 crate 时：(a) desktop_poc 用 vello 0.12 + wgpu 30.0.2，native_poc 用 vello_gpu 0.4.0 + wgpu 30.0.2，两套 Vello 实例在 workspace 合并后同名 `vello` 包以不同 major 共存，类型不互通（peniko/kurbo 周期性冲突），需要 facade 适配而没有；(b) rstml 0.13.1 直接用 syn 3，但 proc_macro 客户端 leptos/dioxus/typed-builder 仍多在 syn 2，混合使用时 `rstml::Infallible` / `syn::Expr` 跨 AST 不兼容；(c) parley 0.13 + parley_engine 0.13 必须同版本，但 parley_engine 只是个内部子诊断接口，下次 minor bump 时容易 torn；(d) `glifo 0.5.0` 只对齐 `vello_gpu 0.4.0` 的 glyph cache 模型（CONCLUSION.md 第 42 行明示缓存是实验性关闭），一旦 v0.4.x patch 改 cache 设计就是 breaking change；(e) parley 复杂脚本字典数据（validation-and-roadmap.md:34 提到第一次 run “ICU4X data error: No segmentation model for Chinese/Japanese”）需要发布 artifact 外带 ICU data，binary size + 加载路径未定义。
- 建议：把版本策略从“固定钉死”升级为“workspace 单点 truth”：(a) 根 Cargo.toml 加 `[workspace.dependencies]`，把六组高频耦合 crate 统一 version；(b) 显式决定是否拆 vello 0.12 与 vello_gpu 0.4 为两种 backend feature，二选一，避免 workspace 合并冲突；(c) rstml/syn 的 AST 边界要在 `macro_poc/macro/Cargo.toml` 上方文档明确警告 syn 3 强制要求；(d) parley ICU segmentation data 单独打包（path + license）；(e) 加 lockfile 入 git；(f) 加最小 CI：每提交 cargo test --workspace + 一次 headless native_poc 在 GPU runner 与 CPU fallback 双轨。
- 验收：RED：当前仓库不存在 workspace-level Cargo.toml（已经读，根 Cargo.toml 1-7 行只有 [package]+[dependencies]），cargo build --workspace 失败。GREEN：根 Cargo.toml 转 `[workspace]`，六个 version 用 `workspace.dependencies` 提供；`cargo metadata --locked --format-version=1` 不再出现重复 major 版本的 vello/peniko/kurbo/wgpu；CI 通过 GPU + CPU 双 backend；readme headless run + CI artifacts 明确带来的 ICU 字典文件列表。

**建议**：三件事按顺序做，再做下一步： (1) 把 examples/todolist/main.rs 与 docs/research/CONCLUSION.md 中所有会引导读者“class/Signal Copy handle/vello 拼起来即可”的叙述显式降级为 aspirational，并将 macro_poc/runtime/src/lib.rs:48-96 仅有 6 个 token 的 compile_classes、macro/src/lib.rs:46 仅有 3 标签的证据头置——现在的 TodoList 可在 PoC 上渲染的暗示是这套研究的最大安全风险。 (2) 立即把 examples/desktop_poc/src/lib/project_signals.rs 的 Signal<T>（Rc/Weak/RefCell）改造成 ID+generation 的纯 Copy 结构，并能用 Rc::strong_count 断言；这决定了 README 第 13 行承诺的“stale handle 必须报错”是否成立。 (3) 把根 Cargo.toml 升级为 [workspace] + [workspace.dependencies]，引入 lockfile + CI（GPU + CPU 双 backend、三平台至少一台），否则 Vello 两代 major 共存 + rstml/syn 3 + parley/parley_engine 双 pin 这三条裂缝在合并到真仓库时会同时爆，验证全部要重跑。

## U2：熟悉 React/SolidJS/Tailwind 的前端初学者；只读了根 Cargo.toml、docs/research 与 examples/todolist、macro_poc、desktop_poc，未独立执行测试。

不确定性：没有运行相关测试，【已实测】仅指源码内显式 assertions 所覆盖的局部机制；TodoList 整文件及 Store/Form/受控编辑器仍不可运行。For key 的类型约束、宏参数化组件语法等以目标源码为准，不假设未读的 rstml/未来实现细节。

### 证据
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/Cargo.toml`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/todolist/main.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/todolist/README.md`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/macro_poc/macro/src/lib.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/macro_poc/runtime/src/lib.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/macro_poc/frontend/tests.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/desktop_poc/src/lib/style.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/desktop_poc/src/lib/project_signals.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/native-macro.md`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/style-system.md`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/tailwind-coverage.md`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/validation-and-roadmap.md`

### P0 受控输入不能只保留 React 的 value/onInput 外形，缺 native editor 状态规范；示例本身还有 Copy derive 类型错误

- 证据（有源码依据）：examples/todolist/main.rs:26-31 给含 String 的 TodoRow derive(Clone, Copy)，42 行 clone title 又用作多闭包；71-78 是 form/on:submit 和 value={move || draft.with(Clone::clone)} + on:input，147-150 是 checked/on:change。README.md:14-15 已说明原生而非 HTML form，但只提要求保存 caret/selection/IME。当前宏 examples/macro_poc/macro/src/lib.rs:46 只允许 view/text/button，68-81 事件无参数路径；runtime/src/lib.rs:103 的 Event 是 Box<dyn FnMut()>，138 的 on 也不接收值。
- 失败场景：迁移者直接认为 <input value on:input> 已是 React 等价受控输入；中文 IME 按 Enter 上屏被误判 submit，外部 set/For 重建时 caret/selection 丢失；on:input={move |value: String| ...} 若套现有 Event 类型也直接不匹配。TodoRow 的 Copy derive 会显式报 E0204，add_todo 的 move 捕获与 draft.with 的复制语义也难判断。
- 建议：先修 TodoRow 派生和闭包捕获，保持 API 目标可类型检查；增加 typed SubmitEvent/InputEvent/ChangeEvent<T> 和 TextEdit 状态契约。明确 native submit 的 trigger、IME composition 内 Enter 不 submit、editor 仅外部逻辑值变更时同步、内部 input 回写不回灌字符串并重置 caret；For 删除/过滤恢复是否保存编辑快照要写进规范。不要把它改名为 onChange 来暗示 DOM 语义。
- 验收：Red：TodoRow Copy 编译失败；受控输入每次 on:input 回写造成 selection/中文候选取消，composition Enter 提交任务，过滤再恢复丢焦点/编辑态。Green：cargo check 目标示例通过；on:input/change 能传 typed value；composition Enter 只确认候选，正常 Enter 与 Add 共享一次 command，外部 draft.set 才更新 editor，内部键入 caret/selection 连续，For 删除 owner 后订阅释放。

### P1 {value} 一次性初值 vs {move || ...} 订阅，与 JSX 每次 render 求值是形似而本质不同；For/Props 的子表达式边界要单独规范

- 证据（有源码依据）：docs/research/native-macro.md:46-67 和 CONCLUSION.md:44 明确 {expr} 是初值、{move || expr} 才是绑定、on:* 不 tracked；todolist/main.rs:76-77 受控字段因此写 move，53-58 add_todo 内 draft.with/get 只作为事件命令。已有 frontend/tests.rs:33-43 仅证明 mount 不 eager 执行和显式 evaluate/fire 次数，不是真实 tracker。PoC lower_block 只按 ExprClosure.capture=Some 判文本绑定（macro/src/lib.rs:184-200），还未涵盖 prop/attr 泛化。
- 失败场景：React 初学者写 <text>{remaining.get()}</text> 或 <input value={draft.with(Clone::clone)}>，视觉与 JSX 一致但只挂载一次，计数和输入框永远停在初始值；反之把 on:click 包在 memo/effect 追踪里也会引入不应有的订阅。
- 建议：保留 SolidJS run-once 和显式更新边这一优点，但所有数据位置统一显式 binding 语法，给出 JSX→Zenui 对照 lint/codemod：动态文本、value/checked/disabled/style、Show/For 必须 move closure；事件只 untracked command。Props/子组件 factory 中的 expression 是 mount 配置还是 response 边需按位置写明，不靠任意 closure 魔法推断。
- 验收：Red：非 closure 动态表达式可编译但 signal.set 后 UI 不变，或事件 closure 静默成为订阅者。Green：对漏写 move 的高风险 value/checked/text 给出针对性诊断/提示；使用 move 后仅对应 field patch，组件 setup 不重跑；事件中 reads 不产生 subscriber，闭包只在 dispatch/binding 时执行。

### P1 active: 看起来是 Tailwind 的按压态，class:if-selected/checked/aria:pressed 才是 native 业务状态；示例筛选按钮应避免 JS className 条件反射和仅色彩可见状态

- 证据（有源码依据）：CONCLUSION.md:52 明确 active: 指 native Pressed 非选中；macro runtime compile_classes 在 examples/macro_poc/runtime/src/lib.rs:54-57 将 active 映射 Condition::Pressed；native-macro.md:16-21 示例另有 class:if-selected={move || selected.get()}。TodoList main.rs:92-103 的筛选按钮用 aria:pressed 加两个 style:* 条件色/字色，尚未用 selected variant；README.md:16 指定 inline 与 class 按固定 origin/variant 优先级，非拼接顺序。现有 desktop style.rs:181-188 则只是 base/current modifier 的局部覆盖，production precedence 未定稿。
- 失败场景：前端迁移者写 active:bg-blue-600 表示当前过滤项/待办完成态，结果只在鼠标按下瞬间着色；或者写 filter == X ? "selected" : "" 动态字符串，编译期没有对应 ClassId；如果再用同名 class/static style 覆盖 style:background，易出现与 Tailwind 预期不同的 precedence。
- 建议：把 active→Pressed、selected/checked→typed semantic states 做置顶迁移警示；筛选按钮证明 aria:pressed 与 selected visual style/reconciliation 同源，最好用注册 class:if-active-filter 或明确 typed style 条件，不用运行时拼接 ClassString。补齐 literal class/dynamic class/inline/focus-visible/disabled precedence fixture，而不是只在文档描述；无 aria selected 和 color contrast 不应算完成。
- 验收：Red：active 被用于持久筛选状态，点击后颜色回落，或 class 字符串中出现未注册 token 仍被接受。Green：按住才应用 active/Pressed；选中样式在 pointer release 后保持，并同步 aria:pressed=true、reorder 后对应正确 key；未注册动态 class 编译诊断；static/class/inline/variant 同字段冲突按文档矩阵逐 fixture 通过。

### P1 函数组件的标注参数并不等同 React props object；事件带参、children/slots 与 closure 所有权要有可编译的真实垂直样例

- 证据（有源码依据）：todolist/main.rs:40-41 的 #[component] fn TodoApp(todos: TodoStore) 与139-140 的 TodoItem(todo, store)，89-125 有 For child closure，均超出 PoC。native-macro.md:107-121 只给了另一个带 #[prop(default)]/Children 的目标签名，42 明确组件参数不要求 Copy；macro/src/lib.rs:44-51 只收小写 intrinsic，不支持 PascalCase；children 目前受限且每元素至多一个 content（89-99）。root Cargo.toml:1-7 实际无 zenui API 依赖或 lib。
- 失败场景：初学者按 React 写 fn TodoItem(props: TodoItemProps)、props.children 或 spread {...rest}，在 Rust 中类型/生命周期/ownership 都不等价；也可能误以为 TodoItem 每次父筛选 rerun 可局部变量替代 signal，或 For each/key 与子闭包是按 JS map 执行。
- 建议：先稳定 run-once typed function props 与显式 children factory/slot 形式，不实现伪 JSX props 对象或任意 spread；Todo 示例的 For each={FILTERS} key={|entry| entry.0} 要明确接收 &/value 类型，以 TodoRow Copy handle 为例逐步解释 move capture。增加同一 TodoItem typed builder/宏双路径，能让初学者直接对照 React props、Solid callback children 与 Rust ownership。
- 验收：Red：TodoItem<TodoItemProps> 或 {...props} 没有明确 unsupported 诊断，初学者只得 opaque macro error；For 中重复 key、重排或 Copy handle 借用导致错误复用行状态。Green：PascalCase typed component、typed event payload、children factory 在真实 build 中通过；props 字段 rename/类型错误定位到 Rust 字段，For 必须 key、按 key preserve row/local state、delete owner，组件 setup 只运行一次。

### P2 TodoList 的 README 标注很明确，但文件仍像完整应用；需要提供同一界面的可运行迁移阶梯，避免把“已支持字段绑定”误读成“框架可运行 Todo”

- 证据（有源码依据）：todolist/README.md:3 和 main.rs:1-5 已诚实标为目标语法；但根 Cargo.toml:6-7 无依赖，main.rs:7 zenui::prelude 未实现，native-macro.md:19/42 与 CONCLUSION.md:44 都声明 components/props/slots/For 是设计。当前 PoC 全量只支持 class + on:click/change/input 和小撮 class（macro/src/lib.rs:61-81、runtime/src/lib.rs:48-95），没有 form、input 属性、style:*、aria:*、For/Show。
- 失败场景：新用户从 examples/todolist/main.rs 复制到 cargo new，看到 JSX-like 外观和 README 中具体 Store contract，直接 npm-to-cargo 迁移预期运行，最后会同时得到 crate/API/macro/style 多类错误，无法判断是代码写错还是阶段未实现；容易放弃或错误转用 VDOM。
- 建议：README 顶部增加 runnable/support 矩阵和 copy-guard（例如模块级 compile_error 或独立 target-design package 避免意外进 examples auto-discovery）；另做 4 个由静态 UI→局部 signal→受控 TextEdit→keyed For 的 staged PoC，每一步对应真实 runtime 能力，不伪造 Store。将 Tailwind N/D/R/X 链到每个 class，让学习者明确浏览器语义应放弃还是保留。
- 验收：Red：复制整个 Todo 文件被当作 supported app 尝试构建，用户只看到一堆 missing API；文档示例中的普通 class 在原 PoC 也直接 unknown class。Green：目标示例和可运行阶梯明确隔离，cargo test/check 都只会在正确 target 运行；前端初学者按阶段每步可运行并知道新增能力边界，不支持标签/attr/class 给出具体位置诊断。

**建议**：保留 SolidJS 的一次组件执行、显式字段订阅和 keyed For，而不要把 JSX 的 re-render、React 受控输入及 CSS className 拼接偷带进原生树。优先交付一条 typed form/TextEdit + IME + For disposal 的真实可用迁移样例，并用编译诊断/fixture 固化动态绑定与样式状态规则。

## X2：Rust 类型系统、零成本抽象与公共 API 红队

不确定性：本轮仅静态读取源码，未执行 cargo、trybuild、Todo 或任何工作流，因此没有把文档中的“已实际验证”当成自己的实测。根 Cargo.toml:6-7 无依赖，Todo 文件自身也声明不可运行；除了可由 Rust 类型规则直接确定的 Copy 问题及现有函数签名外，完整组件/For/Memo/Store 均没有可检查实现。第 4 条是目标 API 的所有权风险：Memo::get 尚无正式签名，克隆或重新查询的代价取决于最终契约，不应被当作现有库的性能实测；需要用明确的计数与分配验收来定型。

### 证据
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/Cargo.toml`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/todolist/main.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/todolist/README.md`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/macro_poc/macro/src/lib.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/macro_poc/runtime/src/lib.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/macro_poc/frontend/tests.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/examples/desktop_poc/src/lib/project_signals.rs`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/native-macro.md`
- `E:/code/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/runtime-and-rendering.md`
- `E:/code/mine/mine/zenui/.delta/worktrees/ws200twab8j5/zenui/docs/research/validation-and-roadmap.md`

### P0 TodoRow 的 Copy 派生含 String，目标类型层面直接不成立

- 证据（有源码依据）：examples/todolist/main.rs:26-31 为 `TodoRow` 派生 `Clone, Copy`，但字段 `title: String`。这与 `Signal<T>` 是否为 Copy 无关：`String` 不实现 Copy，派生必然触发 E0204（并伴 E0277）。README.md:7、9 又将可复制 handle/row 快照当作 For 的稳定语义，故不是普通排版瑕疵。
- 失败场景：即使给 zenui 填上全部组件、Signal、For 和组件宏，Todo 这个无外部宏歧义的行结构仍会在 `derive(Copy)` 处编译失败；后续 TodoApp/TodoItem 的可组合性验证无从开始。
- 建议：不要改成复制整份文本，也不要用逐项 `clone()` 掩盖问题。保留 `TodoRow: Copy`，把静态标题留在 Store 的 row model 中，row 仅携带 `TodoId` 与小型稳定字段 handle（例如 `TitleHandle`/`Signal<String>`）以及 `completed`；静态渲染安装在首次读取时取文本。若选择整行所有权快照，则移除 Copy 并同步改掉 README 的 handle 快照约定，但不应继续宣称该结构是小型 Copy 行。
- 验收：red：将当前 TodoRow 定义放入 trybuild，因 `String: Copy` 不满足得到 E0204。green：完整 Todo fixture 通过 cargo check；`fn assert_copy<T: Copy>() {}` 对 TodoRow 成立，Console 无字段/业务数据克隆；同一 TodoRow 可复制传入 TodoItem 和多个事件闭包，标题仍从稳定 Store/字段 handle 获取，增删行后不错读。

### P1 事件 PoC 的零参 FnMut 与裸字符串分发，无法承载 Todo 的 typed payload

- 证据（有源码依据）：目标 Todo 在 main.rs:71、77、150 分别传入 `SubmitEvent`、`String`、`bool` 参数的处理器。真实 PoC 则在 macro_poc/runtime/src/lib.rs:103 定义 `Box<dyn FnMut()>`，在 138-147 用裸 `&'static str` 查找并零参调用；macro/src/lib.rs:68-80 只是原样转写 `.on(#event, #expression)`。frontend/tests.rs:21-25 也只验证零参 click，没有证明 payload 推断。公开 builder 直接接受任意字符串事件名。
- 失败场景：如果把 Todo 的一参闭包直接交给现 Node::on，类型不匹配；若为适配而统一剥成 FnMut()，就必须从隐式共享/全局“最后事件值”取 String/bool，或丢弃 payload，形成错误输入、跨控件串值及不可组合 API。另一方面 `Node::on("inptu", || {})` 可编译，fire 时静默不匹配；typed builder 与宏的诊断能力不等价。不要据此声称 Todo 已验证，它是目标 API。
- 建议：按控件与事件类型提供 `on_submit(F: FnMut(SubmitEvent))`、`on_input(F: FnMut(String))`、`on_change(F: FnMut(bool))`；宏仅做编译期名称到 typed 方法的映射。底层存储可用 `Box<dyn FnMut(E)>` 的异构事件表，这不是需要清除的动态分派。click 可无参，或用单位 payload 的明确 adaptor；禁止裸字符串作为公共 builder 的事件契约，无需隐含全局状态。
- 验收：red：把 main.rs:77/150 的闭包交现有 Node::on 应编译失败，而未知裸字符串事件名却可通过编译。green：三个目标处理器在生产候选 typed-builder 与宏两条路径均编译，runtime 分派分别收到原始 SubmitEvent/String/bool，且一次 dispatch 一次调用；错误 input 参数类型、未知 on: 名称及对应 builder unknown event 都产生编译错误。

### P1 失效 Signal 仍可读旧值、写入静默返回，且没有世代校验

- 证据（有源码依据）：desktop_poc/src/lib/project_signals.rs:24-45 的 `with` 只用 validity 条件包住订阅登记，最后仍执行 `get(&self.value.borrow())`；47-55 的 `set` 在失效时直接 `return`。`Signal` 仅保存 usize id（17-22），dispose（171-180）清图并置 liveness=false。319-335 的测试只断言 dispose 后 flush/subscriber 为空，没检查读写。docs/research/native-macro.md:68 与 todolist/README.md:13 则要求失效句柄有明确错误、槽位复用不能误命中新 row。
- 失败场景：行被过滤/删除后，排队的 checkbox 回调仍持旧 completed：写入静默无效，但任何 `with` 仍能读到销毁前的 bool；若上层把操作成功与 UI 状态关联，会形成无诊断的旧数据/命令分叉。未来若现 Copy ID 的 usize 槽位被重新分配而没有 generation，同一旧 handle 还可能绑定到新行。这里当前不会由这几行造成内存不安全，而是安全 API 的生命周期正确性缺口。
- 建议：定型 `SignalId { owner/slot, generation }`（Store row 同理）和单一失效策略：`try_with -> Result<R, InvalidHandle>`、`set -> Result<(), InvalidHandle>`，开发构建提供 owner/字段诊断。失效读取不能返回旧值，写入不能伪装成功；删除 UI scope 与删除 Store row 分层释放。沿用安全 Rust 的 arena/Weak 即可，不需要 unsafe；对 reentrant borrow 也返回明确错误而不是裸露 RefCell panic。
- 验收：red：现有测试在 dispose 后继续 `with` 会得到旧标量，`set` 只静默返回；新增断言能复现此行为。green：Owner dispose、For 临时移除 row、Store 真正删除三种路径均验证旧句柄返回 InvalidHandle；释放后重新插入并复用 slot，旧 completed/Store handle 仍不可读取或修改新 TodoId，而活句柄与订阅正常。

### P1 visible 的 Memo<Vec<TodoRow>> 被两个消费者 get，空态也承受整列表快照物化

- 证据（目标设计）：main.rs:47 缓存 `todos.filtered(...)`；112 的 Show 调用 `visible.get().is_empty()`，118-125 的 For 又调用 `visible.get()`。因此若 get 返回拥有所有权的 Vec，缓存值不能同时被移交给多个绑定，只能克隆列表或重新查询；若 Memo::get 仿 Signal 限定 Copy，则 Vec 根本不满足。README.md:12 已承认共享订户/commit 求值尚待 production runtime 验证，validation-and-roadmap.md:9、50 明确没有完整 Memo/Store/For 验证。
- 失败场景：一万行数据切换 Filter 时，即使 Store Query 被 Memo 缓存，显示“是否有匹配任务”的布尔绑定仍可能先物化整份列表，For 再产生第二份拥有式快照；移除 Memo 虽可省缓存层，却会让多个消费者重复执行查询。两种极端都没有兑现这里暗示的低开销，性能和共享求值不能只靠类型名 memo 保证。
- 建议：不要一刀切删掉 Memo：remaining、has_completed 等标量缓存有用。为 visible 定义 `StoreSnapshot/RowSetId { generation }` 之类的廉价可复制列表快照，由 For 按稳定业务 ID 消费并管理 row owner；为 Show 单独派生 `Memo<bool>`（通过只读 `with`/query predicate），不要暴露缓存 Vec 的所有权 clone。Store Query/Memo/For 组成可测试的 typed contract，并明确同 commit 的求值次序。
- 验收：red：若采用 Memo<Vec> + Copy-get，编译应拒绝 visible；若采用拥有式 Clone-get，仪器化原型会观察到 empty 绑定物化 N 行、For 又物化一次，测试判失败。green：标量 Memo 保持减少重复计算，empty 投影产生零 TodoRow/String 克隆；For 一次获取廉价快照并按 key 增删/重排；在一批 filter/completed 变更中 query 计算计数、快照获取次数及空态/列表 UI 都符合明确契约。

**建议**：先建立一个仅含 typed builder 和最小 reactive runtime 的可编译 Todo 垂直契约，而不是扩写目标语法：优先消除 Copy String；固定事件 payload 与失效 handle 的 Result/generation；再让 Show 消费 bool、For 消费稳定的廉价列表快照。把上述 red 用例纳入真实 trybuild/单元测试，未通过这些 Gate 前不宣称 Todo 是可组合、零额外克隆的 Rust API。

## A1 补审｜首席 UI 框架架构师（独立只读评审）

### 证据
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

**建议**：明确保留“一棵原生保留 UI 树 + 单一 reactive graph”，且不要引入 VDOM：树持有稳定 ViewId、widget/focus/scroll 状态和资源，图只负责派生值及生成 typed patch，OwnerId 负责图与子树共同卸载。这样字段更新不必重建视图或 diff，列表移动仍可保留原生状态，而 `For` 才用 keyed diff 决定节点增删重排，不能直接替换为任意孩子 Vec 重建。应先实现并验证上述 For 过滤与共享 Memo 的最小结构切片，再扩展宏、组件和 visibility/geometry。边界：根 crate 目前仅 `src/main.rs:1-3` hello-world；已验证的 reactive 与 macro probes 不等于已有生产树、组件运行时、窗口或 AccessKit。
