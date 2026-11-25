# ZenUI Starter

一个最小的 ZenUI 原生应用：窗口标题为 `ZenUI Starter`，按钮增加计数，关闭窗口退出。
它只依赖本地 `zenui` 与 `zenui-windows`，不依赖 demo crate。

## 运行

在 Windows 11 上，从仓库根执行：

```sh
cargo run --manifest-path templates/minimal-app/Cargo.toml --locked
```

这是独立 workspace，拥有自己的 `Cargo.lock` 与 `rust-toolchain.toml`。

把模板复制到仓库外时，`Cargo.toml` 的两个 path 依赖要指向 checkout；完整的环境要求、git 依赖写法与冷构建预期见 [从空目录开始](https://github.com/ZenUIOrg/zenui/blob/main/docs/SDK/从空目录开始.md)。

## 发布

```sh
cargo build --manifest-path templates/minimal-app/Cargo.toml --release --locked
```

产物是 `templates/minimal-app/target/release/minimal-app.exe`；设置 `CARGO_TARGET_DIR` 时是 `<CARGO_TARGET_DIR>/release/minimal-app.exe`，复制到仓库外的应用是 `<应用目录>/target/release/minimal-app.exe`。release 使用 Windows GUI 子系统，双击不附带控制台；debug 保留控制台便于诊断。

exe 是单个可移植文件，复制到另一个目录仍可运行；不捆绑 asset 或字体，文本使用系统字体。`target/release` 中的 PDB 与中间产物只是构建输出，不作为分发内容。目标机器需要 Microsoft Visual C++ 2015–2022 Redistributable（x64）；DX12/WARP、未签名/SmartScreen 与不提供 installer/icon/version resource/signing/auto-update 的边界见[从空目录开始](https://github.com/ZenUIOrg/zenui/blob/main/docs/SDK/从空目录开始.md#发布构建)。

## 检查

```sh
cargo fmt --manifest-path templates/minimal-app/Cargo.toml --all -- --check
cargo clippy --manifest-path templates/minimal-app/Cargo.toml --workspace --all-targets --locked -- -D warnings
cargo test --manifest-path templates/minimal-app/Cargo.toml --workspace --locked
```

release 构建与产物说明见 `## 发布`。

测试包含 headless 组件点击、工具链/dev-profile drift 检查，以及 Windows 隐藏宿主
（宿主在没有硬件适配器时回退 WARP）的读回 smoke。隐藏 smoke 是 partial evidence；可见窗口仍需人工运行。

## 许可证

本模板以 `MIT OR Apache-2.0` 双重许可发布，你可以任选其一使用。许可全文见 [LICENSE-MIT](https://github.com/ZenUIOrg/zenui/blob/main/LICENSE-MIT) 与 [LICENSE-APACHE](https://github.com/ZenUIOrg/zenui/blob/main/LICENSE-APACHE)（绝对链接，复制到仓库外后仍可用）。分发模板副本或 release exe 时，请一并附带这两份许可文本。
