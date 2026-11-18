# 验证记录与阶段 Gate

日期：2026-11-18。执行环境：Windows 11，Rust `1.99.1 x86_64-pc-windows-msvc`，GPU 运行时报告 NVIDIA GeForce RTX 4060 Laptop GPU / Vulkan。**以下仅是已执行的小型探针，不是已发布框架、性能基准或三平台验收。**

## 1. 实际执行通过的验证

| 路径 | 实际结论 | 未由此证明的能力 |
|---|---|---|
| `examples/desktop_poc` lib | 4 个 reactive tests：局部 sink、动态边删除、batch 去重、Owner dispose；3 个 typed-style tests：字段映射、局部条件覆盖、非法 syntax/offset诊断 | 完整 Memo/effect/resource、For 行生命周期、Tailwind cascade/theme 与官方 fixture 全枚举 |
| `examples/macro_poc --workspace` | 2 个 integration tests：rstml 0.13.1 的 nested RSX，literal class 编译到 StyleRule，静态文本、显式 move 文字绑定、事件 parity；调用方无需导入内部 Node；closure 只在显式求值/dispatch 时执行。另有 1 个 trybuild test、3 个 compile-fail：unknown class+offset、native tag、禁止脚本事件 | 全套 intrinsic/PascalCase components/show/for、span 到 class 内字符的高亮、renamed dependency、生产 RenderBinding/所有权 runtime |
| `examples/native_poc --lib` | 4 个不同维度 tests：Parley CJK glyph + CPU pixel、ICU4X 真正 `中文/文本/测试` boundaries、Taffy 0.15 测尺寸与真文本更新后布局变化、透明 alpha CPU reference | mixed bidi intrinsic、Taffy global/baseline bugs、完整字体/fallback、系统 input/a11y |
| `cargo run -p native_poc manifest` | 在 headless `Rgba8Unorm` target 实际执行 `vello_gpu 0.4` + `vello_cpu 0.4` + `parley 0.13` + `glifo 0.5` + `wgpu 30.0.2`：2D rect、CJK positioned glyph、普通外部 `TextureView`、半透矩形；CJK glyph区域非空与像素对照通过 | 多 OS/GPU/matrix/full PBR、window/IME/accessibility/mask/filter、cache性能 |
| `cargo run -p classic graphics` | `vello 0.12` Classic 的 64×64 shader GPU interop：2D underlay、foreign wgpu `LoadOp::Load` 与 overlay、直接 RGBA 断言通过 | 新 sparse-strips 的 alpha 与绑定、glTF material、跨 platform preserve target promise |
| tailwind compiler 离线研究 | Rust `tailwind 0.4.0` default-features=false 本机能构造 candidate/主题 CSS output，验证了 `p-4`、颜色、hover、arbitrary、theme fixture | 官方 Tailwind 完全语义一致、quickjs MSVC、native 样式适配、Tailwind coverage 宣称 |

命令（在仓库根目录）：

```sh
cargo test --manifest-path examples/desktop_poc/Cargo.toml --lib
cargo test --manifest-path examples/macro_poc/Cargo.toml --workspace
cargo test --manifest-path examples/native_poc/Cargo.toml --lib
cargo run --manifest-path examples/native_poc/Cargo.toml
cargo run --manifest-path examples/desktop_poc/Cargo.toml
```

这几次实际 native run 的 core 输出：

```text
PASS: sparse-strips GPU, real Parley/Glifo CJK pixel runs, ordinary external texture binding.
Transparent semi-red GPU pixel: [128, 0, 0, 128]; CPU reference matches.
SCOPE: headless Windows RGBA graph; no glTF/window/IME/a11y/platform-font/platform-GPU/mask/filter/performance claim.
```

第一次 native run 中出现 `ICU4X data error: No segmentation model for Chinese/Japanese`，随后启用 `parley_engine/parley complex-scripts`，用真正的词组 boundaries 断言 ``[true,false,true,false,true,false]`` 验证已消除。后续 GPU run 不再出现这条 warning。**这说明只为英文设计 text 或只覆盖 CJK rendering 不够，词典数据在发布 artifact 中不可缺。**

## 2. 验证时发生的实际修正

- typed-style parser 拒绝 `p--2`、非法 arbitrary width、指数/符号/NaN/Inf；不能通过总体 parse::<f32>() 的宽松 grammar 营造成功。
- reactive `batch` 使用一次提交的 patch vector；不能再 flush 后旧输出又期待仍在队列。
- class IR 按属性查找，不按 rules[n] 假定 class 出现顺序；border 是 border_color 而非误写 background。
- rstml 0.13.1 使用 `syn 3`，把 syn 2 AST 混入产生 incompatible error；宏代码路径用绝对 runtime crate，调用方不需手动 import 内部名称。
- Parley `NormalizedCoord` 与 glifo i16 type 要 `to_bits()` 显式转换；默认 `atlas_cache(false)` 避免把其标注“实验”的 cache 作为基础能力。
- `vello_gpu::Resources` 出自同一 `Renderer::new`，不构造临时自主 `Resources` 给另一个 renderer 用；每个 render encoder 提交后才 readback。
- Taffy 新 callback 使用 `LayoutInput, NodeId, Option<&mut NodeContext>, &Style -> LayoutOutput` 和 `compute_leaf_layout`；content 改后 explicit dirty/refresh，且实际断言 measured size 跟着变。
- 旧 Classic output 预处理的 alpha 结论不可直接套用：sparse-strips 实际透明目标的 GPU half-opacity 红色 `[128,0,0,128]` 已与 software premultiplied counterpart 相等。

## 3. 现在还不能叫“顶级框架”的明确缺口

1. **只有 Windows/Vulkan 单机最小验证。** winit 真实 surface、macOS/Metal、Linux Vulkan/GL、multi-monitor DPI、device lost、x11/wayland 输入、屏幕阅读器全部没有验收。
2. **PoC reactive 不是 production arena。** 尚无完整 Owner ID 世代、Memo fairness、多线程 bridge、resource generations、Store/For 行 owner 和百万信号/长链压力。
3. **rstml 的宏前端只有目标形状。** 完整 syntax spec、props/slot/component typing、intrinsic class masks、inline diagnostics、动态 class manifest 同步、extends/Ide tooling 还没有。
4. **Tailwind coverage 表是设计标定，不是通过统计。** 没有固定官方 v4 commit/generated manifest/differential test 的百分比。CSS-first 只做了 Rust port 的可行性参考，并未把 CSS compiler 变原生 UI runtime。
5. **text complete product 边界很大。** 新字典已验，但 emoji shape/render、bidi intrinsic widths、IME selection/caret conversion、bidi 文本编辑/indent/font shapes/AccessKit 未验。
6. **全新 sparse-strips 只是正确性开始。** GPU mask/complex filter 会 panic 的官方风险仍在；分层 clip/mask/opacity、glifo 实用缓存、color profiles 和 atlas lifecycle 未验。
7. **3D 验证目前是带 `TextureView` 的绿色占位而非真实 mesh。** glTF buffers/materials/fallback、camera/PBR/depth generation、multiple viewport、实际 2D overlay model 仍是下一阶段。

## 4. 后续阶段 Gate

> 阶段按验收门槛组合，不以“代码可以 build”代替验收里程碑。

### Gate A：交互模型与宏 API

- typed builder 与宏是同样 runtime/IR，可单测；Rust IDE rename/clippy 可用。
- Field patch 而非 component rerun；同 Owner dispose 的 text/style/layout binding、node identity 和资源一起清除。
- `Show/<For key>/<Index>` 子树、行 caret/focus、重复 key错误、异步 stale generation 全面通过。
- `view!` 涵盖 typed component/children/attrs/events，static literal classes spans 诊断，未知 class/动态 manifest 有反馈。

### Gate B：layout、text、style 差分基线

- Parley text snippets（CJK/emoji/bidi/mixed fonts）与 fixed font artifacts 一起存 baseline。
- min/max-content、wrap、padding/border/baseline、font/DPI 改变、Taffy parent reparent 缓存 canary。
- 固定 Tailwind v4 source commit/owned fixture，生成 utility/variant/value/origin diagnostics 与 native semantic diff。**不接受手写 percent 通过。**
- GPU/CPU basic fill/text/images/clipping/reference layout 按明确 channels 比像素，决定可以为哪些 class/variant 发文。

### Gate C：垂直桌面窗口切片

- 应用含导航、scroll、TextField/IME、context menu、window DPI、显卡 resize、dark theme、focus-visible。
- 运行时 `Wait`，信号仅 dirty binding → measure/layout → native paint；单字段动态不应 rerun whole root。
- AccessKit tree/action/caret/focus semantics 与 render/hit 同 ViewId/original geometry。
- Windows + macOS + Linux 桌面实测，Wayland/X11/各显示服务器 IO capability 明确 `Result` 而非假等价。
- CPU fallback 与 GPU error/dpi/initialization 失败退化，不用 panic 作为用户层 feature detection。

### Gate D：真正的模型 viewport 和性能验收

- worker 解析 glTF 文件允许 subset，再 upload（generation+owner+resource usage）并渲染 triangle/mesh。
- 自有 depth/color和匹配 alpha，对应 `TextureBindings` source；2D model 上层、不透明/半透明 context viewport clip、多实例与异步 disposal。
- 逐步加 materials/定向光、textures、animations、orbit.focus/hit。无免费承诺 arbitrary 2D/3D depth intersections。
- 以帧时间/内存/布局文本更新/编码、GPU rasterization vs cached scene、静 idle 能耗测过来决定架构，不把“细粒度=O(1）/缓存=局部 GPU raster”写进 benchmark 外宣传。

## 5. 后台研究的诚实状态

landscape workflow 有 22 个 agent，21 完成、1 个 evidence-synthesis API error；architecture 曾有单个 agent 未以 StructuredOutput 收尾。其信息作为多视角研究来源使用，**不自动成为源码/机器验证**。本文所有“实际验证”项的判据均为 repo 内本机可重现的 executable assertions、官方文档/源码或已注明的构造输出。
