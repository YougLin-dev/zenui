# System preferences are host inputs for high contrast and text scale

ZenUI paints its own content, so Windows cannot adapt it the way it adapts system-drawn chrome. A window keeps the application light/dark palette and fixed 14-point text when the user turns on a high-contrast theme or raises the accessibility text size. #293 asks for a generational preference input; #294–#297 implement it.

## Decision

`zenui-windows` exposes `SystemPreferences`, a `Copy + Eq` value with `high_contrast: bool` and `text_scale_percent: u16`. The percentage is valid in 100–225; any other value falls back to 100, and `Default` is high contrast off at 100. Preferences are host inputs: they are not snapshot attributes, application state, undo entries or model transactions.

The visible host samples at startup and on every `WM_SETTINGCHANGE`:

- high contrast comes from `SystemParametersInfoW(SPI_GETHIGHCONTRAST)` and the `HCF_HIGHCONTRASTON` flag;
- text scale comes from the `HKCU\Software\Microsoft\Accessibility\TextScaleFactor` `REG_DWORD`; a missing key, a missing value, or a failed read means 100%.

The registry value is the per-user value Windows writes for the accessibility text-size slider. It avoids activating WinRT `UISettings` during host startup and keeps the sampled value an integer. A preference change marks every live window for repaint; `Host::set_system_preferences` provides the same path for embedders. Hidden hosts never read the machine's settings: `HostConfig::with_system_preferences` injects the initial value and `SystemPreferences::default()` is used otherwise. Tests inject a value; they never change the machine.

### High contrast palette

`HighContrastColors` carries the sampled `GetSysColor` values for window, window text, button face, button highlight and shadow, gray text, highlight, highlight text and window frame. `RenderPalette::high_contrast` maps semantic roles and built-in visuals to them:

| role | system color |
| --- | --- |
| `surface`, `background` | window |
| `control` | button face |
| `control_hover`, `control_pressed` | button highlight, button shadow |
| `text` | window text |
| `text_muted`, `text_disabled` | gray text |
| `text_inverse` | highlight text |
| `accent`, `danger` | highlight |
| `border` | window frame |
| `caret` | window text |
| `selection` | highlight at 80 alpha |
| scrollbar track, thumb | button face, window frame |

When high contrast is on, the host ignores the application light/dark appearance for these roles; when it is off, the existing palettes are unchanged. Explicit `Color::Literal` values are never remapped; that boundary is documented rather than silently rewriting application choices. Selection keeps a translucent highlight so the text role remains readable; Windows high contrast has no separate danger role, so `danger` uses highlight.

### Text scale

The text-scale percentage is part of `SceneEnv`, which remains `Copy + Eq`. Style resolution multiplies the final font size after declarations are applied. Line height, `em` widths and heights, text measurement and wrapping, caret and selection geometry, hit-testing, scroll content extent and row bands therefore derive from the scaled text. Fixed `px` lengths remain logical pixels; applications that need scalable fixed geometry express it in `em`. The display DPI scale stays independent and is never multiplied twice.

Text-layout and measurement cache keys already contain the font-size bit pattern, so a scaled layout cannot be served for another scale. A live change rebuilds open scenes, preserves focus, caret, selection and local edit values, re-clamps the stored scroll offset, and does not change the presentation revision or dispatch `on:input`. UIA bounds come from the rebuilt scene.

### Environment generation

A preference sample, a DPI scale change, a change in the applied scroll offset or a change in keyboard modality advances the window's `env_generation`. The generation is part of `SceneEnv` and enters `Scene::cache_key` together with the snapshot revision and the interaction-path generation, so the host can rebuild geometry and semantics from the same snapshot root. It is a host presentation counter: it never advances the committed tree revision, never appears in the snapshot or its equality, and never enters transactions or the undo history. Wheel, paging, scrollbar, UIA scroll and reveal advance it when they change the offset, before the scene that uses the new offset is built. A preference sample that changes no value does not advance it; a content-extent change that re-clamps an already stored offset advances it when the offset is corrected.

## Considered options

- **WinRT `UISettings.TextScaleFactor`**: rejected for this slice. It adds WinRT activation and a float-to-percentage conversion at host startup; the registry value is the integer Windows writes for the same setting, and the host already resamples on `WM_SETTINGCHANGE`.
- **A third `Appearance` variant for high contrast**: rejected. High contrast is a system override with its own color source, not another application palette; keeping it in `Appearance` would make application appearance and system override indistinguishable.
- **Remapping explicit literal colors**: rejected. ZenUI cannot know whether a literal is decorative or semantic, and silently replacing application colors would make the boundary unpredictable. High contrast coverage for literals is the application's choice.
- **Scaling every `px` length**: rejected. `px` means logical pixels in this API; fixed geometry must stay fixed. `em` is the opt-in for text-relative geometry.
- **Rebuilding the application model on a preference change**: rejected. Preferences are host-owned and must not enter transactions, undo history or `on:input`.
- **Per-component or per-window preference overrides**: out of scope; the Windows setting is per user, and one host value keeps scene equality and cache identity simple.

## Consequences

Applications see high contrast as a palette override and text scale as a layout input, without code changes. Applications that use literal colors or fixed `px` row heights keep responsibility for their own accessibility in those places; the SDK pages state both boundaries. The scene cache, row-band geometry and UIA tree already key off the scaled layout, so no separate invalidation path is added.

The behavior is recorded in [stage-three 高对比度验收](../实现/stage-three-高对比度-验收.md) and [stage-three 文本缩放验收](../实现/stage-three-文本缩放-验收.md); the combined visible run and system-chrome check are in [stage-three 系统偏好验收](../实现/stage-three-系统偏好-验收.md). Reduced motion remains out of scope until an animation or ticker exists. Multi-monitor and mixed-DPI behavior remains #63. System menus, the tray and window chrome are drawn by Windows and follow the system theme without ZenUI involvement.
