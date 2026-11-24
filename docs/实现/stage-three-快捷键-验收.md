# Stage-three 菜单快捷键验收（实机）

本页记录 #195：菜单命令声明快捷键，宿主为每个窗口构建 `HACCEL`，两个消息循环在 `TranslateMessage` 之前调用 `TranslateAccelerator`，一次按键只产生一次命令。快捷键与菜单项共享同一个 `CommandSource::Accelerator` 派发路径和 #194 的菜单状态门。上下文菜单（#196）与托盘（#198）不在本页范围。

**状态：可见自动驱动完成（100% 与 150%）；隐藏 `HACCEL`、`TranslateAccelerator` 与编辑器让位测试只是部分证据。**

验收日期：2026-11-24。功能基线为 `main` 的 `8e19335`。可见运行使用仓库内的 `crates/windows/examples/menu_host.rs`；按键由真实 `SendInput` 注入，截图与窗口查询脚本放在仓库外。

## 环境

| 项目 | 100% | 150% |
| --- | --- | --- |
| 显示缩放 | 96 DPI | 144 DPI |
| 窗口位置 | 屏幕 (52,52) | 屏幕 (190,190) |
| 外框（`GetWindowRect`） | 576×379 | 862×566 |
| 客户区（`GetClientRect`） | 560×320 | 840×480 |
| 菜单栏 | File / View | File / View |
| 快捷键项 | `Save    Ctrl+S` | `Save    Ctrl+S` |

示例窗口的目录尺寸是 560×320 逻辑像素。`Save` 的快捷键文本由宿主放在菜单标签的制表符之后，Windows 把 `Ctrl+S` 右对齐；HACCEL 的命令 id 与菜单前序编号相同。

## 方法

- `MenuCommand::bind_shortcut` 在声明期解析快捷键并保存归一化结果；接受大小写不敏感的 `Ctrl`/`Control`、`Alt`、`Shift`，键位为 ASCII 字母、数字或 `F1`..`F24`，且至少包含 Ctrl 或 Alt。解析失败转换成 `CatalogError::InvalidMenuShortcut`。
- 一个窗口的菜单树不能出现两个归一化后相同的快捷键；不同窗口可以声明同一个快捷键。校验在 `Runtime::declare_windows` 完成时进行，与重复命令 id 和菜单上限同一入口。
- `NativeMenu` 把快捷键文本附加到菜单标签；`NativeAccelerator` 从同一命令表构建 `ACCEL` 项（`FVIRTKEY` 加 Ctrl/Alt/Shift），没有快捷键时不创建 `HACCEL`。`WindowMenu` 同时持有菜单与快捷键表，随窗口条目释放；关闭后重新打开的 activation 重建新表。
- `run` 与 `pump` 都在 `TranslateMessage` 前调用 `TranslateAcceleratorW`。匹配时由系统同步派发 `WM_COMMAND`（`HIWORD=1`、`lParam=0`），消息循环不再翻译或派发这次按键；宿主把该形式映射为 `CommandSource::Accelerator`。
- 派发前复用 #194 的菜单状态回调；禁用命令在菜单与快捷键两个入口都不运行。`HIWORD` 大于 1 或 `lParam!=0` 的 `WM_COMMAND` 不派发。
- 聚焦文本输入时，进行中的 IME 组合优先于所有快捷键；Ctrl+A/Z/Y/C/X/V 继续走编辑器策略，不进入 `TranslateAccelerator`。没有声明的 Ctrl+A 仍由运行时全选。焦点不在文本输入时，这些键可以命中声明的快捷键。
- 可见证据点击 note 输入并键入 `abc`，重新确认窗口前台后按 Ctrl+S；状态文本变为 `Saved`，输入内容保留。随后打开 File 菜单，读取右对齐的 `Ctrl+S`。

## 隐藏测试

`zenui-windows` 的 148 个库测试包含：

- `an_accelerator_table_is_built_only_for_declared_shortcuts`：无快捷键不建表；两个快捷键产生两个表项，命令 id 与前序一致，Ctrl+S 为 `FVIRTKEY|FCONTROL`，Alt+F5 为 `FVIRTKEY|FALT`。
- `a_dropped_accelerator_table_is_destroyed`：表被注销后 `CopyAcceleratorTableW` 不再报告表项。
- `keyboard_messages_translate_through_the_accelerator_table`：测量线程按下 Ctrl 后，把 `WM_KEYDOWN` 投递到窗口队列；`pump` 中的 `TranslateAccelerator` 派发一次 `CommandSource::Accelerator`，同一队列里的 Ctrl+A 仍选中全部文本，快捷键不改写文本或选择。
- `an_active_composition_shadows_the_accelerator`：聚焦输入处于组合时，Ctrl+S 不产生命令，组合保持。
- `a_composition_that_outlives_focus_still_shadows_the_accelerator`：组合在焦点移到按钮后仍未结束时，Ctrl+S 也不产生命令，组合保持。
- `accelerator_commands_use_the_accelerator_source_and_refuse_disabled`：菜单形式记录 `MenuBar`，快捷键形式记录 `Accelerator`；状态回调禁用后两个入口都不派发；恢复后快捷键再次生效。
- `reopening_rebuilds_the_accelerator_table`：关闭后窗口条目消失，重新显示创建新 HWND 与含两个表项的新表。
- `hidden_window_reports_the_declared_menu_bar`：`GetMenuStringW` 返回 `Show status\tCtrl+S`，菜单标签包含快捷键文本。
- `menu_commands_dispatch_once_and_reject_non_menu_messages`：`HIWORD=1` 与菜单形式一样恰好派发一次；`lParam!=0`、未知 id 与分隔线 id 不派发。
- `shortcuts::tests::accelerators_yield_to_a_composition_or_reserved_editor_key`：组合或聚焦输入的保留键遮蔽快捷键；无 Ctrl 或非保留键不遮蔽。无焦点输入时宿主根本不进入该策略。

根包的 7 个 `menu_shortcuts` 集成测试覆盖：大小写与修饰键顺序归一化、非法声明、命令绑定读取、同一窗口重复快捷键（含子菜单）、同一快捷键跨窗口允许、无效声明转成 `CatalogError`。全部 148 个 `zenui-windows` 库测试与 workspace 739 个测试在 2026-11-24 通过。

## 可见读数

| 缩放 | 初始窗口 | 输入 `abc` | Ctrl+S 之后 | File 菜单 |
| --- | --- | --- | --- | --- |
| 100% | [100-initial.png](证据/host-accelerator/100-initial.png) | [100-typed.png](证据/host-accelerator/100-typed.png) | [100-accelerated.png](证据/host-accelerator/100-accelerated.png) | [100-file-menu.png](证据/host-accelerator/100-file-menu.png) |
| 150% | [150-initial.png](证据/host-accelerator/150-initial.png) | [150-typed.png](证据/host-accelerator/150-typed.png) | [150-accelerated.png](证据/host-accelerator/150-accelerated.png) | [150-file-menu.png](证据/host-accelerator/150-file-menu.png) |

两个缩放都显示 note 输入保留 `abc`，状态文本从 `Ready` 变为 `Saved`；File 菜单显示 `Save` 与右对齐的 `Ctrl+S`。完整读数见 [`states.txt`](证据/host-accelerator/states.txt)。

## 判据对照

| #195 要求 | 状态 | 证据 |
| --- | --- | --- |
| 一个已声明快捷键的 `WM_KEYDOWN` 恰好执行一次命令，不同时编辑、导航或滚动 | 通过 | `keyboard_messages_translate_through_the_accelerator_table`；可见 Ctrl+S 图 |
| 文本输入聚焦时 Ctrl+S 生效 | 通过 | 可见图（note 输入聚焦且保留内容）；`accelerator_commands_use_the_accelerator_source_and_refuse_disabled` |
| 未声明的 Ctrl+A 仍全选 | 通过 | `keyboard_messages_translate_through_the_accelerator_table` |
| Escape/Enter 与进行中的 IME 组合不被抢占 | 通过 | 组合让位测试；快捷键不接受 Escape/Enter 声明 |
| 重复快捷键是声明错误 | 通过 | `duplicate_normalized_shortcuts_are_a_catalog_error`；子菜单测试；`menu_shortcuts` |
| 禁用项不触发 | 通过 | `accelerator_commands_use_the_accelerator_source_and_refuse_disabled` |
| `HACCEL` 随窗口销毁；关闭/重开没有旧表 | 通过 | `a_dropped_accelerator_table_is_destroyed`；`reopening_rebuilds_the_accelerator_table` |
| 可见 `menu_host` 按下真实快捷键 | 通过 | 八张可见图；`SendInput` 的 `Ctrl+S` |
| `cargo fmt --all -- --check`、workspace clippy、workspace tests | 通过 | PR 的 12 个必需检查与本地门禁 |

## 边界

- **键位范围：** 本切片的快捷键只接受 ASCII 字母、数字与 `F1`..`F24`；带标点的组合、媒体键与 Windows 键不声明。HACCEL 只包含声明了快捷键的命令。
- **让位范围：** IME 组合扫描窗口内的全部文本输入，焦点移动不会结束组合；Ctrl+A/Z/Y/C/X/V 的优先级只针对聚焦的文本输入。焦点在其他元素时，这些组合可以命中应用声明的快捷键。没有声明 Ctrl+A 时，编辑器照常全选。
- **采集：** 完整窗口图来自 `PrintWindow(PW_RENDERFULLCONTENT)`，右/下的黑色条是采集产物；菜单弹出区域用屏幕读取裁剪，窗口圆角外可能带一条桌面像素，不作为应用绘制。第一次 Ctrl+S 在 PowerShell 采集进程抢走前台后没有到达窗口，证据在重新置前后采集；`states.txt` 记录这一点。
- **未覆盖：** 菜单打开期间的快捷键、AltGr/Ctrl+Alt 布局、自定义键盘布局与多显示器不在本页。上下文菜单（#196）、应用级命令（#197）与托盘（#198）不在本工单。
- **单屏：** 只在单屏 2560×1600 上运行；100% 是临时切换，结束时恢复 150%。不声明多显示器或混合 DPI。

## 后续状态

本页关闭 #195 的菜单快捷键声明、HACCEL 生命周期、消息循环翻译与可见证据。#196 的上下文菜单、#198 的托盘与 #200 的契约汇总仍由后续工单交付。
