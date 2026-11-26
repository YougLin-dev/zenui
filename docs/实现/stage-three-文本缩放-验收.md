# Stage-three 辅助功能文本缩放验收（实机）

本页记录 #296：Windows 辅助功能文本大小（100–225%）放大 ZenUI 文本与由字号派生的几何；显示 DPI 保持独立。高对比度调色板由 #295 单独验收。

**状态：已完成；150% 显示、900×700 窗口下 100% 与 225% 实时切换，文本与行高放大、工具栏换行、无裁剪，焦点、selection 与编辑值保留；验收后已移除文本缩放设置并恢复 100%。**

验收日期：2026-11-26。功能基线为 `main` 的 `2718b48`（#295 合并提交）加本工单的修改。契约见 [ADR-0022](../adr/0022-system-preferences.md)，与高对比度的组合证据见 [系统偏好验收](stage-three-系统偏好-验收.md)。

## 环境

| 项目 | 读数 |
| --- | --- |
| 系统 | Windows 11 Pro 10.0.26300，单屏 2560×1600 |
| 显示 | 150%（144 DPI） |
| 文本缩放 | 100% 起始，运行中切到 225%，结束后恢复 100%（注册表值原本不存在） |
| 示例 | `crates/windows/examples/todo_host.rs`，debug 构建 |
| 窗口 | 客户区 900×700 逻辑像素，`SetWindowPos` 设置并回读 |
| 采集 | 每线程 per-monitor V2；`GetClientRect` + `ClientToScreen` + `CopyFromScreen` |

## 方法

- `SceneEnv` 增加整数 `text_scale_percent`，有效范围 100–225，越界回退 100；它属于宿主呈现环境，不进入 presentation snapshot。
- 样式解析在应用完所有声明后把最终字号乘以缩放比例；`em` 宽度、高度与行长因此按放大后的字号解析，固定 `px` 长度仍是逻辑像素。
- 文本布局与测量缓存键包含字号位模式，缩放变化不会复用另一比例的布局；caret、selection、命中测试、滚动范围与 UIA 边界都来自同一份布局。
- demo 的窗口化 `For` 行高与行内容高度从 `px(37.0)` 改为 `em(2.5)`（100% 下 35 逻辑点），让行随文本缩放而不是裁剪。
- 宿主在 `WM_SETTINGCHANGE` 时重新采样文本缩放；变化只标记窗口重绘，下一次场景构建使用新环境并重新夹取滚动偏移。
- 隐藏测试覆盖 100%、150% 与 225% 的字号、行高、caret、命中测试、滚动范围、缓存身份、CJK/emoji 与行带；宿主测试覆盖实时重建、编辑状态保留与滚动重新夹取。
- 可见运行通过 UIA 添加 `中文 😀` 行、聚焦第一行并 `Ctrl+A`，截取 100%；写入 `TextScaleFactor = 225` 并广播 `WM_SETTINGCHANGE`（`lParam = "Accessibility"`），截取 225%；移除该值并再次广播，截取恢复后的 100%。

## 读数

| 项目 | 100% | 225% |
| --- | --- | --- |
| 默认字号 | 14 逻辑点 | 31.5 逻辑点 |
| Add 按钮边界（UIA，物理像素） | 93×55 | 144×91 |
| 列表行高 | 35 逻辑点 | 78.75 逻辑点 |
| 工具栏 | 三行 | 五行，按钮标签完整 |
| 焦点与 selection | 第一行编辑框选中 `Write tests` | 保持选中，caret 随字号放大 |
| 中文与 emoji | `中文 😀` 完整 | 完整，无裁剪 |
| 呈现版本 | 切换前后不变 | 不变 |

## 截图

| 场景 | 截图 |
| --- | --- |
| 100% 文本缩放 | [150dpi-text-100.png](证据/text-scale/150dpi-text-100.png) |
| 225% 文本缩放（实时切换） | [150dpi-text-225.png](证据/text-scale/150dpi-text-225.png) |
| 恢复 100%（与起始帧相同） | [150dpi-text-restored.png](证据/text-scale/150dpi-text-restored.png) |

## 判据对照

| 要求 | 状态 | 证据 |
| --- | --- | --- |
| 文本缩放是场景环境中的整数百分比，驱动度量、几何与 UIA 边界 | 通过 | `text_scale_enlarges_fonts_controls_and_glyph_metrics`、`text_scale_moves_caret_and_hit_testing_boundaries`；上表 UIA 边界 |
| 缓存键包含缩放；变化后不提供旧布局 | 通过 | `a_shared_cache_does_not_serve_layouts_from_another_text_scale` |
| 实时变化保留本地编辑状态并重新夹取滚动；无 `on:input` 或模型事务 | 通过 | `a_text_scale_change_rebuilds_text_and_preserves_edit_state`、`a_text_scale_change_reclamps_the_scroll_offset`、`a_settings_broadcast_applies_a_new_text_scale` |
| 隐藏测试覆盖 100%、150%、225%、CJK/emoji、caret/selection、命中、滚动范围与无裁剪 | 通过 | 上述测试与 `cjk_and_emoji_stay_inside_scaled_text`、`text_scale_grows_row_bands_and_scroll_extent` |
| 可见 100% 与 225% 运行，并恢复原设置 | 通过 | 本页截图；注册表 `TextScaleFactor` 已移除，显示仍为 150% |
| fmt、clippy `-D warnings`、串行 workspace 测试 | 通过 | 本工单 PR 的本地与 CI 门槛 |

## 边界

- 只记录 100% 与 225% 两端；150% 由渲染测试覆盖，未单独截图。
- 固定 `px` 长度不随文本缩放变化，这是本切片的约定；依赖固定行高的应用需要改用 `em` 才能随文本增长。
- Taffy 把布局尺寸对齐到整像素；非整数行高在 100% 与 225% 下可能产生 1 像素级的间距取整。
- demo 使用系统字体，emoji 可读；模板的固定 Noto Sans SC 不含 emoji，是否会回退由字体资产决定。
- 文本缩放不改变显示 DPI、窗口逻辑尺寸或多显示器行为；跨显示器混合 DPI 仍属 #63。
- 系统菜单、托盘与 Settings 自身的文本由 Windows 绘制，不在本页范围。
