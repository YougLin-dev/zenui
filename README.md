# ZenUI

ZenUI 使用 Rust 编写原生桌面界面。它保留控件及其编辑状态，通过响应式更新修改受影响的部分，而不是每次数据变化都重建整棵界面。

你可以用 Rust 管理模型和派生逻辑，用 `view!` 声明结构，用静态 class 书写样式。ZenUI 不嵌入浏览器，也不依赖 JavaScript、HTML DOM 或通用 CSS 引擎。2D 是主干，3D 作为布局中的可选内容接入。

## 当前能力

ZenUI 处在开发阶段。阶段一 target API 的 headless Todo 已可运行；原生窗口、完整 `view!` SDK 与平台输入尚未落地。

仓库目前提供：

- headless Runtime：模型、行集、代际句柄、Owner、Memo 和事务；
- 阶段一 target API：Copy Signal/Memo/Store Field、mount-once 组件、`view!` 数据位、`Show`/`For` 与共享 RowSet；
- headless 窗口 activation 与任务交付边界：app/window 所有权、Send worker、代际检查后的 UI continuation；窗口关闭取消其交付，共享模型仍可由 app 所有权任务提交；
- 有界运行时存储：最近发布快照按固定窗口保留，释放的 Owner 节点回到存活规模，Todo 的列表 reconciliation 与行句柄查找使用索引而不是逐行扫描；
- 可复现的 Todo 场景：插入、删除、筛选、重排、焦点与拒绝句柄后的恢复；
- 独立的宏、文本、布局和 GPU 原型；
- 目标界面语法的预览。

你可以运行 headless Todo 场景，或 stage-one target API 的最小 Todo：

```sh
cargo run -p zenui-app --example todolist_headless --locked
cargo run -p zenui --example minimal_todo --locked
```

target API 的公开行为测试位于根包 `tests/`，其中 `tests/tasks.rs` 覆盖窗口关闭、重开代际、过期交付与 app 所有权模型提交，`tests/ime.rs` 覆盖 headless composition 的 preedit、候选确认与取消；它们不声明原生窗口、原生 IME 适配或 Windows 完成。

根工程的二进制只保留最小入口，不会打开桌面窗口。目标语法的预览文件不参与默认 workspace 构建，因为只有部分所需 API 已经存在。

## 阅读路径

### 产品与路线

- [愿景](docs/产品/愿景.md)：产品定位、原则与边界。
- [路线图](docs/产品/路线图.md)：以 Windows 为首个完整产品平台的实现顺序。

### SDK

- [快速开始](docs/SDK/快速开始.md)：工具链、运行方式、工作区结构。
- [界面与组件](docs/SDK/界面与组件.md)：组件、属性、条件、列表和 `view!`。
- [数据与更新](docs/SDK/数据与更新.md)：Signal、Memo、Store、Field 和异步边界。
- [窗口与任务](docs/SDK/窗口与任务.md)：窗口实例、关闭确认、对话框和任务归属。
- [样式与输入](docs/SDK/样式与输入.md)：静态 class、typed style、文本编辑和 IME。

### 实现

- [技术架构](docs/实现/技术架构.md)：目标技术模型、研究借鉴与架构边界。
- [渲染与平台](docs/实现/渲染与平台.md)：布局、文本、绘制、内容接入和宿主能力。
- [验证重点](docs/实现/验证重点.md)：目标模型落地时需要验证的关键语义。
- [Stage-one 验收](docs/实现/stage-one-验收.md)：headless Todo 的可执行完成判据、边界证据与原生缺口。

## 界面语法

`view!` 描述结构，普通 Rust 代码负责逻辑。数据位传递句柄或明确的一次性值，不使用任意零参数闭包充当持续绑定。

```rust
let remaining = signal(3);
let remaining_txt = memo(move || format!("{} remaining", remaining.get()));

view! {
    <text class="text-sm text-muted">{remaining_txt}</text>
}
```

`once(value)` 表示在挂载时使用一次，不建立后续订阅。窗口、菜单、托盘和系统对话框由独立的平台 API 管理，不进入 `view!`。

## 开发环境

使用仓库固定的 Rust 工具链：

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run --release -p zenui-app --example reconcile_bench --locked
```

`reconcile_bench` 是固定负载的时间探针：扫描 500/1000/2000/4000 行，分别测量未变化列表 reconciliation 与空提交，并打印发布窗口和 Owner 存储规模。它只提供测量，不作为 wall-clock 断言。

独立的原型工作区使用各自的 `Cargo.toml`。宏原型可以这样运行：

```sh
cargo test --manifest-path examples/macro_poc/Cargo.toml --workspace --locked
```

开发约定见 [CONTRIBUTING.md](CONTRIBUTING.md)。
