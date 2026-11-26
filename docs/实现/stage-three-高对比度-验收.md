# Stage-three 系统高对比度验收（实机）

本页记录 #295：Windows 高对比度开启时，ZenUI 内容改用系统颜色绘制语义角色与内建控件；应用明暗外观被覆盖，显式字面颜色保持不变。文本缩放由 #296 单独交付。

**状态：已完成；150% 单屏、Windows 高对比度黑色主题下 Todo 可读，焦点、selection 与 caret 可见，中文与 emoji 可读；验收后已关闭高对比度并恢复原设置。**

验收日期：2026-11-26。功能基线为 `main` 的 `ffdd62a`（#294 合并提交）加本工单的修改。

## 环境

| 项目 | 读数 |
| --- | --- |
| 系统 | Windows 11 Pro 10.0.26300，单屏 2560×1600 |
| 显示 | 150%（144 DPI） |
| 高对比度主题 | 高对比黑色（High Contrast Black），验收后关闭 |
| 示例 | `crates/windows/examples/todo_host.rs`，debug 构建 |
| 窗口 | 客户区 900×700 逻辑像素，`SetWindowPos` 设置并回读 |
| 采集 | 每线程 per-monitor V2；`GetClientRect` + `ClientToScreen` + `CopyFromScreen` |

## 方法

- 宿主在启动与 `WM_SETTINGCHANGE` 时采样 `SPI_GETHIGHCONTRAST`；高对比度开启时，`palette_for` 忽略应用明暗外观，改用 `RenderPalette::high_contrast`。
- 系统颜色通过 `GetSysColor` 采样：窗口、窗口文本、按钮面、按钮高亮/阴影、灰色文本、highlight/highlight text 与窗口边框。
- 语义角色映射：`surface`/`background` 用窗口色，`control` 用按钮面，`control_hover`/`control_pressed` 用按钮高亮/阴影，`text` 用窗口文本，`text_muted`/`text_disabled` 用灰色文本，`text_inverse` 用 highlight text，`accent`/`danger` 用 highlight，`border` 用窗口边框。
- `selection` 保留 80 alpha 的 highlight 覆盖，避免选中文本与背景同色；`caret` 用窗口文本。
- 显式字面颜色在 `role_color` 之外，不经过调色板。
- 隐藏测试覆盖每个角色的映射、字面颜色不变、明暗外观被覆盖，以及隐藏宿主按 `COLOR_WINDOW` 绘制；可见运行切换真实高对比度主题、截图、注入中文/emoji 行，并用 UIA 聚焦、`Ctrl+A` 选择与 `End` 放置 caret。
- 验收结束把高对比度关闭，恢复 `Flags = 126`、空的 scheme 值与清除运行期新增的注册表值。

## 读数

| 采样点 | 读数 |
| --- | --- |
| 窗口背景 | `(32, 32, 32)` 不透明 |
| 正文与按钮文本 | 白色 |
| Add / Delete / 选中筛选按钮 | 浅青 highlight `(140, 224, 237)`，文本为深色 |
| 按钮边框与输入框边框 | 系统窗口边框灰 |
| 中文与 emoji 行 | `中文 😀` 在标签中完整显示 |

## 截图

| 场景 | 截图 |
| --- | --- |
| 高对比度 Todo，900×700 | [150-high-contrast-todo.png](证据/high-contrast/150-high-contrast-todo.png) |
| 中文/emoji 行 + 第一行 `Ctrl+A` selection | [150-high-contrast-selection.png](证据/high-contrast/150-high-contrast-selection.png) |
| 聚焦输入的 caret（`Write tests` 末尾白条） | [150-high-contrast-caret.png](证据/high-contrast/150-high-contrast-caret.png) |

## 判据对照

| 要求 | 状态 | 证据 |
| --- | --- | --- |
| 语义角色与内建视觉映射到系统高对比度颜色 | 通过 | `high_contrast_maps_every_semantic_role_to_a_system_color`；上表读数 |
| 焦点、selection 与 caret 可见；中文与 emoji 可读 | 通过 | 实机截图；第一行焦点边框、selection 带与 caret 白条；`中文 😀` 行 |
| 高对比度覆盖明暗外观；字面颜色不变；普通模式不变 | 通过 | `high_contrast_overrides_the_application_appearance`、`explicit_literal_colors_are_not_remapped_by_high_contrast`；普通模式仍由既有 light/dark 测试覆盖 |
| 隐藏调色板测试与可见 Todo 记录 | 通过 | 上述测试与本页截图 |
| 偏好变化无需模型事务即可重绘 | 通过 | #294 的 `WM_SETTINGCHANGE` 测试；本工单的调色板在重绘路径读取 |
| fmt、clippy `-D warnings`、串行 workspace 测试 | 通过 | 本工单 PR 的本地与 CI 门槛 |

## 边界

- 只验证高对比黑色主题与单屏 150%；高对比白色与其他主题使用同一组系统颜色映射，未单独截图。
- `danger` 在系统高对比度中没有独立颜色，与 `accent` 共用 highlight；按钮的 hover/pressed 使用系统按钮高亮/阴影，某些主题可能弱化状态差异。
- selection 保留半透明覆盖而不是完整反色文本；选中文本自身颜色仍由 `text` 角色决定。
- 显式字面颜色不重映射，这是本切片的文档化边界；应用自定义高对比度调色板不在范围内。
- 系统菜单与托盘由 Windows 绘制，自动跟随主题，不在本页验证。
- 文本缩放、UIA 边界随缩放的几何与实机 225% 记录属于 #296/#297；多显示器与混合 DPI 仍属 #63。
