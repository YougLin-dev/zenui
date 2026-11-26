# ZenUI Starter

一个最小的 ZenUI 原生应用：窗口标题为 `ZenUI Starter`，按钮增加计数，关闭窗口退出。
它随模板携带固定字体 Noto Sans SC，界面包含一行中文样例；只依赖本地 `zenui` 与 `zenui-windows`，不依赖 demo crate。

`view!` 的标签、属性、事件与组件 props 词表见[视图语法参考](https://github.com/ZenUIOrg/zenui/blob/main/docs/SDK/视图语法参考.md)。编辑器目前不补全 `view!` 的标签与属性；参考页是这些名称的权威列表。

## 运行

在 Windows 11 上，从模板目录执行：

```sh
cd templates/minimal-app
cargo run --locked
```

模板从工作目录的 `assets/` 读取 `NotoSansSC-Regular.otf`；字体来源、覆盖、SHA-256 与 SIL OFL 1.1 许可见 [`assets/README.md`](assets/README.md)。这是独立 workspace，拥有自己的 `Cargo.lock` 与 `rust-toolchain.toml`。

把模板复制到仓库外时，`Cargo.toml` 的两个 path 依赖要指向 checkout；完整的环境要求、git 依赖写法与冷构建预期见 [从空目录开始](https://github.com/ZenUIOrg/zenui/blob/main/docs/SDK/从空目录开始.md)。

## 发布

```sh
cargo build --manifest-path templates/minimal-app/Cargo.toml --release --locked
```

产物是 `templates/minimal-app/target/release/minimal-app.exe`；设置 `CARGO_TARGET_DIR` 时是 `<CARGO_TARGET_DIR>/release/minimal-app.exe`，复制到仓库外的应用是 `<应用目录>/target/release/minimal-app.exe`。release 使用 Windows GUI 子系统，双击不附带控制台；debug 保留控制台便于诊断。

发布目录由 exe 与相邻的 `assets/` 组成；把 `templates/minimal-app/assets/` 复制到 exe 旁即可分发。启动时先找 exe 旁的 `assets/`，再找工作目录的 `assets/`；字体缺失、为空或不受支持时显示带路径的错误消息框并以退出码 1 结束，不会静默改用系统字体。`target/release` 中的 PDB 与中间产物只是构建输出，不作为分发内容。目标机器需要 Microsoft Visual C++ 2015–2022 Redistributable（x64）；DX12/WARP、未签名/SmartScreen 与不提供 installer/icon/version resource/signing/auto-update 的边界见[从空目录开始](https://github.com/ZenUIOrg/zenui/blob/main/docs/SDK/从空目录开始.md#发布构建)。

## 检查

```sh
cargo fmt --manifest-path templates/minimal-app/Cargo.toml --all -- --check
cargo clippy --manifest-path templates/minimal-app/Cargo.toml --workspace --all-targets --locked -- -D warnings
cargo test --manifest-path templates/minimal-app/Cargo.toml --workspace --locked
```

release 构建与产物说明见 `## 发布`。

测试包含 headless 组件点击、字体加载与缺失诊断、工具链/dev-profile drift 检查，以及 Windows 隐藏宿主
（宿主在没有硬件适配器时回退 WARP）的读回 smoke。隐藏 smoke 是 partial evidence；可见窗口仍需人工运行。

## 许可证

本模板以 `MIT OR Apache-2.0` 双重许可发布，你可以任选其一使用。许可全文见 [LICENSE-MIT](https://github.com/ZenUIOrg/zenui/blob/main/LICENSE-MIT) 与 [LICENSE-APACHE](https://github.com/ZenUIOrg/zenui/blob/main/LICENSE-APACHE)（绝对链接，复制到仓库外后仍可用）。随模板分发的 Noto Sans SC 使用 SIL OFL 1.1，许可见 [`assets/OFL.txt`](assets/OFL.txt)。分发模板副本或 release 目录时，请一并附带这三份许可文本。
