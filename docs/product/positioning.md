---
doc-id: PRODUCT-POSITIONING
layer: product
status: draft
date: 2026-11-18
owner: 框架产品经理（技术型，10 年+）
sources:
  - archive/research/README.md
  - archive/review/FINAL.md
  - archive/review/README.md
replaces: []
---

# Zenui 产品定位

> 本文回答「为谁做、凭什么选它、承诺到哪一步」。实现形制见 `../spec/`，能力边界的**规范条目**见 [`../spec/00-scope.md`](../spec/00-scope.md)（`SPEC-SCOPE-*`），版本 pin 见 [`../spec/01-route.md`](../spec/01-route.md)。
> 状态词只用 `SPEC / PROBE / VERIFIED / WITHDRAWN / OPEN`；本文不含 `VERIFIED` 声明。

## 1. 一句话定位

**Zenui 是纯 Rust 的原生桌面 UI 框架**：一棵 retained View 树 + 一张细粒度响应式图，组件只 mount 一次；`view!` 给 HTML 外形，class 给 Tailwind 词表的**原生语义**；Taffy/Parley/Vello/wgpu 在明确边界内协作；2D 是主干，3D 是可选的布局叶节点。

它不试图「把 web 搬上桌面」。它把**写 React/Tailwind 时的表达力**，换成**没有 WebView、没有 CSS 引擎、没有 JS 运行时**的确定性。

## 2. 目标用户与 80% 场景

三层用户，产品形态相同、取舍不同。**每个里程碑只服务一层**（`SPEC-SCOPE-003`）。

| 用户层 | 他们 80% 在做什么 | 真正的痛点 | Zenui 给什么 / 代价 |
|---|---|---|---|
| **独立开发者 / 小团队**（首要） | 写一个 1–5 个窗口的本地工具：笔记、清单、抓取器、dev tool | Electron 包 100MB+、内存高、启动慢；Tauri 仍要会前端栈与 IPC；原生 Rust GUI 要么 immediate-mode 不好写表单，要么 API 门槛高 | 一个 `cargo run` 出窗口，`view!` + class 直写；代价：组件生态几乎为零，自己写控件 |
| **企业内部工具** | 内部后台/运维面板/数据查看器，Windows 为主，需要中文 IME 与无障碍合规 | 分发与许可、离线、DPI、中文输入、读屏软件；Web 方案的打包与签名链 | 单二进制、无 web 栈、DPI/IME/a11y 是一等目标；代价：还**没有**组件库、表格、富文本等企业必需品（`OPEN`） |
| **商业桌面应用** | 多窗口、大列表、GPU 绘制、品牌化主题、三年以上维护 | 长期 API 稳定性、性能可解释、跨平台一致性、审计（依赖与 unsafe 面积） | semver 承诺 + 可复跑 benchmark + 明确能力矩阵；代价：1.0 之前不可依赖，早期采用者需接受破坏性变更 |

**反向定位（谁不该用 Zenui，至少现在）**：需要成熟组件库/图表/富文本编辑器；需要 Web 生态或 SSR；需要 iOS/Android；需要 6 个月内交付生产。以上都应在文档首页直说，而不是让用户试了才发现。

## 3. 差异化与迁移理由

| 对比 | 它们的模型 | Zenui 的差异 | 迁移的真实成本 |
|---|---|---|---|
| Electron / Tauri | WebView + HTML/CSS/JS | 无 WebView、无 JS 运行时，渲染与布局都在 Rust 内 | 不能复用 npm 组件；需要重写 UI |
| Dioxus | VDOM + 多目标（web/desktop/mobile） | 单一桌面目标、retained 树、无 VDOM | 概念相近（RSX 外形），但更新语义与组件生命周期不同 |
| Slint | 自有 DSL + 编译器 | 用 Rust 宏 + Tailwind 词表，不引入第二门语言 | 需要适应 class 词表而非 DSL 属性 |
| GPUI | 高性能 retained，内部使用 | 公开规范、验收 Gate、单一响应式权威 | 无迁移路径，等待正式 API |
| Flutter | 三树 + Skia，Dart | 单树 + Taffy/Parley/Vello，Rust | 语言与生态全换 |
| egui / iced | immediate-mode / Elm 架构 | 细粒度字段更新，结构更新有独立生命周期 | 心智模型从「每帧重建」变为「一次 mount + patch」 |

以上对比是**产品判断，不是 benchmark 结论**（`SPEC`）。任何「比 X 快 N 倍」的说法在 `EVID-VAL-*` 采集前不得出现。

**差异化能否成立，取决于三个尚未证明的假设**（均 `OPEN`）：
1. Tailwind 词表映射到原生 StyleIR 后，普通应用的样式表达力足够（当前只有少量 utility 探针）；
2. 一次性 mount + 细粒度 patch 能覆盖真实表单/列表的复杂度（生产运行时未实现）；
3. Taffy + Parley + Vello 的组合能达到桌面观感与交互精度（headless 已验，真窗口未验）。

## 4. 稳定性承诺强度

**当前（pre-alpha，0.x 之前）**：没有任何公开 API 可被依赖。仓库中的 `examples/todolist/main.rs` 是目标语法预览，不是可运行应用；规划的 workspace 与 launcher 尚不存在。任何「已支持」表述必须能指向 `EVID-*`（`SPEC-SCOPE-021`）。

**0.x 阶段承诺**（`SPEC`，随首个可发布 crate 生效）：
- 破坏性变更**允许**发生在 minor 版本，但每次必须：一条 ADR、一条 CHANGELOG 条目、以及一页迁移说明；没有这三样不得发版。
- `experimental` 标记（`experimental` feature gate 或文档 `#[experimental]` 标签）的 API：**不承诺** semver，可随时删除或改形；删除前至少一个 minor 的弃用警告，安全/正确性修复除外。
- 未标记 experimental 的 API 在 0.x 内**SHOULD** 保持至少一个 minor 的弃用期。

**1.0 准入条件**（全部满足才发 1.0，见 `../product/roadmap.md` §5）：
- Gate C（三平台真实窗口 + 真实 AT）与 Gate D（3D 最小闭环 + 性能定稿）全部闭合；
- 公开 API 完成一轮「无 breaking 的 minor」观察；
- MSRV 由 CI 固定工具链宣布（当前 `OPEN`）；
- 文档：每个 `SPEC-*` 条目有失败模式与可运行示例；示例跑通率 100%；
- 无已知的「文档说支持、实际 Unsupported」的不一致。

**承诺的边界**：稳定性承诺只覆盖 **API 形制与行为契约**，不覆盖**视觉像素级一致**（字体/DPI/后端差异必须在能力矩阵中公开），也不覆盖**性能数字**（性能只按 `SPEC-SCOPE-023` 的方式报告）。

## 5. 成功指标

把开发者体验当产品指标。每项给出定义与当前值；当前值缺失即 `N/A`，不得估填。

| 指标 | 定义 | 测量方式 | 当前 |
|---|---|---|---|
| Time to first window | 从空目录到出现首个窗口的分钟数（按官方 quickstart 原文操作） | 每季度真人计时，记录环境 | `N/A`（无可运行 launcher，`OPEN`） |
| 示例跑通率 | 文档中承诺可运行的示例中，实际 `cargo run` 成功的比例 | CI 逐条执行 | `0/1`（`examples/todolist` 为目标预览，`SPEC-SCOPE-004`） |
| 首次编译时间 | 干净 target 下 `cargo build` 到可运行 demo 的墙钟时间 | 固定机器 + 固定工具链 | `N/A` |
| issue 重复困惑 | 同一 API 误用重复出现的 issue 数；目标是靠文档条目消除而非靠回答 | 每月人工归类 | `N/A`（尚无 issue 基线） |
| 文档缺口 | 有 `SPEC-*` 条目但无失败模式/示例的比例 | 脚本扫描 spec 目录 | 统计中 |
| 生态 crate 数 | 依赖 zenui 的第三方 crate 数 | crates.io 反向依赖 | `0`（未发布） |
| 破坏性变更面 | 每个 minor 的 breaking 条目数 | CHANGELOG 统计 | `N/A` |

指标的目标值**不在此处编造**：首个可发布版本后按实测基线设定。

## 6. 当前状态（诚实声明）

- 已有：rstml 解析形状、少量 typed utility、平坦 sink 更新、Parley/Taffy/CPU 与两条 headless GPU 路径的**探针级**验证（`PROBE`）。
- 尚无：生产 Signal/Store/Memo/Owner/事务、完整组件宏与 For/Show、真实 winit 窗口/IME/AccessKit、三平台实机、真实 glTF、性能数据。
- 因此本文的定位与承诺均为 `SPEC`；产品尚未具备对外发布条件。
