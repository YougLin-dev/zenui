# Stage-three 系统偏好验收（高对比度 + 文本缩放，实机）

本页记录 #297：把高对比度与文本缩放放在同一次可见运行中验收，记录实时切换、UIA 边界与系统窗口边框，并恢复原设置。分项读数见 [高对比度验收](stage-three-高对比度-验收.md) 与 [文本缩放验收](stage-three-文本缩放-验收.md)；契约见 [ADR-0022](../adr/0022-system-preferences.md)。

**状态：已完成；150% 显示、900×700 窗口下高对比度开/关与文本 100%↔225% 实时切换，系统窗口边框随高对比度变化，焦点与 selection 保留；结束后关闭高对比度、移除文本缩放值并恢复原注册表。**

验收日期：2026-11-26。功能基线为 `main` 的 `820fd21`（#296 合并提交）加本工单的文档修改。

## 环境

| 项目 | 读数 |
| --- | --- |
| 系统 | Windows 11 Pro 10.0.26300，单屏 2560×1600 |
| 显示 | 150%（144 DPI） |
| 高对比度 | 高对比黑色；运行中开/关，结束后关闭 |
| 文本缩放 | 起始 100%，运行中切到 225%，结束后移除注册表值 |
| 示例 | `crates/windows/examples/todo_host.rs`，debug 构建 |
| 窗口 | 客户区 900×700 逻辑像素；整窗截图 1372×1106 物理像素 |
| 采集 | 每线程 per-monitor V2；`GetWindowRect` + `CopyFromScreen`；UIA `BoundingRectangle` 读按钮边界 |

## 方法

- 宿主在启动与 `WM_SETTINGCHANGE` 时采样高对比度与文本缩放；`WM_SETTINGCHANGE` 由 `SendMessageTimeout(HWND_BROADCAST, ..., "Accessibility")` 广播，模拟系统设置变化。
- 高对比度通过 `SystemParametersInfoW(SPI_SETHIGHCONTRAST)` 开关；关闭后恢复 `Flags = 126`、空 scheme 与原有注册表值。
- 文本缩放写入 `HKCU\Software\Microsoft\Accessibility\TextScaleFactor`，结束后移除。
- 通过 UIA 添加 `中文 😀` 行，聚焦第一行编辑框并 `Ctrl+A`，在每次切换后截取整个窗口（含系统标题栏与边框）。
- 每次切换后读取 `Add` 按钮的 UIA 边界；不重建应用模型、不改变呈现版本。

## 读数

| 状态 | Add 按钮 UIA 边界（物理像素） | 截图 |
| --- | --- | --- |
| 高对比度关，文本 100% | 93×55 | [150dpi-text-100.png](证据/text-scale/150dpi-text-100.png)（客户区） |
| 高对比度开，文本 100% | 93×55 | [150dpi-hc-text-100-window.png](证据/system-preferences/150dpi-hc-text-100-window.png) |
| 高对比度开，文本 225%（实时） | 144×91 | [150dpi-hc-text-225-window.png](证据/system-preferences/150dpi-hc-text-225-window.png) |
| 高对比度开，文本恢复 100%（实时） | 93×55 | [150dpi-hc-text-100-restored-window.png](证据/system-preferences/150dpi-hc-text-100-restored-window.png) |
| 高对比度关，文本 100%（结束） | 93×55 | 同 100% 客户区截图；系统边框恢复 |

整窗截图中，标题栏与窗口边框由 Windows 绘制，随高对比度主题变为黄色边框与高对比度标题栏；ZenUI 客户区同步切换到系统调色板。225% 下文本、行高与按钮放大，工具栏换成多行，第一行 selection 与窗口焦点保持；没有裁剪。

## 判据对照

| 要求 | 状态 | 证据 |
| --- | --- | --- |
| ADR 记录偏好来源、宿主所有权、调色板优先级、文本缩放、缓存与边界 | 通过 | [ADR-0022](../adr/0022-system-preferences.md) |
| SDK 与验证文档更新并链接 | 通过 | `docs/SDK/样式与输入.md`、`渲染场景.md`、`窗口与任务.md` 与 [验证重点](验证重点.md) |
| 组合可见运行覆盖高对比度、100%/225% 文本、实时变化、UIA 边界与系统边框 | 通过 | 上表与截图 |
| 恢复原设置 | 通过 | 高对比度 `Flags = 126`、空 scheme；`TextScaleFactor` 已移除；显示仍为 150% |
| fmt、clippy `-D warnings`、串行 workspace 测试与文档链接 | 通过 | 本工单 PR 的本地与 CI 门槛 |

## 边界

- 组合运行使用高对比黑色主题与 150% 显示；高对比白色与其他主题未单独截图。
- 系统标题栏、边框、菜单与托盘由 Windows 绘制；本页只确认标题栏/边框跟随主题，不声称 ZenUI 绘制它们。
- 显式字面颜色与固定 `px` 几何仍由应用负责；边界见 ADR-0022。
- 多显示器、混合 DPI 与运行中显示缩放仍属 #63。
- 组合截图来自 debug 示例与外部探针，探针与原始截图位于仓库外；仓库内只保留脱敏后的窗口截图。
