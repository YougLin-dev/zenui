# ZenUI Starter

一个最小的 ZenUI 原生应用：窗口标题为 `ZenUI Starter`，按钮增加计数，关闭窗口退出。
它只依赖本地 `zenui` 与 `zenui-windows`，不依赖 demo crate。

## 运行

在 Windows 11 上，从仓库根执行：

```sh
cargo run --manifest-path templates/minimal-app/Cargo.toml --locked
```

这是独立 workspace，拥有自己的 `Cargo.lock` 与 `rust-toolchain.toml`。

把模板复制到仓库外时，`Cargo.toml` 的两个 path 依赖要指向 checkout；完整的环境要求、git 依赖写法与冷构建预期见 [从空目录开始](../../docs/SDK/从空目录开始.md)。

## 检查

```sh
cargo fmt --manifest-path templates/minimal-app/Cargo.toml --all -- --check
cargo clippy --manifest-path templates/minimal-app/Cargo.toml --workspace --all-targets --locked -- -D warnings
cargo test --manifest-path templates/minimal-app/Cargo.toml --workspace --locked
cargo build --manifest-path templates/minimal-app/Cargo.toml --release --locked
```

测试包含 headless 组件点击、工具链/dev-profile drift 检查，以及 Windows 隐藏宿主
（CI 回退 WARP）的读回 smoke。隐藏 smoke 是 partial evidence；可见窗口仍需人工运行。
