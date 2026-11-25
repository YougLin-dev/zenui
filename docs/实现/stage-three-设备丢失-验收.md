# Stage-three 设备丢失实机尝试（单显示器）

本页记录 #214 的单屏实机尝试：在可见 HWND、真实 NVIDIA 适配器与 DX12 上执行真实 GPU 丢失触发，观察宿主是否收到设备丢失并恢复。恢复策略与边界见 [ADR-0017](../adr/0017-device-loss-recovery.md)。

**状态：已尝试真实 GPU 丢失触发（Win+Ctrl+Shift+B 与同适配器挂起），未观察到设备丢失；本页不宣称恢复。** #211–#213 的隐藏测试覆盖合成重建路径；真实硬件的恢复仍无证据，跨显示器混合 DPI 仍属 #63。CI 的隐藏 smoke 与 WARP 结果是部分证据，不是本页的原生验收。

验收日期：2026-11-25。应用为提交 `4d0ea63` 的 `crates/windows/examples/todo_host.rs`（debug 构建）。探针打印、截图/触发脚本与强制挂起 helper 已删除。

## 环境

| 项 | 值 |
| --- | --- |
| 操作系统 | Windows 11 专业版 10.0.26300，64 位，交互式会话 |
| GPU / 驱动 | NVIDIA GeForce RTX 4060 Laptop GPU，驱动 32.0.15.7283 |
| 图形后端 | DX12；探针读数 `GpuAdapterInfo { name: "NVIDIA GeForce RTX 4060 Laptop GPU", backend: "Dx12", device_type: "DiscreteGpu", driver: "32.0.15.7283", fallback: false }`，不是 WARP |
| 显示 | 单显示器 2560×1600，150%（144 DPI） |
| 应用 | `todo_host`，`HostConfig::visible()`，900×700 逻辑客户区，真实 GPU 呈现 |
| 输入 | 原生窗口消息（点击、`WM_CHAR`、Shift+Left）；触发使用 `keybd_event` 与临时 GPU helper，不是人工鼠标键盘 |

## 方法

- 探针在宿主打印 `adapter_info()`、设备丢失回调、`DEVICE_LOSS` 消息、恢复尝试，以及每次成功呈现的代际与阶段；这些打印是临时源码改动，采集后移除，提交的宿主源码与运行基线只差这些打印。
- 状态通过 UI Automation 读取：草稿值、selection、焦点元素与任务文本；截图为 `PrintWindow(PW_CLIENTONLY | PW_RENDERFULLCONTENT)` 的应用客户区，不含桌面、标题栏、用户名或本机路径。
- 触发 A：`keybd_event` 发送 Win+Ctrl+Shift+B（轻按一次、按住 4 秒一次），触发后查询 System 日志的 Display provider 事件。
- 触发 B：临时 helper 进程在同一高性能适配器上创建 wgpu 设备，提交一个持续写 storage buffer、永不结束的 fragment 着色器，等待 10 秒后退出。该触发不需要管理员权限；它请求 GPU 挂起，驱动可能只记录错误事件而不执行 TDR。
- 触发后通过 `WM_CHAR` 输入 `X`，点击 `Complete Write tests` 复选框把 `Write tests` 从完成改为未完成（`1 remaining`→`2 remaining`），再点击 `Active` 过滤，证明输入、模型与过滤动作仍能呈现。

## 结果

| # | 步骤 | 读数 | 证据 |
| --- | --- | --- | --- |
| 1 | 准备可见窗口 | `draft='recovery draft'`（14 字符），`selection=' draft'`（8..14），caret 在活动端 8（截图中的竖线）；焦点在草稿；任务 `Write tests` / `Read docs`，`1 remaining`；HWND `0x4660A8E` | [01-before.png](证据/device-loss/01-before.png) |
| 2 | 触发 A：Win+Ctrl+Shift+B 轻按与按住 | 没有回调、`Lost`/`Validation` 或重建；HWND、draft、selection、焦点与任务内容不变；Display 事件 0 | [states.txt](证据/device-loss/states.txt) |
| 3 | 触发 B：同适配器 fragment shader 挂起 | NVIDIA 驱动在 12:53:12.728 记录 `nvlddmkm` 事件 153（`Error occurred on GPUID: 100`），没有 4101/TDR 或设备移除；应用设备未被移除，仍是 `generation=1`，没有回调、`Lost`/`Validation` 或重建；状态与触发前相同 | [02-after-attempt.png](证据/device-loss/02-after-attempt.png) |
| 4 | 触发后输入 `X`、点击 `Complete Write tests` 与 `Active` | `draft='recoveryX'`，`Write tests` 取消完成（`2 remaining`），`Active` 选中且列表显示两项；输入、模型与过滤动作正常呈现 | [03-after-input-filter.png](证据/device-loss/03-after-input-filter.png) |

触发前的 draft、selection、焦点、HWND 与任务内容在触发后逐项相同；探针日志的 52 条呈现全部是 `generation=1 phase=Clear`，没有 `callback`、`DEVICE_LOSS` 或 `recover` 行。01 与 02 的选区底色和 caret 闪烁相位不同，内容状态由 UI Automation 与 `states.txt` 证实相同，不把截图逐像素相等作为断言。触发 B 的驱动事件说明真实 GPU 错误发生了，但没有 TDR/重置或 D3D12 设备移除，宿主的丢失与恢复路径没有被执行。

## 证据边界

- **未观察到设备丢失，因此没有恢复证据。** 真实硬件的恢复仍只有合成路径（`Device::destroy` 等）的隐藏测试，不能把本页当作原生恢复验收。
- **未尝试的触发：** GPU 禁用/启用与驱动服务重启需要管理员权限；RDP 会话切换需要第二个会话。当前会话不满足这些条件，它们仍是后续真机证据的入口。
- **设备级事件不等于设备丢失：** 触发 B 的 `nvlddmkm` 事件 153 是真实驱动错误记录，不是 TDR/重置或设备移除的证据；应用继续在 `generation=1` 上呈现；本页不把它记为设备丢失。
- **单屏声明：** 只在单屏 2560×1600、150% 下运行；不声称多显示器、混合 DPI 或跨屏拖动。跨显示器混合 DPI 与真实硬件设备丢失仍属 #63。
- **部分证据：** 隐藏 smoke 与 CI WARP 只覆盖隐藏窗口的合成重建，不显示窗口，也不代替本页。
- **脱敏与探针：** 截图只含应用客户区；临时探针打印已回退，截图/触发脚本与 GPU helper 已删除，仓库与 `target/` 采集目录都不保留这些探针。
