# 最小宏与样式编译验证

这不是 Zenui 发布版。范围恰好是：

1. `rstml 0.13.1` AST 读取 native tag、`class`、`on:click`、静态文本和 `move || String` 内容绑定。
2. class 在**编译期**变成 `StyleRule`，运行时不 split CSS string。
3. 嵌套元素按原生 Builder 组成；文本作为字段保留，不伪装成 DOM。
4. 普通事件 closure 不在 mount 时执行；只有显式 dispatch 才调用。文字 closure 也只在显式 evaluate 时调用；这里没有自动 subscriptions。
5. 未知 class 和 script-string handler 在编译期拒绝，非法闭合标签交给 rstml 错误。

包结构：`frontend` 是公开 facade；`macro` 是 proc-macro processor；`runtime` 是 inspection record 与共享的小型 class resolver。它**不是正式的 `zenui`/`zenui-macros` API**。

## 已实际验证

```sh
cargo test --manifest-path examples/macro_poc/Cargo.toml --workspace
cargo clippy --manifest-path examples/macro_poc/Cargo.toml --workspace --all-targets -- -D warnings
```

集成测试让调用方只 import 宏和公开 inspection 类型，不自行 import 生成代码里的 `Node`/`Content`。它验证 class 只初始化一次、嵌套 identity 不融合、事件一次 dispatch 执行一次、读文字一次调用一次，不会偷跑进 mount 初始化。

## 故意不做的事

- `Show`、`For`、`Index` 的结构/卸载/作用域还没有编译实现。
- `Rc<Cell>` 只是测试读写见证，不是最终 Signal 或 Resource。
- 这里只覆盖本 probe 的静态 enum utility 和两种状态变体，不是完整 Tailwind 或完整 cascade。
- processor 当前按这个实验 crate 命名输出 absolute `::zenui_macro_runtime` 路径。生产包必须解决 renamed dependency，使用 `$crate::__private`/公开 facade 支持路径或 manifest package resolution，并用独立 package trybuild 验证。三方 fixture 成功**不能证明这个 rename 边界已解决**。
- 错误目前指向整个 class 字面量，并包含 token 名与字节 offset；精确到字面量内部 UTF-8 范围的 diagnostic 仍需独立实现与 machine 验证。proc-macro2 的任意 string 创建 span 不能凭 “源码 offset” 自动绑定真实位置。
- `frontend` 与 proc macro 依赖图不能构造 runtime↔processor 循环。**不要中途加入所谓 runtime macro 安装 hook、如果包名不同的全局 unsafe 路由或隐式 import 条件。**
