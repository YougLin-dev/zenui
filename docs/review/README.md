# 多领域评审：当前目标与真实证据入口

> [FINAL.md](FINAL.md) 是当前唯一推荐架构与目标规格，不是发布完成声明。
> `examples/todolist/main.rs` 是目标语法示例，不属于当前可运行二进制。

## 评审材料怎么读

本项目的候选目标是面向 Windows、macOS、Linux 的纯 Rust GPU UI：SolidJS 风格细粒度图、HTML 外形宏、Tailwind 词表的原生语义，以 2D 为主、可选 3D。围绕当前设计与已有探针，本目录保存了：

- 12 个独立首轮视角（A1/A2/A3、E1/E2/E3、U1/U2/U3、X1/X2、S）；A1 和 E2 初次收尾失败的记录及后续独立补充；
- 状态/语法、更新/渲染/样式、真实桌面三组交叉评审及主控反方；
- Rust、初学者、桌面可访问性与红队对统一 Todo/规格草稿的复审；
- 样式源码与 CSS cascade 的补充核验、验证矩阵及最终编辑校正。

评审不是用投票代替工程证据。原报告中的候选意见可被后续源码事实推翻；最终采纳与否见 FINAL §8，未实现或未验证的项目不因文字统一而变成已完成。

## 文档索引

1. **[FINAL.md](FINAL.md)**：唯一当前推荐规格。覆盖单一 View/Owner/reactive graph、MountPlan、Store query、事务、宏与输入、StyleIR、渲染/3D，以及 Gate A0–D。
2. **[VALIDATION.md](VALIDATION.md)**：本轮最新实际命令、成功/失败结果与证据边界。13 个已有测试、另外 3 个 compile-fail fixture、两条 headless GPU 程序、独立 Rust/RStml 夹具分别记录，不合并成虚构的完整框架测试套件。
3. **[round1-experts.md](round1-experts.md)**：独立角色首次发现、证据链与补审记录。历史意见不是最终规则。
4. **[round2-debate.md](round2-debate.md)**：三组交叉争论与主控反方，含被拒绝的候选。
5. **[round3-review.md](round3-review.md)**：统一 Todo/规格草稿后的 Rust、桌面、可访问性和红队复审。
6. **[text-layout-review.md](text-layout-review.md)**：E2 对实际 Taffy/Parley 探针的 baseline、intrinsic、Font synthesis 及 3D 能力边界核验；相应缺陷未被现有测试掩盖。
7. **[verification-contract.json](verification-contract.json)**：11 组 red/green fixture 与性能协议的机器可读规划。顶层 `status` 明示未创建的 paths、历史行号及 superseded rules；旧“hover 胜过 ordinary inline”的建议不是最终合同，普通 inline 胜出见 FINAL §6。

8. **[validation-matrix-raw.json](validation-matrix-raw.json)**：评审原始输出的历史 JSON，其顶级 `_status` 明示归档身份；不可当测试证据，也不要用旧优先级 golden 覆盖 FINAL。

## 当前实现边界

**已有探针**：

- rstml 小型 parser/lowering 与 inspection Node，不是 retained UI runtime；
- 平坦 field sink graph 与少量 typed utility，不是 generational production core；
- Taffy/Parley/CJK/CPU 与两类 wgpu headless 探针，其中 layout 已有专门记录的缺陷。

**均未完成**：

- `MountResult<MountPlan>` 的完整组件宏与生产 MountPlan；
- 生产 Signal/Store/Memo/Owner、field/query 依赖、原子事务 journal；
- typed For/Show/完整 props、TextEdit epoch/origin、真实 winit/IME/AccessKit；
- 固定版本的完整 StyleIR 支持清单、修好的 baseline/intrinsic/Font synthesis；
- macOS/Linux 实机、真实三平台 AT、glTF/私有 3D depth、生产资源生命周期与性能证据。

没有可报告的 Tailwind 覆盖百分比，也没有三平台真实应用通过结果。支持能力只由各自 Gate 的实际执行记录决定。

## 下一步

先完成 **A0** 的纯 Rust checked core/store/query/reducer 与可观测事务/fixture，再做 **A1 Windows Todo 实窗**。规划的 workspace、crate、binary 和 feature 尚不存在；规划的命令不能当作当前可执行步骤。A0 未验证 GPU 或完整宏，A1 的单 Windows 成功也不能替代 C Gate 的 macOS/Linux 与真实屏幕阅读器。
