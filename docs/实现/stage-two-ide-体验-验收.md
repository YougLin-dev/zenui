# Stage-two IDE 体验验收（rust-analyzer）

本页记录 #291：以公开模板 `templates/minimal-app` 为入口，检查编辑 ZenUI 应用时的补全、跳转、悬停文档、宏诊断、编译错误与快速修复。检查由脚本化 LSP 客户端完成，不经过具体编辑器界面。

**状态：已完成；Rust 表达式内的补全、跳转、悬停与宏诊断可用；`view!` 的标签与属性补全缺失，记录为 [#313](https://github.com/ZenUIOrg/zenui/issues/313)。**

验收日期：2026-11-26。功能基线为 `main` 的 `49b6c3e`（#312 合并提交）加本工单的 `Runtime` 文档补充。

## 环境

| 项目 | 读数 |
| --- | --- |
| 分析器 | rust-analyzer 1.99.1（58cf8f9c，2026-10-13），由 `rustup component add rust-analyzer` 安装 |
| 客户端 | `initialize`、`didOpen`、`hover`、`definition`、`completion`、`publishDiagnostics`、`codeAction` 的 LSP 脚本 |
| 分析根 | `templates/minimal-app`。模板是独立 workspace，仓库根 `Cargo.toml` 的 members 不含它 |
| 工具链 | 模板自带的 `rust-toolchain.toml`；Windows 11 |

## 方法

- 打开模板的 `src/lib.rs` 与 `src/main.rs`，等诊断收敛后逐项请求。
- 悬停取 `&mut Runtime` 中的 `Runtime`、`src/lib.rs` 中第二次出现的 `TextAssets`，以及 `main.rs` 中的 `load_text_assets`；跳转在同一 `Runtime` 位置请求定义。
- 在 `count.` 之后请求补全，代表 `view!` 内的 Rust 表达式；在 `<view role="main"` 的 `<` 之后请求补全，代表标签位置。
- 另建仓库外的坏代码应用，分别写入未知 class token、未知事件、非法 `style:height` 值与未使用导入，记录发布诊断的范围与对应 code action。
- 在模板目录运行 README「检查」一节的命令。

## 清单与结果

| 检查项 | 结果 | 读数 |
| --- | --- | --- |
| `view!` 表达式内补全 | 通过 | `count.` 后返回 `get`、`set`、`with`、`try_into`、`into`、`clone`、`to_owned`、`clone_into`、`clone_from` |
| 标签、属性与组件 props 补全 | 缺失 | `<` 后返回空列表；记录为 [#313](https://github.com/ZenUIOrg/zenui/issues/313) |
| 跳转到定义 | 通过 | `view!` 中的 `Runtime` 跳到 `src/runtime.rs` 的结构体定义 |
| 悬停文档 | 通过 | `Runtime`、`TextAssets`、`load_text_assets` 都返回中文文档；`Runtime` 与 `new` 的文档由本工单补齐 |
| 宏错误范围 | 通过 | 未知 class 指向 token 本身（第 6 行 22–40 列）；未知事件指向事件名（第 13 行 20–27 列）；非法属性值指向字符串（第 17 行 32 列），并给出 `ValueSlot<zenui::Length>: From<&str>` 与引入该约束的调用位置 |
| 编译失败诊断 | 通过 | 非法 `style:height` 值发布 E0277，severity 为 error，位置落在宏内的表达式上 |
| 快速修复 | 部分 | 未使用导入提供 “Remove all unused imports”；宏错误只提供 “Rewrite as raw string” 与 “Inline macro” 两个重构，没有自动修复 |
| 模板命令 | 通过 | `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked`（3 个测试）与 `cargo check --locked` 全部通过 |

## 缺陷与文档补充

- [#313](https://github.com/ZenUIOrg/zenui/issues/313)：`view!` 内没有标签、属性或组件 props 补全；需要决定文档方案或工具方案。
- `Runtime` 与 `Runtime::new` 原先没有文档，悬停只显示签名；本工单在 `src/runtime.rs` 补上类型与构造函数的契约说明。`TextAssets` 与 `load_text_assets` 已有完整文档，悬停直接可用。
- 悬停文档中的 intra-doc 链接按 rustdoc 默认规则指向 docs.rs 上的本地 crate（如 `https://docs.rs/minimal_app/...`），在编辑器中打开会 404；正文不受影响。

## 环境边界

- 检查由脚本化 LSP 客户端完成，不覆盖编辑器的补全列表渲染、文档弹窗、快捷键与 inlay hints。
- 模板是独立 workspace；把仓库根作为分析根时，模板文件不属于任何 workspace member，诊断与跳转不完整。编辑器应把 `templates/minimal-app` 作为独立根打开，或使用支持多根的 workspace 配置。
- 本机 rust-analyzer 的 flycheck 日志出现镜像路径（`e:\kache\workspace\src\view.rs`）的映射错误，属于本地构建镜像环境；原生诊断仍正常发布。
- 只记录 rust-analyzer 1.99.1；其他语言服务器、编辑器扩展版本与多根 workspace 不在本次范围。
