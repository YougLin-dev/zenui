# Stage-two 工具栏换行验收（实机）

本页记录 #290：窗口窄于工具栏内容宽度时，按钮不再被裁掉，而是按行换行。实现为通用布局能力 `flex-wrap`，主窗口与 Details 窗口的工具栏都使用它。

**状态：已完成；900×700 布局不变，420×700 在 100% 与 150% 下按钮换行、标签可读、无裁剪。**

验收日期：2026-11-26。功能基线为 `main` 的 `deb7e60`（#311 合并提交）加本工单的修改。

## 环境

| 项目 | 读数 |
| --- | --- |
| 显示 | 单屏 2560×1600；150%（144 DPI）与 100%（96 DPI） |
| 示例 | `crates/windows/examples/todo_host.rs`，debug 构建 |
| 窗口尺寸 | 客户区 900×700 与 420×700 逻辑像素，`SetWindowPos` 精确设置并回读 |
| 采集 | 每线程 per-monitor V2；`GetClientRect` + `ClientToScreen` + `CopyFromScreen` |

## 方法

- 样式层新增 `flex-wrap` 工具类，映射到 `Property::FlexWrap` / `StyleValue::Wrap`；渲染器把它应用到 Taffy 的 `FlexWrap::Wrap`。
- demo 的两个工具栏视图（主窗口与 Details 窗口各两处）改为 `gap-3 flex-wrap`。
- 隐藏回归测试 `crates/render/tests/scene.rs::a_narrow_toolbar_wraps_instead_of_clipping`：420 逻辑像素宽下断言六个工具栏按钮都落在视口内，并且至少出现两个不同的行 y。
- 样式单元测试 `layout_utilities_map_to_their_properties` 覆盖 `flex-wrap` 的 token 解析。
- 可见运行：分别在 150% 与 100% 下把客户区设为 900×700 与 420×700，截图客户区。

## 读数

| 缩放 | 客户区 | 结果 | 截图 |
| --- | --- | --- | --- |
| 150% | 900×700 | 与改动前相同：工具栏各自一行 | [toolbar-150-900x700.png](证据/toolbar-wrap/toolbar-150-900x700.png) |
| 150% | 420×700 | 第一工具栏分成两行（Reverse / Clear done / Load sample；Import file / Theme / Details），第二工具栏一行，全部在客户区内 | [toolbar-150-420x700.png](证据/toolbar-wrap/toolbar-150-420x700.png) |
| 100% | 900×700 | 与改动前相同 | [toolbar-100-900x700.png](证据/toolbar-wrap/toolbar-100-900x700.png) |
| 100% | 420×700 | 同样的两行换行，标签单行可读 | [toolbar-100-420x700.png](证据/toolbar-wrap/toolbar-100-420x700.png) |

## 判据对照

| 要求 | 状态 | 证据 |
| --- | --- | --- |
| 窄目标宽度下没有按钮被裁剪，标签可读 | 通过 | 420×700 的 100%/150% 截图；场景测试断言按钮矩形都在 420 内且至少两行 |
| 900×700 布局不变或记录有意的变化 | 通过 | 两张 900×700 截图与改动前的工具栏行一致 |
| 两个宽度的可见截图；更新 stage-two 已知限制 | 通过 | 本页与 [stage-two 验收](stage-two-验收.md) 的「已知限制」 |
| fmt、clippy `-D warnings`、串行 workspace 测试 | 通过 | 本工单 PR 的本地与 CI 门槛 |

## 边界

- 目标窄宽度为 420 逻辑像素；更窄时工具栏继续换行，但窗口没有最小尺寸约束，极端窄宽度下表单与列表仍可能压缩。
- 仅工具栏：表单输入行与行内编辑没有加入换行策略。
- 截图只覆盖主窗口；Details 窗口共用相同的工具栏类，未单独截图。
