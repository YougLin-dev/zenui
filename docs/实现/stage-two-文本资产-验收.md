# Stage-two 文本资产验收（实机）

本页记录 #279 的收尾切片 #286：模板与发布目录携带固定字体，渲染器与宿主在启动期校验资产，缺失或损坏时给出明确诊断。渲染器契约见 [ADR-0021](../adr/0021-text-assets-contract.md)，分词语料决策见 [ADR-0020](../adr/0020-application-segmentation-data.md)；渲染器与宿主的分层交付记录在 #284/#285。

**状态：可见自动驱动完成（Windows 11 单屏 150%，当前用户配置）；隐藏宿主测试、模板测试与 CI 只是部分证据，没有独立干净机器或独立用户配置的运行。**

验收日期：2026-11-26。功能基线为 `main` 的 `697e5d8`（#303 合并提交）；可见运行使用包含本工单模板资产改动的当前工作树。

## 环境

| 项目 | 读数 |
| --- | --- |
| 显示 | 单屏 2560×1600，150%（144 DPI） |
| 构建 | `cargo build --manifest-path templates/minimal-app/Cargo.toml --release --locked`，独立 workspace，release 使用 GUI 子系统 |
| 运行目录 | `E:\code\zenui-acceptance-286`（仓库外），`minimal-app.exe` 与 `assets/` 同级 |
| 字体 | Noto Sans SC Regular，8,331,336 字节，SHA-256 `faa6c9df652116dde789d351359f3d7e5d2285a2b2a1f04a2d7244df706d5ea9` |
| 采集 | UIA 读取窗口标题与文本；`PrintWindow(PW_RENDERFULLCONTENT)` 截图 |

字体文件、来源提交、版权、许可与覆盖记录在 [`templates/minimal-app/assets/README.md`](../../templates/minimal-app/assets/README.md)。

## 方法

- 发布目录由 release 构建的 exe 与从模板复制来的 `assets/` 组成；探针脚本 `capture.ps1` 位于仓库外，不提交。
- 正向运行：`Start-Process` 启动 exe，等待 `MainWindowHandle`，用 UI Automation 读取 `Text` 控件名称，`PrintWindow` 截图，`CloseMainWindow` 结束并记录退出码。
- 负向运行：把 `assets/NotoSansSC-Regular.otf` 改名（缺失）或写为空文件（损坏），重复启动并读取消息框；期望退出码 1。两次运行后恢复字体文件。
- 模板在启动时先找 exe 旁的 `assets/`，再找工作目录的 `assets/`；release 目录命中第一种，从模板目录 `cargo run` 命中第二种。

## 可见场景

同一台 150% 机器、同一次会话；时间戳与完整读数见 [`states.txt`](证据/template-text-assets/states.txt)。

| 步骤 | 读数 | 证据 |
| --- | --- | --- |
| 启动模板 | 窗口标题 `ZenUI Starter`；UIA 文本 `Count: 0` 与 `中文示例：你好，世界`；退出码 0 | [starter.png](证据/template-text-assets/starter.png) |
| 字体缺失（改名为 `.bak`） | 标题 `ZenUI Starter` 的消息框：`bundled font missing: E:\code\zenui-acceptance-286\assets\NotoSansSC-Regular.otf. Copy the template's assets/ directory beside the executable.`；退出码 1 | [missing-font.png](证据/template-text-assets/missing-font.png) |
| 字体为空文件 | 消息框：`text assets failed: font asset 0 has no bytes`；退出码 1 | [empty-font.png](证据/template-text-assets/empty-font.png) |

正向窗口同时显示 Latin 与中文样例；中文由模板 `assets/` 中的固定字体整形，系统回退在该配置下关闭。缺失与空文件都没有产生主窗口，也没有静默改用系统字体。

## 隐藏测试

- `crates/render/tests/text_assets.rs`：固定字体注册并覆盖文本、系统回退关闭、未注册家族与各类资产错误的 typed 诊断、同一缓存下 rect/caret/hit-test 稳定；Windows 上另测系统回退开启与主家族优先。
- `crates/render/tests/segmentation.rs`：应用 dictionary provider 被 Parley 查询，未配置时走编译数据。
- `crates/windows` 隐藏测试：无效资产在创建 HWND 前返回 `HostError::TextAssets`，唯一标题的窗口不存在；配置 Tuffy 的隐藏宿主所有 run 都使用该字体。
- 模板 `tests/assets.rs`：从模板目录找到字体；缺失路径的诊断含被检查路径。`tests/hidden_host.rs`：隐藏宿主用随附字体渲染 `Count: 0` 与中文样例；空字体在启动期返回 `MissingFont`。`tests/starter.rs`：headless 文本列表包含中文样例。
- CI `windows-template-release` 把 `templates/minimal-app/assets/` 复制到 release 目录，并断言字体 SHA-256 与 `OFL.txt` 存在。

## 判据对照

| 要求 | 状态 | 证据 |
| --- | --- | --- |
| #286：字体选择记录许可、大小、覆盖与 SHA-256，包含许可文件 | 通过 | [assets/README.md](../../templates/minimal-app/assets/README.md)；[OFL.txt](../../templates/minimal-app/assets/OFL.txt) |
| #286：模板加载资产并渲染 Latin/CJK，不依赖系统回退；可见运行记录家族与路径 | 通过 | 本页正向运行；家族 `Noto Sans SC`，路径 `assets/NotoSansSC-Regular.otf` |
| #286：发布目录把资产放在 exe 旁；缺失或改名给出 typed 诊断与明确消息 | 通过 | 本页两个负向运行；CI release 复制与哈希断言 |
| #286：记录干净机器/干净配置运行；只有当前配置时说明边界 | 通过（边界见下） | 本页环境与边界 |
| #286：README、快速开始与发布页记录资产与诊断 | 通过 | [模板 README](../../templates/minimal-app/README.md)、[快速开始](../SDK/快速开始.md)、[从空目录开始](../SDK/从空目录开始.md#发布构建) |
| 门禁：fmt、clippy、workspace 测试、模板测试与文档链接 | 通过 | 本地 `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked -- --test-threads=1`、模板同名门禁与 `.github/scripts/check_docs_links.py` |

## 边界

- **配置隔离：** 只在当前用户配置上运行；没有干净 Windows 安装、独立用户配置或字体管理器隔离。系统回退关闭与资产注册由隐藏测试覆盖，可见运行不逐 glyph 证明字体来源。
- **字体覆盖：** Noto Sans SC Regular 的 cmap 覆盖 30,890 个码点，包含模板的 Latin 与中文样例；任意用户输入的罕见字仍以该字体的覆盖为准，不承诺全 Unicode。
- **输入：** 可见运行只读取窗口文本，没有注入键盘或 IME；输入行为沿用既有的文本编辑与 IME 验收。
- **截图：** `PrintWindow` 采集；右/下黑边是采集产物。负向消息框由系统绘制，按钮文字随系统语言。
- **调试构建：** debug 入口同样显示消息框，并额外保留控制台；可见运行使用 release。
