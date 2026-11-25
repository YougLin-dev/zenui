# Stage-three 最终跨方向验收

本页记录 #220 的最终可见验收：在同一个提交上，把模板首跑、release 拷贝、Todo 长列表滚动、UIA 动作与 ScrollPattern、菜单/快捷键/上下文菜单、Explorer 拖放、托盘、Narrator Speech Recap，以及一次 GPU 复位尝试串成一轮可见运行，并把各方向专题验收页汇总为索引。它是跨方向整合证据，不替代各专题页的边界用例。

## 基线与环境

- 基线提交：`ad7538d`。本页与 `证据/final-cross/` 只增加文档与截图，不修改代码。
- 日期：2026-11-25（本机时区）。
- 系统：Windows 11 Pro `10.0.26300` x64，单显示器 2560×1600 @240 Hz。
- 比例：150% 运行使用 144 DPI；100% 运行使用 96 DPI。切换是单显示器上的比例切换，不是混合 DPI。
- GPU：NVIDIA GeForce RTX 4060 Laptop GPU，驱动 32.0.15.7283。同一机器的隐藏宿主适配器信息为 RTX 4060、`fallback=false`（完整 `GpuAdapterInfo` 见[设备丢失实机尝试](stage-three-设备丢失-验收.md)）；可见宿主使用同一“硬件优先、无适配器时 WARP”循环。
- 构建：`todo_host`、`menu_host` 与 `tray_host` 为 debug 示例；模板为 release 构建。所有命令从 `ad7538d` 的 checkout 运行，工作树在运行前干净。

## 150% 运行

### 模板首跑与 release 拷贝

模板在仓库内以 release 构建启动（`cargo build --release --manifest-path templates/minimal-app/Cargo.toml --locked` 的产物）；窗口标题 `ZenUI Starter`，UIA 读取 `Count: 0`，Invoke `Increment counter` 后为 `Count: 1`，`WM_CLOSE` 正常退出。客户区 630×420，DPI 144。

release 拷贝路线：把 `templates/minimal-app` 复制到仓库外的 `E:\code\mine\zenui-220\hello-zenui`，只把两个 path 依赖改到 checkout，`cargo build --release --locked` 后把 `minimal-app.exe` 复制到另一个目录。从该目录用 `Start-Process`（ShellExecute，双击等价）启动，进程路径与拷贝路径一致；UIA 同样完成 `Count: 0` → `Count: 1` 并正常退出。exe 大小 15,716,864 字节，PE 子系统 2（Windows GUI），SHA-256 `2A4F31F37E9D6F03D1600CFAE8A73D3211A79BA8A6B28E61544D071551F2E3B7`。release 路径与 CRT/未签名边界的完整记录见[发布打包验收](stage-three-发布打包-验收.md)。

证据：[01-template-starter](证据/final-cross/150/01-template-starter.png)、[02-template-incremented](证据/final-cross/150/02-template-incremented.png)、[03-release-copy-starter](证据/final-cross/150/03-release-copy-starter.png)、[04-release-copy-incremented](证据/final-cross/150/04-release-copy-incremented.png)。

### Todo：UIA 动作、10,000 行与滚动

`todo_host` 的客户区为 1350×1050（144 DPI）。UIA 客户端把 `What needs to be done?` 设为 `UIA action` 并 Invoke `Add`，剩余计数从 1 变为 2；Toggle `Complete Read docs` 回到 1；随后 Invoke `Reverse`。这验证 Intention→Add、Toggle 与列表动作走同一条命令路径（[辅助技术动作验收](stage-three-辅助技术动作-验收.md)）。

10,000 行通过真实的 `IFileOpenDialog` 导入：逐字符输入路径会被对话框当成目录导航，因此本轮在对话框的文件列表中双击 `import-10k.txt`。导入完成后剩余计数为 10001（2 行种子数据 + 10,000 行导入，其中 1 行种子已完成），行带总数为 10002。窗口根的 UIA `ScrollPattern` 报告 `VerticallyScrollable=True`、`VerticalViewSize=0.189...`；`SetScrollPercent` 依次到 50、100、0。滚轮 5 次 ×24 单位到 1.948%，10 次 ×120 到 21.428%；End 到 100%，Home 回 0%，PageDown 到 18.289%。

证据：[05-todo-uia-actions](证据/final-cross/150/05-todo-uia-actions.png)、[06-todo-10k-top](证据/final-cross/150/06-todo-10k-top.png)、[07-todo-10k-pct50](证据/final-cross/150/07-todo-10k-pct50.png)、[08-todo-10k-pct100](证据/final-cross/150/08-todo-10k-pct100.png)、[09-todo-10k-wheel120](证据/final-cross/150/09-todo-10k-wheel120.png)、[10-todo-10k-end](证据/final-cross/150/10-todo-10k-end.png)、[11-todo-10k-pagedown](证据/final-cross/150/11-todo-10k-pagedown.png)。完整滚动边界见[滚动总验收](stage-three-滚动总验收.md)、[宿主滚动验收](stage-three-宿主滚动-验收.md)与[窗口化 For 验收](stage-three-窗口化-for-验收.md)。

### 菜单、快捷键与上下文菜单

`menu_host`（窗口 `ZenUI Menu`，840×480 客户区，144 DPI）从菜单栏打开 File，看到 Save / Clear / Close 与 Save 的 `Ctrl+S` 标注；File > Clear 后状态文本为 `Cleared`；View > Show status 后为 `Status hidden`；Shift+F10 打开窗口级上下文菜单，选择 Clear note 后状态为 `Note cleared`，note 输入被清空。Ctrl+S 在本轮按下；其 `Saved` 状态的截图由[快捷键验收](stage-three-快捷键-验收.md)承担。

证据：[12-menu-initial](证据/final-cross/150/12-menu-initial.png)、[13-menu-file-open](证据/final-cross/150/13-menu-file-open.png)、[14-menu-cleared](证据/final-cross/150/14-menu-cleared.png)、[15-menu-view-toggled](证据/final-cross/150/15-menu-view-toggled.png)、[16-menu-context](证据/final-cross/150/16-menu-context.png)、[17-menu-context-cleared](证据/final-cross/150/17-menu-context-cleared.png)。

### GPU 复位尝试

在 Todo 有草稿 `final acceptance draft` 且输入聚焦时发送一次 Win+Ctrl+Shift+B 组合（tap）。进程与 HWND 保持存活，UIA 读回的草稿与焦点不变；`System` 日志在尝试窗口内没有 Display/TDR 或 `nvlddmkm` 事件。本轮记录为“未观察到设备丢失”，不声称恢复路径被触发。完整尝试与边界见[设备丢失实机尝试](stage-three-设备丢失-验收.md)。

证据：[18-device-loss-before](证据/final-cross/150/18-device-loss-before.png)、[19-device-loss-after](证据/final-cross/150/19-device-loss-after.png)。

## 100% 运行

在单显示器上把比例切到 100%（96 DPI）后重跑同一组紧凑序列，并补做 Explorer 拖放、托盘与 Narrator 三项跨方向检查。

- 模板：客户区 420×280；UIA `Count: 0` → Invoke `Increment counter` → `Count: 1`；正常退出。release 拷贝同样为 420×280，进程路径与拷贝一致。
- Todo：客户区 900×700；UIA SetValue + Invoke Add → 2 remaining，Toggle → 1，Reverse；真实对话框导入 10,000 行后为 10001 remaining。
- `ScrollPattern` 的垂直 view size 与 150% 相同（0.189...），`SetScrollPercent` 0→50→100→0；滚轮 5×24 到 1.948%、10×120 到 21.428%，End/Home/PageDown 到 100/0/18.289%。这些是内容比例，跨比例一致。
- `TextPattern` 读取 `a中éz`，caret selection 有 1 个矩形，Select 全选后读回同一文本。
- 菜单：窗口 576×379 frame / 560×320 client；File（Save Ctrl+S/Clear/Close）、Clear → `Cleared`、View → `Status hidden`、Shift+F10 → Clear note → `Note cleared`。

证据：[01-template-starter](证据/final-cross/100/01-template-starter.png)、[02-template-incremented](证据/final-cross/100/02-template-incremented.png)、[03-release-copy-starter](证据/final-cross/100/03-release-copy-starter.png)、[04-release-copy-incremented](证据/final-cross/100/04-release-copy-incremented.png)、[05-todo-uia-actions](证据/final-cross/100/05-todo-uia-actions.png)、[06-todo-10k-top](证据/final-cross/100/06-todo-10k-top.png)、[07-todo-10k-pct50](证据/final-cross/100/07-todo-10k-pct50.png)、[08-todo-10k-pct100](证据/final-cross/100/08-todo-10k-pct100.png)、[09-todo-10k-wheel120](证据/final-cross/100/09-todo-10k-wheel120.png)、[10-todo-10k-end](证据/final-cross/100/10-todo-10k-end.png)、[11-todo-10k-pagedown](证据/final-cross/100/11-todo-10k-pagedown.png)、[12-todo-textpattern](证据/final-cross/100/12-todo-textpattern.png)、[13-menu-initial](证据/final-cross/100/13-menu-initial.png)、[14-menu-file-open](证据/final-cross/100/14-menu-file-open.png)、[15-menu-cleared](证据/final-cross/100/15-menu-cleared.png)、[16-menu-view-toggled](证据/final-cross/100/16-menu-view-toggled.png)、[17-menu-context](证据/final-cross/100/17-menu-context.png)、[18-menu-context-cleared](证据/final-cross/100/18-menu-context-cleared.png)；状态见 `证据/final-cross/100/states.txt`。

### Explorer 拖放、托盘与 Narrator

- **Explorer 拖放**：把仓库外 `zenui-220` 文件夹里的 `drop-final.txt`（两行标题）从文件资源管理器用真实鼠标拖拽进 Todo 客户区。UIA 在拖放后读到 `Drop final A` 与 `Drop final B` 两行；截图同时显示提示 `Imported 2 tasks` 与剩余计数 3（1 行未完成的种子任务 + 2 行导入）。窗口随后按 `WM_CLOSE` 正常退出。拖放与文件对话框导入共用应用级读取/导入 worker。
- **托盘**：`tray_host` 的图标进入 Windows 11 溢出面板，UIA 名称为 ` ZenUI Tray`（前导空格由系统提供），矩形 40×40。右键菜单包含 Show window、Reset status、Hide window、Close window、Quit。菜单 Show window 后状态文本为 `Restored from the tray`；Reset status 后为 `Reset from the tray menu`；UIA `Hide to tray` 与菜单 Hide window 都隐藏窗口，随后左键点击图标恢复，恢复走 `Runtime::show` 而不派发 `tray.show`，所以状态文本保留点击前的 `Hidden to the tray`；Quit 结束进程。菜单截图把与菜单无关的第三方屏幕工具浮动条区域填充为菜单背景色，全部菜单项文本可见。
- **Narrator**：讲述人（zh-CN）运行期间，Todo 的焦点从草稿 `What needs to be done?` 依次 Tab 到 `Add`、`All`、`Active`；`Narrator+Alt+X` 打开的 Speech Recap 窗口（标题 `语音回顾`，660×500 物理像素）记录了 `ZenUI Todo 窗口, Add task 组, Add, 按钮.`、`Filter 组, All, 按钮.`、`Active, 按钮` 三行。截图是 Recap 底部的裁剪，只保留这三行；讲述人主页与其他应用的朗读行不在证据内。本轮没有采集音频，不声称逐字朗读内容、语速或朗读次数；截图后关闭 Recap 窗口并停止讲述人。

证据：[19-todo-drop](证据/final-cross/100/19-todo-drop.png)、[20-tray-icon](证据/final-cross/100/20-tray-icon.png)、[21-tray-menu](证据/final-cross/100/21-tray-menu.png)、[22-tray-show](证据/final-cross/100/22-tray-show.png)、[23-tray-reset](证据/final-cross/100/23-tray-reset.png)、[24-tray-leftclick](证据/final-cross/100/24-tray-leftclick.png)、[25-narrator-recap](证据/final-cross/100/25-narrator-recap.png)。

## 各方向专题证据索引

各专题页记录各自基线与 100%/150% 覆盖；本页只汇总入口，不重复其边界矩阵。

| 方向 | 专题页 | 覆盖 |
| --- | --- | --- |
| 模板与发布 | [空目录启动](stage-three-空目录启动-验收.md)、[发布打包](stage-three-发布打包-验收.md) | 仓库外复制、UIA 计数、release 子系统与双击路径 |
| 滚动与长列表 | [宿主滚动](stage-three-宿主滚动-验收.md)、[滚动总验收](stage-three-滚动总验收.md)、[焦点显露](stage-three-焦点显露-验收.md)、[滚动条](stage-three-滚动条-验收.md)、[窗口化 For](stage-three-窗口化-for-验收.md) | 滚轮精度、翻页、边界、显露、滚动条与 10,000 行窗口化 |
| UIA | [只读树](stage-three-辅助技术-验收.md)、[动作](stage-three-辅助技术动作-验收.md)、[事件](stage-three-辅助技术事件-验收.md)、[文本](stage-three-辅助技术文本-验收.md)、[滚动](stage-three-辅助技术滚动-验收.md)、[总验收](stage-three-辅助技术总验收.md) | 树、动作、事件、TextPattern、ScrollPattern、Inspect/Narrator |
| 菜单、托盘与会话 | [菜单栏](stage-three-菜单栏-验收.md)、[快捷键](stage-three-快捷键-验收.md)、[上下文菜单](stage-three-上下文菜单-验收.md)、[托盘](stage-three-托盘-验收.md)、[会话](stage-three-会话-验收.md) | 原生菜单、HACCEL、弹出菜单、托盘恢复与常驻会话 |
| 剪贴板与编辑历史 | [剪贴板](stage-three-剪贴板-验收.md)、[文本编辑历史](stage-three-文本编辑历史-验收.md) | Notepad 互操作、Ctrl+C/X/V、Ctrl+Z/Y 与合并步 |
| 文件拖放 | [文件拖放](stage-three-文件拖放-验收.md) | Explorer 单/多文件、Unicode/长路径、文件夹与非文件拒绝 |
| 设备丢失 | [设备丢失](stage-three-设备丢失-验收.md) | 真实 GPU 复位尝试、事件日志与恢复边界 |
| 窗口与对话框 | [多窗口](stage-three-多窗口-验收.md)、[文件对话框](stage-three-文件对话框-验收.md)、[保存与文件夹](stage-three-保存与文件夹-验收.md) | 多窗口、打开/保存/文件夹对话框、应用所有权结果 |

## 证据文件

`证据/final-cross/150/states.txt` 与 `100/states.txt` 记录各自的 commit、环境、客户区、UIA 名称、滚动百分比与事件计数。截图均为客户区或窗口裁剪；菜单弹窗裁剪到窗口与弹出菜单区域，不包含桌面、账号或未脱敏文件列表。100% 的托盘菜单截图覆盖了与菜单无关的第三方浮动工具条区域；Narrator 截图裁剪掉讲述人主页与其他应用的朗读行。

## 边界

- 单显示器上的 100%/150% 切换，不是混合 DPI；跨显示器混合 DPI 仍是 #63 的开放缺口。
- 本页是跨方向 smoke：滚轮精度、显露、滚动条、IME 候选、拖放的多文件/Unicode/长路径/拒绝路径，以及托盘 Explorer 重建边界仍由各专题页承担。
- 本轮补做了 Explorer 单文件拖放、托盘菜单/恢复/退出与 Narrator Speech Recap；Inspect 的树、边界与滚动后几何仍以[只读树验收](stage-three-辅助技术-验收.md)与[辅助技术总验收](stage-three-辅助技术总验收.md)为准，本轮没有重开 Inspect。剪贴板与 Notepad 互操作仍由[剪贴板验收](stage-three-剪贴板-验收.md)承担。
- Narrator 的逐字短语只来自 Speech Recap 底部裁剪；不声称朗读音频、语速或朗读次数。
- GPU 复位未观察到丢失，记录的是 no-loss 观察，不是恢复成功。
- Hidden/WARP/合成输入只作 partial evidence；本页的可见运行使用真实 HWND 与真实可变刷新显示器。
