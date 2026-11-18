# 技术栈决策与边界

日期：2026-11-18。这里区分候选正式依赖、可交换适配层、明确拒绝。不是对尚未实现代码的版本声明。

## 版本与依赖域

| 层 | 固定研究版本 | 是否能替换 | 理由 / 禁例 |
|---|---|---|---|
| Reactive / Tree | 自有 UI-thread runtime | 不能由 dependencies 自动获得 | 一套状态到界面的默认权威；Owner 可卸载，Field patch，而非全量 View rebuild |
| Markup | `rstml 0.13.1` + `syn 3` + `quote` | 仅前端 IR 可替换 | AST 有 tags、attrs、Rust expr 和 spans；小写 tag 是 native intrinsic，不是 HTML |
| Style | 自有 StyleIR / Class resolver | Tailwind Rust port 仅可选 oracle | UI 状态与字段的 type/conditions；不执行 CSS/JS；不宣称任意 arbitrary CSS 兼容 |
| Layout | `taffy 0.15.1` | 通过本框架 Adapter，NodeId 私用 | flex/grid/math layout；不是浏览器，也不负责控件焦点、scroll、inline text |
| Text | `parley 0.13.0` + `parley_engine 0.13.0`，`complex-scripts` | 不该轻易换 engine；需规模化 fixtures | shaping/bidi/fallback；Parley/glifo normalized coords 显式 bit conversion |
| Text editor | 自有 widget + winit/AccessKit | glyph 无意义地伪装成 editable input，拒绝 | caret/selection/IME 文字坐标命名 newtype；context the conversion tests 分开 |
| GPU types | `wgpu 30.0.2` | 同一代 backend 的版本必须统一 | 一个 app Device/Queue；需要 capability checks 和 resource lifetime policy |
| 2D 渲染 | `vello_gpu 0.4.0`；`vello_cpu 0.4.0`；`glifo 0.5.0` | Scene Adapter 可以替换实现 | 生产 sparse-strips 首选；CPU reference；mask/complex filter 等 capability gap |
| Classic interop | `vello 0.12.0` 仅在独立 probe | 旧研究路线不作为产品首选 | Classic observed alpha/composition 与 sparse strips 不是同一 contract |
| Window / A11y | `winit 0.30.13` / `accesskit 0.25.2` / `accesskit_winit 0.34.2` | WindowShell 实现可以替换 | stable 配合更明确；0.31 beta 不为漂亮新 API 破坏生态一致 |
| 可选 3D | `gltf 1.4.1` + `glam` + 自有 wgpu 基础 mesh | 3D backend 可选，UI core 无重 ECS | 导入支持 ≠ PBR/动画完整实现；不将 Bevy/游戏 runtime 强插入核心 |

## 推荐的 workspace

```text
zenui/             公共 re-export、预编译 view!、typed builder 风格入口
zenui-macros/      proc macro，尽量只做 rstml AST → typed calls
zenui-reactive/    signal graph、Owner、Memo、RenderBinding、thread bridge
zenui-tree/        原生节点、ViewId、commands、focus/scroll/lifecycle
zenui-style/       literal class、theme、StyleIR、fixtures/差分工具
zenui-text/        Parley leaf、font registry、index conversions、editor state
zenui-render/      retained paint records、Scene adapters、capability gate
zenui-platform/    winit、Surface、AccessKit、IME、native services
zenui-3d/ optional asset loader + own render pipeline + offscreen target
zenui-tailwind/    CSS-first theme/native-utility registry + build integration
```

目标是宏瘦身、runtime 编译边界明确、library backend 不外泄实际 surface/handles。**不必现在就把全部 placeholder crates 写出来**；第一是共同稳定和可测的 API/IR，不是在目录数上制造“完成”。

## 吸取项目优点的结论

- deltaui：run-once 与节点 retained/binding 可参考；不提供我们需要的 HTML-like 语法、完整 Tailwind 或现成三平台保证。**不能把其 fork/version 依赖无声继承成我们的生产设备保证**。
- FLUI：布局算法/精确 invalidation/damage 与回滚意识值得借鉴；不将三树 reconciler 再塞进单一 signal 更新权威。
- llimphi：2D + 外部 3D 的 wgpu 互操作值得验证；其 Elm model 不成为我们的信号 API，很多 glTF rendering 能力仍需自己实现。
- GPUI：复杂 widgets 自定义 layout/paint，独立 workflow pass；不把其 entity/notification 整个搬成第二套权威。
- Slint：DSL 编译、IDE/clippy 错误定位、文档工具是标杆；不复用其不同许可或自己的脚本 engine，而采用 Rust AST。
- Masonry/Parley/Dioxus Blitz：text 和 retained rendering 的实践都可读；API 成熟语法/API 应自己定义，不堆砌框架概念。

## 稳定性和不支持策略

1. 选 stack 时按实际执行验证和源码成熟度，而非发布日期或名字“更高级”。
2. Pub trait 不出现 Taffy `NodeId`、Vello 内部 target 或 `wgpu::SurfaceTexture` 这种调度/资源持有责任尚未封装的类型。
3. 公共 RenderBoundary 需要明确 format/alpha/frame epoch/texture usage/ownership，不包含散乱非版稳定 private handles。
4. intrinsic UI semantics 不由 browser CSS implicit。定义 `<view>` 的默认 flow，`<row>/<column>` 小类是语法糖。
5. 同一类 font/capability 若非当前实现能力内，构建/运行 API 明确返回 unavailable；不假装 unsupported=empty image。
