# 参与开发

## 开发范围

ZenUI 使用固定 Rust 工具链。主工作区聚焦 headless Runtime 和可重复执行的模型场景；独立原型位于 `examples/`，各自管理依赖。

`crates/core` 保持标准库依赖，不加入 `unsafe`。修改它的身份、事务或释放语义时，同时更新相应单元测试和本地说明，不需要外部背景才能读懂行为。

## 必做检查

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

涉及示例时运行对应命令：

```sh
cargo test -p zenui-core --example syntax_model_shapes
cargo test --manifest-path examples/macro_poc/Cargo.toml --workspace --locked
```

这些检查验证实际的格式、类型和单元行为，不引入文档治理、编号或统计台账。

## 改动原则

先在当前模块讲清楚不变式，再决定是否需要补充注释。命名能承担的信息不重复写成注释。不能用类型、断言或结构直接表达的部分，保留简短解释。

代码注释不引用外部页面或历史编号。例如，释放 Owner 时说明何时拒绝再次挂载、子级释放和清理回调的顺序；事务中说明调用方可见的中间态和最终结果。这样阅读一份源文件即可理解该局部约定。

新功能应有针对实际行为的测试，包括常见输入、边界和失败路径。静态示例用于展示行为或类型形状，不能把不可运行的目标代码当成已经实现的 library。

对目标 SDK 的改动集中在同一份语法说明和最新示例中；不要把设计判断散落在各源文件的装饰性模板、责任人或历史评审术语里。

## 分支和提交

按一个可独立审查的行为或文档范围组织提交。提交标题和正文使用英文，说明完成的内容与必要的原因。

在推送或创建 PR 前，确保上面的检查仍然通过。CI 继续做格式化、Clippy 和三平台 workspace 测试；它不关心旧的文档编号系统。
