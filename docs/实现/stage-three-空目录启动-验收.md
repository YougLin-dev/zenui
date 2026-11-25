# Stage-three 空目录启动验收（实机记录）

本页记录 #215 的可见 Windows 验收：把 `templates/minimal-app` 复制到仓库之外，只把两个 path 依赖改到 checkout，构建并打开 `ZenUI Starter`。环境要求、两种依赖路线的步骤与 CI 边界见 [从空目录开始](../SDK/从空目录开始.md)；本页只记录可见运行。

**状态：可见运行完成（单屏 150%）。** CI 在 Ubuntu 上复制模板、改写 path 依赖并构建/测试，不显示窗口；Windows 的可见窗口记录在本页，两者互不替代。

验收日期：2026-11-25。应用使用 `origin/main` d60299d 的模板源码（本轮改动只有文档与 CI），debug 构建。

## 环境

| 项 | 值 |
| --- | --- |
| 操作系统 | Windows 11 专业版 10.0.26300，64 位，交互式桌面会话 |
| 显示 | 单显示器 2560×1600，150%（`GetDpiForWindow=144`） |
| 图形 | NVIDIA GeForce RTX 4060 Laptop GPU，驱动 32.0.15.7283；宿主使用默认适配器（本机 #214 的读数为 DX12、非 WARP；本轮未重复读取） |
| 应用来源 | 模板复制到仓库外目录；`Cargo.toml` 的两个 path 依赖指向 checkout |
| 构建 | debug；`cargo build --locked`，源码与 target 都在仓库外 |
| 输入 | UIA `InvokePattern.Invoke()`，不是人工鼠标键盘 |

## 方法

- 按[从空目录开始](../SDK/从空目录开始.md)的路线 A 复制模板，改写两个 path 依赖；用 `cargo build --locked` 构建仓库外的副本。
- 启动副本的可执行文件；等 UIA 树出现 `Increment counter` 后读取窗口标题、客户区、DPI 与全部 UIA 元素名称。
- 用 `PrintWindow(PW_CLIENTONLY | PW_RENDERFULLCONTENT)` 截取客户区（01-starter.png）。
- 通过 UIA `InvokePattern` 按下按钮，轮询元素名称直到 `Count: 1`，再次截图（02-incremented.png），最后用 `WM_CLOSE` 关闭。
- 外部驱动脚本与副本都位于仓库外，未提交；应用源码未修改，只有两个 path 依赖改为 checkout。

## 结果

| # | 步骤 | 读数 | 证据 |
| --- | --- | --- | --- |
| 1 | 启动 | 标题 `ZenUI Starter`；客户区 630×420，144 DPI，可见；UIA 名称 `Count: 0`、`Increment counter`（按钮可见文本 `Increment`） | [01-starter.png](证据/empty-directory/01-starter.png) |
| 2 | UIA 按下 `Increment counter` | UIA 名称 `Count: 1`、`Increment counter`；窗口保持可见 | [02-incremented.png](证据/empty-directory/02-incremented.png) |
| 3 | `WM_CLOSE` | 进程正常退出 | [states.txt](证据/empty-directory/states.txt) |

两张截图的客户区都是 630×420 物理像素（420×280 逻辑像素 × 1.5），只含应用客户区，不含桌面、标题栏、用户名或本机路径。完整状态见 states.txt。

## 证据边界

- **CI 与可见运行分开：** 本页的截图与状态来自 Windows 11 的可见运行；CI 的 copy-out 与 git 解析步骤在 Ubuntu 上构建/测试，不显示窗口，详见[从空目录开始](../SDK/从空目录开始.md)的验证边界。Windows leg 的模板测试也不打开可见窗口；隐藏 smoke 只做隐藏窗口读回，宿主在没有硬件适配器时回退 WARP。
- **路线 B 是构建证据：** 同一份模板 `src/` 与 git 依赖（包名 `minimal-app`，`rev` 指向 d60299d）在本机构建通过，耗时约 3 分钟；没有为它单独截取可见窗口。本页的窗口证据来自路线 A；两条路线共享同一份界面源码，本页不据此声称两条路线的窗口行为不同。
- **单屏：** 只在单屏 2560×1600、150% 下运行；不声称其他缩放、多显示器或跨屏拖动。混合 DPI 仍是 #63。
- **输入：** 计数变化由 UIA 动作队列驱动，不是人工鼠标；截图证明客户区像素与 UIA 读数，不证明用户注视下的 DWM 时序。
- **脚本：** 驱动脚本、复制副本与构建目录都在仓库外，未提交；仓库不保留临时探针。
