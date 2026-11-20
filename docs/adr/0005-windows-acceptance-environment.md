# Windows acceptance combines a CI smoke test with real-machine checks

The hosted Windows runner has no hardware GPU, so acceptance must not depend on a visible swapchain. CI creates a hidden HWND, initializes wgpu with WARP fallback, renders offscreen and reads back pixels; that proves window creation, the message loop, surface configuration and input mapping. A visible window, real GPU, Chinese IME candidate UI, focus behavior and per-monitor DPI are verified manually on Windows 11.

## Consequences

If hidden-window or swapchain behavior differs on the runner, CI falls back to renderer-only tests and the host paths remain covered by the manual checklist. CI passing alone never counts as native presentation acceptance.
