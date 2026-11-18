# Zenui 研究结论（简明入口）

> 总研究日期：2026-11-18。本文回答“技术路线怎么走、哪些已得到最小验证”。不是产品发布或三平台兼容性承诺。
>
> **历史研究入口**：当前唯一统一规格为[评审 FINAL.md](../review/FINAL.md)，真实复跑与限制为[VALIDATION.md](../review/VALIDATION.md)。下文早期 `class:if-*`、隐式 flush、任何 condition 胜过普通 inline 的候选均已撤销；组件应以 `MountResult<MountPlan>` 和原子事务为当前目标，不应按本历史上简化流程直接实现。

## 结论：沿原生保留树 + 单一响应式图的路线前进

建议以用户构想的 **SolidJS 风格细粒度更新 + HTML 外形的过程宏 + Taffy + Parley + Vello/wgpu** 做主干，但不把“这些 crate 组合起来”视为完成。真正需要的是自有、清晰的 Tree/Owner/Binding/StyleIR 和适配层。

生产候选版本固定为：`taffy 0.15.1`、`parley 0.13.0`、`vello_gpu/vello_cpu 0.4.0`、`glifo 0.5.0`、`wgpu 30.0.2`，以及稳定组合的 `winit 0.30.13 / accesskit 0.25.2 / accesskit_winit 0.34.2`。UI 主线程默认本地；worker 计算通过带 generation 的桥提交。

Classic Vello 0.12 的已验证 target interop 是研究证据，不等于生产端选用它。生产端默认采用 sparse-strips 的 GPU backend，CPU 做 reference/必要 fallback。

## 一个数据结构讲清整个框架

```text
组件函数（mount/setup 一次）
  → 原生保留节点、Rust closure 对应的字段绑定、Owner-scope
  → Signal 更新后只 mark 对应 observer
  → 同帧 flush memo/binding，提交文本/样式/布局或结构 patch
  → Taffy style projection + Parley paragraph/glyph layout
  → Camera2D/geometry → 自有 retained paint records
  → Vello GPU Scene / CPU reference → wgpu surface present
  → AccessKit 语义输出、external services、延迟到 post-frame 的 effects
```

不把 VDOM/Elm View 重建或 Flutter 三套 reconciler 混为第二权威。`ViewId` 负责 UI identity、`OwnerId` 负责生命周期、`SignalId` 负责依赖、`Taffy NodeId` 是私有布局 ID，AccessKit ID 从稳定 ViewId 生成。不能一个整数 ID 覆盖不同资源域。

宏是薄编译桥，可清楚回退到 typed builder：

```rust
view! {
    <view class="flex flex-col gap-2 bg-zinc-900">
        <text>{move || title.get()}</text>
        <button
            class="rounded-md bg-blue-600 hover:bg-blue-800"
            on:click={move || open_project()}
        >
            "打开"
        </button>
    </view>
}
```

`{expr}` 作初始值；数据位置 `{move || expr}` 作绑定；`on:*` closure 总是事件，不被追踪。`Show` 创建/销毁 child owner，`For` 按 key 保留与 reorder，`Index` 按位置更新项目 signal；不能全换成 `display:none`。完整 components/props/slots/动态列表目前仍是设计 API。

## Tailwind 支持策略：typed native utility，不是浏览器 VM

推荐允许字面量 class 在构建期编译成 `StyleRule { property, declaration, condition, precedence, source }`，再分发到 layout/layout text/绘制字段。

常见类族目标涵盖 size/flex/grid/align/spacing/colors/rounding/typography/paint/hover/focus-visible/active/disabled/checked/dark/window/container conditions。动态 classes 依赖 manifest 或 ClassId registry，不支持任意外部字符串在 runtime 试错。

**不会完整复刻 CSS cascade 或 DOM。** 任意 selector、pseudo、JS plugin、需要浏览器 DOM 的 arbitrary rule 默认编译诊断或拒绝。`active:` 指 native Pressed，非选中；`class:if-*` 是候选的原生条件注册语法，不等于官方 Tailwind identical feature。

真正的 support % 必须固定官方版本，生成 fixture manifest 并做 token→declaration、variant precedence、box geometry、render、unknown diagnostics 的差分。目前 `tailwind-coverage.md` 是候选支持矩阵，不是已经跑完官方用例的统计结果。

## 通过验证增强了哪些关键信心

- **7 个 reactive/style tests**：source 只影响自己的 sink；inactive dependency 解绑；Owner dispose；batch 去重；typed utility fields 与非法值/byte offset。
- **2 个宏 integration tests**：真实 rstml→StyleRule/typed record 转写、嵌套节点、静态文本/显式闭包绑定、事件仅 dispatch 时执行。另有 **1 个 trybuild test，3 个 compile-fail case**，锁定 unknown class、DOM tag 与脚本事件 diagnostics。
- **4 个 native tests**：Parley CJK shaping 与实际 CPU glyph pixels、ICU segmentation dictionary boundaries、Taffy 新测量 API 与真实文字更新、预乘 alpha 常识对照。
- **两个真正的 Windows/NVIDIA/Vulkan 渲染执行**：
  - Classic：vello 0.12 + foreign wgpu pass preserve neighbors + overlay 以及 actual pixel readback。
  - 当前生产候选：vello_gpu 0.4 的真实 CJK glyph 与 2D/外部 rgba TextureView 合成；透明目标 `[128,0,0,128]` 等同 vello_cpu reference。

这些说明最危险的“句法是不是只能通过字符串？”和“渲染器能不能真正并上 wgpu 与文本/贴图？”已经获得最小可行验证。仍未证明这东西已经是桌面应用框架。

## 必须继续的门

1. **Owner/Store/For/Index/Resource** 的严格生命周期与调用顺序，压力测试和公布数据。
2. **真正打开桌面窗口**：winit 输入、IME、AccessKit、焦点、scroll、DPI/resize、device loss；Windows/macOS/Linux 都实测。
3. **样式 golden tests**：Swift UI 无关，将 Tailwind 固定 version 的输出语义与原生字段逐项对照、严格 unknown 与整个人体工学。
4. **文字产品能力**：emoji、混合 bidi、intrinsic sizes/caches、selection/caret a11y、确定性字体 artifacts。
5. **真正的 glTF viewport**：asset worker、mesh/material、own depth/color target、alpha format、generation 与多 viewport lifecycle。当前外部纹理测试是**绿色占位**，不是加载模型或 PBR 完成。

## 从这里阅读详细材料

1. [stack-decisions.md](stack-decisions.md)：版本、crate 划分与参考框架借鉴边界。
2. [native-macro.md](native-macro.md)：目标语法、动态值和生命周期语义。
3. [style-system.md](style-system.md)：utility 到 StyleIR，构建与动态主题。
4. [runtime-and-rendering.md](runtime-and-rendering.md)：帧协议、布局/文字/渲染/三维合成。
5. [tailwind-coverage.md](tailwind-coverage.md)：明确哪些支持、受限与拒绝。
6. [validation-and-roadmap.md](validation-and-roadmap.md)：本机执行命令、实际修正、通过项与阶段验收门。
