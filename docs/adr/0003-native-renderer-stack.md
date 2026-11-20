# First native renderer reuses the native_poc stack

The first Windows renderer reuses the proven offscreen stack from `examples/native_poc`: Taffy for layout, Parley/glifo for text, and vello_gpu/wgpu for drawing. Mainline Vello and a custom Direct3D path remain alternatives; adopting them now would delay the window loop and duplicate a working text path. The backend stays replaceable behind the scene seam.

## Consequences

`vello_gpu` has only been proven offscreen, and only with `Rgba8Unorm`. The renderer must add an offscreen test for the surface formats `Bgra8Unorm` and `Bgra8UnormSrgb` before host integration; if sRGB output is wrong, the host requests a non-sRGB surface or adds a blit. If surface integration or maintenance becomes a problem, replace it behind the scene seam rather than letting it leak into the runtime.
