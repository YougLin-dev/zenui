# Stage-three 空闲窗口重绘验收（实机）

本页记录 #288：空闲窗口不持续重建场景或呈现；只有聚焦 caret 闪烁与真实工作驱动帧。回归测试钉在 Windows 宿主 seam；本页记录可见读数与边界。

**状态：隐藏回归测试与可见 CPU 采样完成；没有把逐帧呈现计数暴露为公开 API，可见采样只是部分证据。**

验收日期：2026-11-26。功能基线为 `main` 的 `f505b22`（#287 合并提交）加本工单的测试修改。

## 环境

| 项目 | 读数 |
| --- | --- |
| 显示 | 单屏 2560×1600，150%（144 DPI） |
| 示例 | `crates/windows/examples/todo_host.rs`（窗口 `ZenUI Todo`），debug 构建 |
| 聚焦方式 | UI Automation 找到草稿框 `What needs to be done?`，真实鼠标点击 |
| 采集 | 进程 `TotalProcessorTime` 在 15 秒间隔前后采样；UIA 读取文本；`CopyFromScreen` 截图 |

## 方法

- 聚焦空闲：点击草稿框后保持 `ZenUI Todo` 为前台窗口 15 秒，不注入其它输入；caret 闪烁允许产生帧。
- 失焦空闲：启动 Notepad 并置为前台，让 `ZenUI Todo` 可见但不拥有前台 15 秒；此时 caret 不闪烁，不允许任何重绘。
- 隐藏回归测试使用仅测试期存在的场景构建/呈现计数器与前台注入：
  - `an_idle_unfocused_window_neither_rebuilds_nor_presents`：1.2 秒空闲后构建与呈现都为 0。
  - `an_idle_focused_window_presents_only_caret_blinks`：1.2 秒聚焦空闲后呈现不超过 3 次，且场景构建次数等于呈现次数——每次闪烁从缓存快照重建一次场景并呈现一次。
  - `a_rendered_change_rebuilds_and_presents`：渲染变化快照时两个计数都增加，证明计数器有效。
- 可见采样与隐藏测试互补：前者证明真实可见窗口在 15 秒内没有忙循环，后者证明计数层面的零重绘上限。

## 可见读数

完整时间戳见 [`states.txt`](证据/host-idle/states.txt)；截图 [idle-150.png](证据/host-idle/idle-150.png)。

| 阶段 | 前台窗口 | 15 秒 CPU | 结论 |
| --- | --- | --- | --- |
| 聚焦空闲 | `ZenUI Todo` | 0.046875s | 只有 caret 闪烁与缓存快照呈现的开销 |
| 失焦空闲 | Notepad | 0s | 没有重绘或后台计算 |

两种阶段的 UIA 文本都保持 `1 remaining | Write tests | Read docs`，窗口保持可响应；截图记录聚焦空闲时的客户区。

## 判据对照

| 要求 | 状态 | 证据 |
| --- | --- | --- |
| #288：空闲窗口在固定区间内不重建场景或呈现，聚焦 caret 闪烁除外 | 通过 | 两个隐藏回归测试；可见 15 秒采样 |
| #288：回归测试在空闲重绘重新引入时失败 | 通过 | 计数器断言；`a_rendered_change_rebuilds_and_presents` 证明计数随真实工作增长 |
| #288：行为已正确时只加测试与文档 | 通过 | 无生产行为修改；计数器与前台注入都只在 `cfg(test)` 下存在 |
| #288：记录可见/空闲证据，隐藏/CI 标注为部分 | 通过 | 本页与 `states.txt`；隐藏测试在 WARP/隐藏窗口上运行 |

## 边界

- 可见证据是 CPU 采样，不是呈现计数：GPU 呈现与闪烁异步，0.047 秒只说明没有忙循环，不能推导精确帧数。
- 隐藏测试的“前台”由测试注入，不覆盖真实前台切换的 DWM 时序；隐藏窗口使用 WARP 或离屏 surface。
- 未在空闲期间注入输入、任务或滚动；这些路径由既有的任务交付、滚动与重绘测试覆盖。
- 截图来自 `CopyFromScreen` 的客户区，右/下不含窗口边框；它记录空闲时的静态画面，不证明用户注视下的呈现时序。
