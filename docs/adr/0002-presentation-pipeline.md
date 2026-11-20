# Presentation is a one-way snapshot → scene → surface pipeline

The headless target runtime exposes an immutable presentation snapshot of committed UI; `zenui-render` consumes it and owns Taffy/Parley layout and scene construction; `zenui-windows` owns the HWND, wgpu surface and presentation. The runtime never gains renderer dependencies, the renderer never reads runtime internals, and no second retained UI tree exists.

## Considered Options

- **Runtime computes geometry**: would put Taffy in the headless crate and blur the single-authority boundary.
- **Renderer reads the runtime element tree directly**: couples every backend to private runtime structure and makes offscreen tests depend on runtime internals.
