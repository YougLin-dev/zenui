# Stage-three 发布打包验收（实机记录）

本页记录 #216 的 release 打包可见验收：构建模板的 release exe，确认它使用 Windows GUI 子系统（双击不附带控制台），把 exe 复制到另一个目录，分别通过 shell 启动与 ShellExecute 双击路径启动，再用 UI Automation 读回计数。打包路径、CRT/asset/WARP/未签名边界见 [从空目录开始](../SDK/从空目录开始.md#发布构建)。

**状态：可见运行完成（单屏 150%）。** CI 在 Windows leg 构建同一 release 目标，并断言 exe 路径与 GUI 子系统；CI 不显示窗口。本页的窗口与复制运行记录独立于 CI。

验收日期：2026-11-25。基线 `origin/main` 9c3caee；release 构建使用当前模板源码，debug 构建不参与本页的可见运行，仅用于 subsystem 对照。

## 环境

| 项 | 值 |
| --- | --- |
| 操作系统 | Windows 11 专业版 10.0.26300，64 位，交互式桌面会话 |
| 显示 | 单显示器 2560×1600，150%（`GetDpiForWindow=144`） |
| 图形 | NVIDIA GeForce RTX 4060 Laptop GPU，驱动 32.0.15.7283；宿主默认适配器（本机 #214 的读数为 DX12、非 WARP；本轮未重复读取） |
| 构建 | `cargo build --release --manifest-path templates/minimal-app/Cargo.toml --locked`；产物 `templates/minimal-app/target/release/minimal-app.exe`，15,716,864 字节，SHA-256 `2a4f31f37e9d6f03d1600cfae8a73d3211a79ba8a6b28e61544d071551f2e3b7` |
| 运行位置 | exe 复制到仓库外目录；shell 启动的工作目录是另一个空目录（既不是仓库也不是 exe 目录）；ShellExecute 启动使用系统的文件打开路径 |
| 启动方式 | ShellExecute open（`Invoke-Item`，与双击相同的 shell 路径）与 shell `Start-Process` 对照；计数由 UIA `InvokePattern` 驱动 |

## 方法

- 用 release 命令构建模板；记录文件大小与 SHA-256。用 PowerShell 读取 PE 头（`e_lfanew + 0x5C`）得到 subsystem，用 `dumpbin -headers` 与 `-dependents` 检查资源目录与导入 DLL，用 `Get-AuthenticodeSignature` 检查签名。
- 把 exe 复制到仓库外的目录。
- 运行 A：在第三个空目录中用 `Start-Process` 启动副本，避免工作目录或相邻文件参与运行；等 UIA 树出现 `Increment counter` 后读取窗口标题、客户区、DPI 与 UIA 元素名称，截图（01），按下按钮，读回 `Count: 1`，再截图（02），最后 `WM_CLOSE`。
- 运行 B：用 `Invoke-Item` 走 ShellExecute 的 open 路径启动同一个副本（与资源管理器双击相同的 shell 路径），从窗口句柄取得进程路径并断言它等于副本；同样用 UIA 读回计数并截图（03/04），最后 `WM_CLOSE`。
- 外部驱动脚本、exe 副本与空工作目录都在仓库外，未提交；应用源码未修改，只有 manifest 增加 license 字段。

## 结果

| # | 步骤 | 读数 | 证据 |
| --- | --- | --- | --- |
| 1 | release PE | subsystem 2（Windows GUI）；15,716,864 字节；SHA-256 如上；Authenticode `NotSigned`；资源目录 RVA 0、size 0；导入 `VCRUNTIME140.dll` 与 6 个 `api-ms-win-crt-*` API set，无 `MSVCP140.dll` | [states.txt](证据/release/states.txt) |
| 2 | 运行 A：复制到仓库外，空工作目录中 `Start-Process` 启动 | 标题 `ZenUI Starter`；客户区 630×420，144 DPI，可见；UIA 名称 `Count: 0`、`Increment counter`（按钮可见文本 `Increment`）；按下后 `Count: 1` | [01-starter.png](证据/release/01-starter.png) · [02-incremented.png](证据/release/02-incremented.png) |
| 3 | 运行 B：ShellExecute open（双击路径）启动副本 | 进程路径与副本一致（`match=True`）；标题、客户区、DPI 同运行 A；`Count: 0` → `Count: 1`，窗口保持可见 | [03-explorer-starter.png](证据/release/03-explorer-starter.png) · [04-explorer-incremented.png](证据/release/04-explorer-incremented.png) |
| 4 | `WM_CLOSE` | 两个运行的进程都正常退出 | [states.txt](证据/release/states.txt) |

四张截图的客户区都是 630×420 物理像素（420×280 逻辑像素 × 1.5），只含应用客户区，不含桌面、标题栏、用户名或本机路径。

## 证据边界

- **无控制台：** release 的 PE subsystem 为 2（Windows GUI），debug 为 3（Windows CUI）。运行 B 走 ShellExecute 的 open verb，与资源管理器双击相同的 shell 路径；它是程序化启动，不是人工鼠标双击。本页用 PE 子系统与可见窗口证明无控制台，不把客户区截图当作“没有控制台窗口”的像素证据。
- **截图与构建：** release 与 debug 使用同一份界面源码，截图像素不能区分两者；release 身份由被启动文件的路径、SHA-256、subsystem=2、运行日志与运行 B 的进程路径断言证明。
- **CRT 依赖：** 导入表在本机观察为 `VCRUNTIME140.dll` 与 UCRT API set；目标机器需要 Visual C++ 2015–2022 Redistributable（x64）。没有在缺少该运行库的干净 Windows 安装上验证，也不声称 exe 自带 CRT。
- **可移植性：** 运行 A 的 exe 复制到另一个目录、在空工作目录启动仍显示窗口；模板不含 asset/字体文件，文本使用系统字体。没有在无 GPU、WARP-only 或干净机器上重复本页。
- **图形：** 本页使用硬件 DX12（本机 #214 读数，本轮未重复）；宿主的 WARP 回退由[从空目录开始](../SDK/从空目录开始.md#环境)记录，本页没有 WARP-only 运行。
- **未签名：** Authenticode 为 `NotSigned`；本地复制的文件没有 Mark-of-the-Web，因此本页没有触发 SmartScreen，只记录文档中的提示，不把“未触发”当作签名或 SmartScreen 行为证据。
- **不提供与不分发的内容：** exe 的资源目录为空，没有 icon、version resource 或嵌入 manifest；没有 installer、代码签名、auto-update。`target/release` 中的 PDB 与中间产物只是构建输出，不作为分发内容；分发只复制 exe 与许可证文本。
- **单屏：** 只在单屏 2560×1600、150% 下运行；不声称其他缩放、多显示器或跨屏拖动。混合 DPI 仍属 #63。
- **CI 与可见运行分开：** CI 的 `Template release build` 步骤在 Windows leg 构建并断言 `<CARGO_TARGET_DIR>/release/minimal-app.exe` 存在且 subsystem 为 2；它只验证共享 target 变体，默认 `templates/minimal-app/target/release/minimal-app.exe` 路径由本页的本地记录支持。`License files (Linux)` 步骤校验 LICENSE 文件、SPDX 声明与文档链接。两者都不显示窗口。
