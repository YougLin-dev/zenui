# 运行时、布局、文本、渲染与 3D

## 1. 数据所有权而不是“临时闭包世界”

```text
Application
 ├── ReactiveRuntime（UI thread）
 │    ├── Owner tree: child scopes, context, cleanup handler
 │    ├── Signal/Memo arena, subscriber edges + current tracker
 │    └── Deferred RenderBinding commit queue
 ├── Native retain Tree
 │    ├── ViewId + generation, NodeKind, typed widget state
 │    ├── style/resolved field origins & local dynamic field bindings
 │    ├── Taffy NodeId ↔ ViewId mapping
 │    ├── event/focus/pointer-capture/scroll/IME owner IDs
 │    └── per-node cache keys and damage flags
 ├── WindowShell
 │    ├── native Window, Surface, DPI/native platform services
 │    └── FrameGraph & GPU resource lifetimes
 └── Optional Services: resources, clipboard/dialog/dragdrop, glTF viewport
```

状态树与订阅图关注不同目标：ViewId 用于界面稳定身份；OwnerId 用于作用域清理；SignalId/MemoId 用于依赖；Taffy NodeId 仅内部布局；AccessKit ID 由稳定 ViewId 映射；资源/texture 由对应 ViewId/generation 拥有。**不把它们混成一个 u64 当作所有功能的万能 key。**

`Rc<RefCell<T>>` 可以用在实现存储上，但不作为用户公开的信号或生命周期工具。`Owner` 只保存作用域与资源。强/弱边在技术上单独模拟，阻断孩子持有父、事件持 owner 和 data effect 回指造成的闭合环。事件处理不是 effect 的生命周期延长原因；dispose 时弱链接 unregister。

## 2. 对外语义的 Signal/Memo/RenderBinding

- `Signal<T>`：Handle，可 Clone/Copy（若选择），实际 `T` 存在图谱/arena；get 和 update 检查 value borrow 与当前 tracker。
- `Memo<T>`：惰性（默认需求驱动）或有需要的 eager compute，equality 由 Memo 负责。Effect 自身没有自动 PartialEq 抑制，避免把 Memo 的含义重复实现。
- `on_cleanup`：属于 owner disposal / subscription refreshed 的机制，将 unregister、timer abort、resource handle recycle 释放落在 owner，不靠 Rc field自然消失再来隐含处理。
- `RenderBinding`：**编译时暴露的同步产生值的特殊 computation**，返回 field-level patch 或整体不可变值，另一个 untracked commit 在 layout 前应用。普通用户 callback 不会自行设置 signal 或触发 mount update。
- `Effect`：框架外副作用/异步收尾，允许 defer；不承担同帧文本/样式写回的必须顺序。这一点与 `reactive_graph 0.3.0-rc1` 异步 Effect 文档不同，API 不应试图隐藏绝对执行差异。
- `batch`：一组 signal commits 共享一次图稳定/memo 计算/UI patching。对多次写同一反应源需要 last value/once queue；对跨图分支拓扑更新避免陈旧执行。
- `untrack`：关闭 reads 依赖跟踪，**不是让 effect 在 layout/paint 允许写树**。在 mount/layout/pending-dispose 临界阶段显式检查重入，防 `RefCell` 和几何旧引用噪声。
- `SyncSignal` 是显式线程边界。普通 UI state 默认 UI thread，worker 计算通过 channel/UI wake 将结果提交，并做 generation 认证。async task 绑定 owner，按 generation 取消过期结果，不强制依赖 Tokio。

### 评估 reactive_graph 复用

已发布的 `reactive_graph 0.3.0-rc1` Owner/ArenaItem/WeakOwner/Storage 很有借鉴/组合价值，但其 effect scheduled via any_spawner 到 runtime tick，源码明确是 async scheduling；**不可直接把它普通 Effect 当作 frame sync 值绑定。** 选择：

1. 先做自有安全 Rust 最小“依赖追踪+同步 render compute”核（可 small、tightly scoped）；
2. 详细评估 `reactive_graph::owner` 生命周期借用与贴图 callbacks；
3. 若引入该库，Owner/signal 部分复用，render binding 提交层始终独立；不为了兼容其默认 effect runtime 让 UI 晚一 tick。

### Signal cleanup 必须测

- inactive branch 的 signals 自动 unsubscribe。
- For reorder 不移除/重建正确的 row owner,caret focus 不跳。
- mount/unmount 10^4/10^5 次后 runtime counts 回落（多少阈值不事先欺骗）。
- 函数在 signal 写入后的 reader era 中先 read older scalar 再 load 新资源，旧返回结果 generation mismatch 丢弃。
- write-during-measure/layout、shadowed scope、Overrun depth 是显式故障，不报 RefCell borrow 文案。

## 3. 每帧协议

```text
native events & queued commands
   ↓ Command phase: hit map/focus/IME/accessibility action + generation validation
reactive mark → synchronous/topological Memo/render-compute flush (batch boundary)
   ↓ commit TreeMut / StylePatch / TextPatch / StructurePatch
   ↓ cleanup focus/capture/IME/pointer references to deleted ViewIds
TaffyT view: compute_layout_with_measure (cached dirty ancestors, text known modes)
   ↓ geometry/scroll/sticky/DPI transforms: same facts for paint,hit,a11y,caret
assemble/replay retained paint commands & external viewport registrations
   ↓ Render GPU: Underlay → 3D passes → overlay/composite → present
   ↓ commit only successful paint descriptors to published frame state
Async Effects/service callbacks; queue the next deadline / ControlFlow::Wait
```

Paint 发生在 rebuild / layout 的最后阶段，但 Tailwind priority、GPU composite、a11y 和焦点变更在对象 identity 层共同处理，不互相踢皮球。`ControlFlow::Wait` 空闲；`request_redraw` coalesce 连续事件，动画只请求下一帧 deadline；3D 静止不 activate 每帧。

## 4. Taffy/Parley 桥

[Taffy 0.15.1 docs](https://docs.rs/taffy/0.15.1/taffy/) 是当前高层布局起点。

- `Style IR → Taffy Style` 只 projection，`context` 放 layout/paragraph descriptor 与缓存 keys。
- `compute_layout_with_measure` 的确切 callback 是 `(LayoutInput, NodeId, Option<&mut NodeContext>, &Style) -> LayoutOutput`，普通 custom leaves 走 `compute_leaf_layout` 传播 padding/border/min-max/replaced，不直接手算 Size。
- 与 Parley 用 `calculate_content_widths()` 对应 min/max/intrinsic 与 width change；bidi 的公开文档明确指出该 intrinsic 函数可能不正确，不能把这一段称为完整 bidi 宽度支持。Text leaf 按 ready width 重 break，baseline 取 `Layout::first_baseline()`，并通过已识别 Taffy cache edge 做 fixtures。
- context 原地改文字/字体 alias 不会自动 dirty layout；因此每个 style/text/font epoch/scale change 都通过 `set_node_context` 或明确 `mark_dirty`。
- 公开 tree identity 是 ViewId，不在业务层传 Taffy NodeId。`remove_subtree` 统一逆向断开 Key 映射和全部资源。reparent 在先 detach 再 attach 的 transaction 后更新；研究 issue #1100（相似的 add_child 重挂风险）和 size-only cache baseline issue #1306 记录 canary，不盲目宣称 full CSS compatibility。
- `overflow` 数学效果不是 scroll widget。ScrollState 实现 wheel、drag、keyboard、Scrollbar、focus reveal、nested scroll + RTL anchor，viewport key 缓存 paint，不把 `hidden` 当 accessible disappearance。
- `Fixed/Absolute/HoistedChildren` 参与 geometry trauma，**所有 paint、hit、a11y 都从相同 accumulated coordinates 生成**；Text run offset ≠ CSS actual ink bounds。
- scroll/virtualize 3D video nodes are lazy viewport states；反复挂靠窗口的 widgets 拿 lifetime scope hook，而非 runtime borrow。
- Layout unit = logical pixel，Per-monitor DPI 单独作为 text+pixel 资源 epoch。不强行设全局只有一个 hidpi scale。

## 5. Renderer 不只一个 “Vello”

必须区分当前三条：

- [`vello`](https://docs.rs/vello/0.12.0/vello/) Classic 0.12：研究 compute-centric，wgpu30-compatible，连通 PoC 用。
- [`vello_gpu`](https://docs.rs/vello_gpu/0.4.0/vello_gpu/) 0.4 和 Common：sparse strips，CPU preprocessing+GPU raster；Rgba8Unorm targets，使用 glifo，和 wgpu30 shared types。
- [`vello_cpu`](https://docs.rs/vello_cpu/0.4.0/vello_cpu/) 0.4：最成熟 software path，deterministic reference/fallback，非 wgpu surface capability 假替代。

不要默认 Vello Hybrid0.2 是最新主路径（那是 wgpu29 的旧线）。命令类型/资源不属于 UI 核心架构：Zenui stable retained subtree command page（never CSS renderer）与 SceneAdapter 负责缓存到任何 backend。GPU 提供 capabilities（rgba/external textures/filters/clips）和能力测试，unsupported effect 不用 GPU validation/panic 解决。比如 sparse strips 有 mask/composite few limitations 与 glifo atlas 标注实验，第一阶段先保守路径 correctness，再 cache performance。

### 缓存有效性

每个 subtree command page 的 key 除 native size/style revision 外包含：

- inherited text/primary content style epoch，
- attached Parley layout font/scale version、accepted width/write-scale，
- renderer shared device & backend generation atlas/shader,
- clip/draw layer, opacity/filters parent applicable properties,
- viewport size/mount generation for GPU external pieces。

位移或 ancestor pure transform 是 pass-level composition change，不一定重新 flatten 几何/编码，但不要伪装控制 native readback/cross stays same。Occlusion、裁剪相交、parent clip/blend mutation，必须更新 composite dependency。旧 FrameState 只有在 successful present 后才代表已提交；否则 old damage 保留到下次（参考 FLUI ADR-0304）。

### Native external image 不能只凭名称判断

Classic 0.12 `register_texture` 通过 copy-src 经 atlas 复制，不是零拷贝采样；其 fine shader 的 alpha store需按版本检查。新 vello_GPU TextureBindings 明确有 external TextureId 机制及格式/capabilities。**因此这里的抽象是 framed texture ref+frame epoch+declared binding**，不是万物 VelloImage。UI renderer graphs 强制 resource borrows/ownership 与所用生命周期。

## 6. 可选 3D：model viewport

```rust
// API 目标，不是当前 PoC 已实现 glTF。
view! {
    <ModelViewer
        class="aspect-video w-full rounded-xl bg-black"
        src={asset}
        camera={move || orbit.camera()}
        playing={move || !paused.get()}
    />
}
```

`ModelViewer` 在二维 Taffy leaf，具有 rect、scale、viewport scissor/focus/capture 与生命周期。加载多线程做 glTF buffer/material mesh，上传 queue owner,fields；提供 camera orbit,zoom、pass styles、paused/redraw; 可选 animation clip未加载完成 fallback，All failure states render in 2D label proper accessible label。

默认先不嵌整个 Bevy/Ecs scene graph：

- `gltf` 读取 glTF 2.0 buffers/index data/material/basecolor texture/Joint transform 子集，严格 unsupported-extension diagnostic。 `glam` 做 camera transforms。
- 自有最小 wgpu vertex/index pipeline + offscreen color/depth + GLB/GLTF import；material render PBR completeness 渐进，不把 docs “支持” #extension feature 自动当 renderer conversion。
- resources slots,scene active generation, depth clear each rendering, camera uniform general; not depth blending 2D pixel vs mesh vertices across Vello z-layers.
- GPU payload formats/unpremultiplication/srgb sample color policy 由 Rendergraph state explicit。rgba 8 target format 可能要求 clone matching texture view formats and render attachments actual capabilities。

### 共享渲染图（主干简单场景）

**生产 baseline 推荐外部纹理 viewport，而不是优先写同一目标。**

```text
model & animations → 3D offscreen color(+own depth, MSAA optional) → resolve
                   → ordinary Vello GPU TextureBindings / TextureId
                   → final Vello 2D scene → surface presentation
```

* 经典线的 `Rgba8Unorm`、Classic separate color output 和新 sparse-strip 的 premultiplied contract **不同**；每次绑定外部资源都要附带格式/颜色空间/alpha、GPU submission serial/generation 声明。
* `wgpu::ExternalTexture` 面向外部视频/特殊 multi-planar 内容，不是每个普通 `wgpu::TextureView` 的替换名字或跨平台硬必需 feature。
* 同一个颜色纹理不能在同一个 pass 写又作为 Vello sample；CPU fallback 没有 magic ExternalTexture，3D 要可选占位、或明示昂贵 CPU readback。
* 不把 Vello 内部 depth_view 当通用 3D depth 共享接口。每个 viewport 自管 color/depth/true geometry order，2D 图形只在前后层合成。

Classic 0.12 PoC 验证的是另一种合法路径：

```text
UI underlay → external 3D render (preserve existing 2D, clip/scissor viewport)
            → UI overlay from separate texture (definite alpha convention)
            → composite / TextureBlitter to surface
```

它证明不同 renderer 可以有序使用 a GPU target，但生产 sparse-strips 使用标准 `TextureBindings` 更自然且接口清晰。Vello Classic 的 `RenderParams.base_color` 在 writes 前清理/确立底层，不能把“加载已有 UI target”当作 Vello Rendering 语义；注意区分 Vello 对单独 Scene 的输出和我们全帧 render graph。render_state 把 above/below sorted commands 缓存，2D 任意 perspective 与真实 3D 深度交错是另外一个专项，不属于基本 viewport 能力。

### 可替代实现选择

- external model to texture + TextureBindings：生产建议的默认；正常 transform/opacity/clip/filter 都由 2D 负责，resource generation 帧 reject。
- directly render into shared targets：本次 Classic 已验证的 POC，可继续给特殊场景测量采样成本；不要求所有 backend 都让其成为通用能力。
- 未来 heavy renderer，可 feature `zenui-port-3d` 之类仅接 GPU Resource Boundary，不改 2D label widgets；不允许再建立独立的 native window/swapchain present。
- heavy backend 与 UI 静态绘制分别调度，从已确认 bounded 3D 需求驱动功能，不把 serializer 支持列表等同已实现 shader。

3D 拾取应 view-local input → mesh ray intersection，holds focus/keyboard optional capture exit；UI gesture code 与普通 view pointer 路径相同。它不会用 flatten hit map 行为假装 UI descendants DOM body automatically。
