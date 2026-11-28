# 可见 Windows 验收矩阵

本页把[运行时 v2 执行方案](事件与响应式更新-方案.md) §20.5 的 Phase 要求映射到已有验收页与证据等级。矩阵是证据索引，不重新宣称运行结果：每条“已记录”都指向对应页面中的实际 Windows 运行；标为“待人工”的条目没有在 #349 中执行。

## 证据等级

| 等级 | 含义 | 能证明什么 |
|---|---|---|
| A：可见 Windows | 真实 HWND、真实输入/IME/UIA/GPU，页面记录机器、缩放与读数 | 原生呈现与交互验收 |
| B：隐藏 HWND / WARP | CI 或隐藏宿主创建窗口、软件适配器、读回像素或驱动 provider | 消息循环、窗口创建、surface 配置与输入映射的部分证据 |
| C：headless / renderer | runtime、renderer 与 provider seam 的确定性测试 | 逻辑与几何契约，不证明原生可见行为 |

CI 通过、隐藏 smoke 或 renderer 测试都不能单独升级为 A 级证据。

## Phase 矩阵

| Phase | 必跑可见验收 | 现有证据 | 等级 | 状态 |
|---|---|---|---|---|
| 0 | 空闲重绘 | [stage-three 空闲重绘验收](stage-three-空闲重绘-验收.md) | A | 已记录（#288） |
| 0 | 长运行 | [stage-three 长运行验收](stage-three-长运行-验收.md) | A | 已记录（#289，30 分钟） |
| 0 | 设备丢失 | [stage-three 设备丢失实机尝试](stage-three-设备丢失-验收.md) | A（负结果） | 已尝试；观察到驱动事件但无 TDR/设备丢失，不声称恢复 |
| 0 | 基本输入 | [stage-two 验收](stage-two-验收.md)、[stage-three 文本编辑历史验收](stage-three-文本编辑历史-验收.md) | A | 已记录；边界见各页面 |
| 2 | 文本输入 / IME / lineage | [stage-three 外部替换与组合验收](stage-three-外部替换与组合-验收.md)、[stage-three 文本编辑历史验收](stage-three-文本编辑历史-验收.md) | A | 已记录 |
| 2 | 焦点显露 | [stage-three 焦点显露验收](stage-three-焦点显露-验收.md) | A | 已记录（#180） |
| 2 | 窗口化 For 编辑 | [stage-three 窗口化 For 验收](stage-three-窗口化-for-验收.md) | A | 已记录（#184，10k 行） |
| 3 | IME、焦点、hover/press | [stage-three 外部替换与组合验收](stage-three-外部替换与组合-验收.md)、[stage-three 焦点显露验收](stage-three-焦点显露-验收.md)、[stage-three 辅助技术合同与朗读验收](stage-three-辅助技术总验收.md)、[stage-three 宿主滚动验收](stage-three-宿主滚动-验收.md)（100% 步骤 9–11、150% 步骤 7–9） | A | 已记录；hover/press 的 NodeId/`checked` 读数与 100%/150% 截图见宿主滚动验收 |
| 3 | DPI 100% / 150% | [stage-three 多窗口验收](stage-three-多窗口-验收.md)、[stage-three 滚动总验收](stage-three-滚动总验收.md) | A | 已记录 |
| 3 | UIA（Inspect/Narrator） | [stage-three 辅助技术验收](stage-three-辅助技术-验收.md)、[辅助技术动作](stage-three-辅助技术动作-验收.md)、[辅助技术事件](stage-three-辅助技术事件-验收.md)、[辅助技术文本](stage-three-辅助技术文本-验收.md)、[辅助技术滚动](stage-three-辅助技术滚动-验收.md)、[辅助技术总验收](stage-three-辅助技术总验收.md) | A | 已记录 |
| 3 | 菜单 / 快捷键 / 上下文菜单 | [菜单栏](stage-three-菜单栏-验收.md)、[快捷键](stage-three-快捷键-验收.md)、[上下文菜单](stage-three-上下文菜单-验收.md) | A | 已记录；节点级上下文菜单仍属后续工单 |
| 3 | 托盘 | [stage-three 托盘验收](stage-three-托盘-验收.md) | A | 已记录 |
| 3 | 拖放 | [stage-three 文件拖放验收](stage-three-文件拖放-验收.md) | A | 已记录 |
| 3 | 多窗口 | [stage-three 多窗口验收](stage-three-多窗口-验收.md) | A | 已记录 |
| 3 | 系统偏好 / 高对比度 / 文本缩放 | [系统偏好](stage-three-系统偏好-验收.md)、[高对比度](stage-three-高对比度-验收.md)、[文本缩放](stage-three-文本缩放-验收.md) | A | 已记录 |
| 3 | checkbox / submit | [stage-two 验收](stage-two-验收.md)、`tests/submit.rs` | A / C | 可见 Todo 运行已覆盖 checkbox；本轮 submit 计数由 headless 测试固定 |
| 3 | Delivery | [stage-three 会话验收](stage-three-会话-验收.md)、[stage-three 长运行验收](stage-three-长运行-验收.md) | A | 已记录（任务交付与长跑） |
| 4 | 100% / 150% 滚动 | [stage-three 宿主滚动验收](stage-three-宿主滚动-验收.md)、[滚动总验收](stage-three-滚动总验收.md) | A | 已记录 |
| 4 | 窗口化 For | [stage-three 窗口化 For 验收](stage-three-窗口化-for-验收.md) | A | 已记录 |
| 4 | UIA 事件 | [stage-three 辅助技术事件验收](stage-three-辅助技术事件-验收.md) | A | 已记录 |
| 4 | 主题 / 高对比度 | [stage-three 高对比度验收](stage-three-高对比度-验收.md)、[系统偏好验收](stage-three-系统偏好-验收.md) | A | 已记录；应用外观切换见 Todo 可见运行 |
| 4 | damage | 无独立可见验收页；renderer 当前按完整场景绘制 | C | 待人工与后续实现对齐，不声称局部 damage 绘制 |
| 4 | 多窗口隔离 | [stage-three 多窗口验收](stage-three-多窗口-验收.md) | A | 已记录 |
| 5 | 外部应用试用 | [stage-two 外部开发者试用协议](stage-two-外部开发者试用-协议.md) | 协议 | 已准备；外部开发者执行部分待人工 |
| 5 | 发布包 | [stage-three 发布打包验收](stage-three-发布打包-验收.md)、[空目录启动验收](stage-three-空目录启动-验收.md) | A | 已记录 |
| 5 | 启动 / 更新 / 长时间运行 | [空目录启动验收](stage-three-空目录启动-验收.md)、[长运行验收](stage-three-长运行-验收.md) | A（启动、长跑） | 更新路径未实现，不能作为已交付能力 |
| 5 | 本轮性能/内存门禁 | [性能门禁](性能门禁.md)、`bench/baselines/*.json` | C | 计数与时间来自可执行测量；固定 `perf-windows` runner 尚未登记与重录 |

## 未覆盖与后续工作

- 固定 `perf-windows` runner 的登记、基线与 nightly/soak 运行；当前只提交 `dev` runner 的趋势基线，时间门禁在 `dev*` 上默认报告。
- damage 局部绘制与增量 Taffy 布局：实现仍是完整快照根重建与完整场景重排；门禁用 delta 变化节点、场景元素与内存计数，不把当前实现表述为增量呈现。
- 可见 matrix 中标注“待人工”的条目（外部开发者试用、更新路径）必须由真实运行补记录，不能由 headless 测试替代。
- 本页不新增设备丢失恢复、多显示器混合 DPI、像素级 hover/press 或 UIA 进程外客户端的结论；这些结论只来自对应页面记录的单屏运行。
