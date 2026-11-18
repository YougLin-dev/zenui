# TodoList：目标语法

> 这是一份**面向最终 SDK 的目标语法预览，当前不能构建运行**。它不是对现有宏、Store、TextEdit、For、IME 或 AccessKit 已完成度的声明。运行验证详见[执行记录](../../docs/review/VALIDATION.md)，统一架构和验证 Gate 见[最终规格](../../docs/review/FINAL.md)。

## 一个例子里的五个关键规则

1. **组件执行一次**。`MountResult<MountPlan>` 允许 mount 失败；`view!` 构造一次性节点与绑定描述，不是在每次字段变化时重新执行整个组件、生成一棵虚拟树。目前只有普通 Rust 函数对 `?` 的类型证明；属性宏降级和 Plan 构建仍待 A1。
2. **一次性值与持续绑定分开**。`{title}`、`value={Filter::All}` 是创建期间的普通 Rust 值。动态文字/属性写 `{move || expression}`，或先命名 `move ||` 再放 `{name}`。它不是创建时的 bool，而是依赖改变后重算的闭包。
3. **字符串不是 Copy**。复制 `Signal<T>`/row handle 只复制 arena 身份，不复制内部字符串。`get()` 用于 Copy 标量，如 bool/enum；String 的 owned 输出写明 `with(Clone::clone)`。`TodoRow` 只存 id 与小字段 handle，不可变标题在每个 UI 行 mount 时通过 `Result<String, InvalidHandle>` 读取一次；句柄不保证永久有效。
4. **组件 props 按 Rust 实参传递**。`current={filter}` 将 `Signal<Filter>` handle 传给声明为 `Signal<Filter>` 的参数，**不是获取 Filter 值**。子组件 `move || current.get()` 才在 tracked closure 中读取。
5. **列表由 key 管理**。`For` 接收 `Memo<RowSet>` 共享的 id/field projection，`key={|row: &TodoRow| row.id}` 借用 id；child factory 接收一个小型 TodoRow。新增、删除、过滤改变结构，勾选一行不重新执行 TodoApp 或其他 TodoItem。RowSet 不意味着查询或物化全局 O(1)，详见[最终规格第 2 节](../../docs/review/FINAL.md#2-store-query-%E4%B8%8E%E6%89%80%E6%9C%89%E6%95%B0%E6%8D%AE%E6%8E%A5%E7%BA%BF)。

## 为什么有 `value` 还要 `on:input`

**`value={draft}` 让外部草稿流向编辑器；`on:input` 把用户提交的输入写回草稿。** 只有 `on:input` 不会让控件响应外部清空或撤销；只有 `value` 则不会把用户输入写回模型。它们是同一原语的两条明确控制流，不是隐藏的自动双向绑定。

`TextEditSession` 还持有 selection、caret 和 composition。已提交的模型值另外记录 epoch 和输入 origin。本地 committed input 通过 origin ticket 更新 Store，回到同一个编辑器时只确认 epoch，不把同一输入作为外部替换重演，也不把 caret 移到末尾。外部清空或 undo 才让专用 `apply_store_revision` 以 `ExternalReplace` 与 selection policy 走同一 reducer，不会再用普通 String binding 二次回写。

IME 候选词 Enter 由 TextEdit 消费，不提交表单；随后正常 Enter 或 Add 只生成一个原生 `SubmitCommand`。`prevent_default` 是命令对象上的操作，不是浏览器 DOM 事件。第一 Gate 只支持单 editor，多 editor/跨 view 并发另立验收。

## 错误、焦点与过滤生命周期

- `Transaction::new().write(todos.insert(title)).write(draft.clear()).finish()` 表示预演后的一次原子提交：成功才同时插入和清空，任一项失败则两项均不提交。已发出的外部 I/O 不由这种业务事务自动回滚，只能在 commit 后启动。
- `Signal::set` 与字段写入返回可检查的 Result。普通信号写入失败显式交给 `report_runtime_failure`；业务命令失败进入可见的 `error` Signal。TodoItem 的错误 Signal 由父级显式传递，错误 Signal 本身写入失败也必须处理，不能静默丢弃，也不能用 panic 当业务错误路径。
- `role="alert"` 的错误文字不自动抢焦点，也不等于已经由屏幕阅读器播报；AT announcement 是 C Gate 的真机验收，不是 ARIA 字符串本身。
- **reorder** 保持 UI id、generation、Owner 和局部编辑状态。
- **filtered out** 默认卸载 UI owner；Store row 模型仍在，再次出现将是新 UI generation。
- 默认**不承诺**同一条目过滤回来仍保留行内草稿、caret 或 IME。保存这些状态必须选择有界的 `retained_visibility` 或 `dispose_and_snapshot(key, TTL)`，另行验证容量、焦点和真删除清理。
- 真删除才使 row field handle 失效。迟到 callback 先经 UI generation 拒绝，再经模型 generation 校验，旧句柄不能命中复用 slot 的另一行。
- 结构 mutation 之前捕获 `FocusRemovalIntent`，commit 后按同 kind 下一项、上一项、可聚焦的输入控件、filter toolbar 顺序选 successor，跳过禁用/不可聚焦项目。用户已经主动设置的新焦点取消旧 intent；批量清除合并一次转移；必要的 scroll reveal 使用新的统一 GeometrySnapshot。
- **空 Store 与筛选无匹配分开。** 无数据时提示添加第一项；Store 有 row 时保留 `role="list"` 容器，即使当前 filter 无匹配也只提示 `No tasks match`，不额外卸载列表容器。

## 样式与语义

- filter 是 toggle button，`aria:pressed` 与官方 `aria-pressed:` 状态样式从同一个 Filter 比较派生，不使用 selected 变体或 `class:if-*` 自创条件语法。`active:` 只表示“正在按下”，不表示当前筛选。
- 勾选是 `<checkbox checked={...}>`，不在普通 text 上挂 `aria:checked`。完成态文字使用 `style:color` 与 `style:text-decoration-line` 的 typed `move ||` 值；不允许运行时拼任意 class 字符串绕过 registry。
- 普通 author inline 与同字段普通 CSS class/状态声明遵守 ordinary-origin 规则：普通 inline 声明胜出；`!important`、layer 等在固定版本差分验证之前不声明完整支持。Tailwind token 覆盖率只能由 pinned registry 与差异 manifest 计算，不是手写百分比。
- 表单、SubmitCommand、TextInput 和 AccessKit 是原生适配语义，不是浏览器 HTML。ARIA 字符串只是声明入口，角色、语义树、可访问输入与播读仍须真实验证。

## 当前还缺什么

示例中的 `StoreId`、`CompletedField`、`AppError`、`Transaction`、`MountResult`、`view!` 等是未来 API 的目标形状，不是已经存在于根 crate 的方法。下面这些 API 名称也都是设计，而不是完成任务：

- `TodoStore::new`、应用拥有的模型与 query；
- generational Owner/Signal/Store、RowSet、Memo 和事务 journal；
- 真实 winit 窗口事件循环、Windows 输入适配，以及 macOS/Linux 平台能力；
- typed component、For/Show/Index、完整 props 和真实宏错误诊断；
- 独立 Windows Todo 入口、`TextEditSession`、IME、FocusReducer、AccessKit。

实施顺序是先完成 **A0** 的纯 Rust command/query/reducer 原子事务与错误 sink，再完成 **A1** 的 Windows Todo 实窗与所需宏。后续 B、C、D 各自验证文字/布局/样式、三平台与可选 3D，不用一个 headless probe 代替这些发布门。

启动原型和未来 manifest 的规划见[机器可读验证合同](../../docs/review/verification-contract.json)及其 `status` 元数据；**该文件中的 fixture 路径和命令都尚未创建或执行**。未来命令可能形如 `cargo run --bin todolist --features windows-desktop --locked`，但当前项目没有这个 binary 或 feature，决不可把它作为现有可运行步骤。

本文件应作为目标 API checklist，而非“漂亮 DSL 已完成”的证明。已有小探针的命令与实际输出仅以 [VALIDATION.md](../../docs/review/VALIDATION.md) 为准。
