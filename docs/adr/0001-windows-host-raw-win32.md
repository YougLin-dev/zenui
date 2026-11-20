# Windows host uses raw Win32 in a separate crate

The Windows host must handle IME candidate positioning, per-monitor DPI, UIA, menus and dialogs. Winit would abstract windowing but still require raw Win32 for those features, so we use the `windows` crate directly in a new `zenui-windows` crate. The root `zenui` crate stays `#![forbid(unsafe_code)]` and headless; platform `unsafe` is confined to the host crate.

## Considered Options

- **winit for window and event loop**: adds an abstraction the platform features above would bypass.
- **Hybrid winit window + raw Win32 for IME/UIA**: two event sources and ownership models in one process.
