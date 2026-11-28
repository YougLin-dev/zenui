# Presentation is a one-way snapshot → scene → surface pipeline

The headless target runtime exposes an immutable presentation snapshot of committed UI; `zenui-render` consumes it and owns Taffy/Parley layout and scene construction; `zenui-windows` owns the HWND, wgpu surface and presentation. The runtime never gains renderer dependencies, the renderer never reads runtime internals, and no second retained UI tree exists.

## 状态更新（2026-11）

运行时 v2 按本条发布不可变快照：每个树槽维护自己的 revision，`PresentationSnapshot` 包裹共享的 `Arc<SnapshotRoot>`，元素与文本节点携带跨快照稳定的 `PresentationKey`。`PresentationUpdate` 携带 `PresentationDelta` 供 renderer 失效缓存；UIA 不消费 delta，而是从快照与同一份 `Scene` 派生整树并在相邻发布之间比较。绑定槽、结构提交与快照构建在 runtime 内完成；快照仍是派生描述，不是第二棵可修改的保留树，也不含几何。实现与门禁见 [事件与响应式更新-方案](../实现/事件与响应式更新-方案.md) 的 §19 与 §20，以及 [ADR-0026](0026-published-presentation-updates-and-deltas.md)。

## Considered Options

- **Runtime computes geometry**: would put Taffy in the headless crate and blur the single-authority boundary.
- **Renderer reads the runtime element tree directly**: couples every backend to private runtime structure and makes offscreen tests depend on runtime internals.
