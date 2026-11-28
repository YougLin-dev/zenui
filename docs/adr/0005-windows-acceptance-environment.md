# Windows acceptance combines a CI smoke test with real-machine checks

The hosted Windows runner has no hardware GPU, so acceptance must not depend on a visible swapchain. CI creates a hidden HWND, initializes wgpu with WARP fallback, renders offscreen and reads back pixels; that proves window creation, the message loop, surface configuration and input mapping. A visible window, real GPU, Chinese IME candidate UI, focus behavior and per-monitor DPI are verified manually on Windows 11.

## Consequences

If hidden-window or swapchain behavior differs on the runner, CI falls back to renderer-only tests and the host paths remain covered by the manual checklist. CI passing alone never counts as native presentation acceptance.

## 状态更新（2026-11）

验收证据按类型分开记录：CI 的隐藏 HWND/WARP 与 renderer-only 测试只算部分证据；可见 Windows 运行按 Phase 矩阵映射到 `docs/实现/stage-three-*-验收.md`，矩阵与人工清单分别是 [stage-three 可见验收矩阵](../实现/stage-three-可见验收矩阵.md) 与 [Windows 手动检查清单](../实现/windows-手动检查清单.md)。性能与内存门禁使用固定 `perf-windows` runner、`bench/baselines/*.json` 与 `--require-time`；开发机基线只记录趋势，不作为发布承诺。详见 [性能门禁](../实现/性能门禁.md)。
