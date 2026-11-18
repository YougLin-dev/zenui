# 原生宏与细粒度语法（方向草案）

> 状态：架构目标，不是已发布的语法承诺。验证项目会尽量采用这里的边界，但在宏命令稳定前，不把片段当作可直接运行的库示例。

## 1. 一个真实开发者该能读懂的界面

```rust
use zenui::prelude::*;

#[component]
fn ProjectCard(project: ProjectSignal, selected: Signal<bool>) -> impl View {
    let title = move || project.title.with(|s| s.clone());

    view! {
        <article
            class="group flex flex-col gap-3 rounded-xl border border-zinc-800
                   bg-zinc-950 p-4 transition-colors hover:border-zinc-600
                   active:border-amber-400"
            class:if-selected={move || selected.get()}
            aria:selected={move || selected.get()}
        >
            <view class="flex items-center justify-between gap-4">
                <text class="text-lg font-semibold text-zinc-100">
                    {title}
                </text>
                <button
                    class="rounded-md px-3 py-2 text-sm bg-indigo-600
                           hover:bg-indigo-500 focus-visible:ring-2 focus-visible:outline-none
                           disabled:opacity-50"
                    aria:label="打开项目"
                    disabled={move || project.loading.get()}
                    on:click={move || project.open()}
                >
                    "打开"
                </button>
            </view>
        </article>
    }
}
```

属性是普通 Rust 表达式。字符串字面量可写 HTML 属性，也可用字符串字面量表达文本。选中等应用状态的注册用法是草案；宏不执行 onclick 字符串，不要求组件参数都 `Copy`，也不支持虚假的 HTML 元素 API。

## 2. 动态值的唯一约定

### `{value}` 是一次性初值，`{move || expr}` 是更新边

```rust
view! {
    <view class="p-4">
        <text>"静态标题"</text>
        <text>{move || format!("未读 {} 条", unread.get())}</text>
        <progress value={move || download.progress.get()} />
        <button on:click={download.start}> "下载" </button>
    </view>
}
```

在**数据绑定位置** `move || expr` 被运行时安装在 reactive `RenderEffect` 中，信号读取在追踪阶段进行。普通值、引用、可变闭包用于组件 Props/事件，不会仅因为 `FnOnce` 就被误判为计算函数。为保证器类型和 API 可解释性，事件采用明确的 `on:` namespace，绑定返回值约束采用专用 trait，而不是尝试对任意 closure 魔法匹配。

考虑再加入静态分析帮助：

- `#[reactive]` 等扩展宏作为显式 escape hatch，不用于大体 API 热更新隐式黑箱。
- 初次写 `unread.get()` 的时刻仍表示追踪读取：事件/普通闭包不会订阅，因为执行时不处于 tracker context。
- `move || expr` 不是 JS getter adaptor，不需要任何额外 `Memo` 才能更新文本；`Memo` 只对需要缓存的可比较结果使用。
- 闭包必须把要捕获的值 `move` 进去。需要借用短期资源时，不假装 async/stream 的 resource 可以跨生命周期自动存在。

`Signal<T>` handle 可采用 Copy ID，但 **T 本身不必 Copy**。事件的读取也正常通过 handle 进入 arena；销毁后返回明确的 handle invalid 错误（开发诊断），不是 use-after-free。

## 3. 分支、列表与 Dispose

```rust
view! {
    <view class="flex flex-col gap-2">
        <Show when={move || state.user.is_some()}>
            <UserPanel />
        </Show>
        <Show
            when={move || has_data.get()}
            fallback={view! { <text>"暂无项目"</text> }}
        >
            <Statistics />
        </Show>
        <For
            each={move || store.visible_projects()}
            key={|project| project.id}
        >
            {move |project| view! { <ProjectCard project={project} /> }}
        </For>
        <Index each={move || store.lines()} >
            {move |line: Memo<String>| view! { <text>{move || line.with(Clone::clone)}</text> }}
        </Index>
    </view>
}
```

- `Show` 的 `when` 按 branch identity 切换时卸载子 owner，不能只把 `display:none` 当通用卸载。
- `For` 必须提供 key，不能缺省退回 index；重排时保留行节点、焦点、输入状态、scroll offset。删除 key 时销毁 owner。
- `Index` 按位置保留行，改变项时更新 item signal，只有缩短时释放尾部，不能用同一个组件偷换成 For。
- `Dynamic`/`Match` 仅用于切换真正结构分支；普通属性改变不应间接卸载整个父组件。
- `Suspense` 留到异步资源原型之后，不将 `Show(loading)` 也当作 Promise cancellation。

Rust 所有权的处理不要求 `project: ProjectRes` 在自 component props 额外 clone 原始内容：Copy handle → runtime ID 只是运行期引用，正确性靠 scope、失联检测与 weak 接口，不靠无限 clone。

## 4. component / Props / Children / Slot

```rust
#[component]
fn Panel(
    #[prop(default = Title::Native)] title: Title,
    class: ClassList,
    children: Children,
) -> impl View;

view! {
    <Panel class="p-4">
        <TitleBar title="设置" />
        <text>"正文 slots 不需要 HtmlElement。"</text>
    </Panel>
}
```
## 5. 宏内部管线（实现建议）- PascalCase 和路径名字表示 Rust 函数组件，类型还是 Rust 类型：数据传递、错误、IDE rename 不需要查找字符串 key。
- `children` 为可重复执行的 FnOnce/children factory，具体语义由 slot 类型指明。
- 想写 RSX 之外功能不再发明 JSX-spread；支持显式 `{..props}` 仅限 `Props` 拥有相同类型的组件，class 合并采用声明集合并，而非合并后重新拼接。
- style/class 优先级的合并由样式层完成。typed props 和 inline styles 不能改写批量 Style object 混合绕过校验。

Rust function 组件只在 mount 执行一次；返回的 builder 是静态节点的创建描述，绑定安装在各 Owner 的 slice 内。显式的 `builder` API 是可单测、不开宏的等价路径，不用 token-level magic 控制正常执行。

## 5. 宏内部管线（实现建议）

```text
tokens (`rstml` Node / attributes / expressions)
        │
        ▼
Zenui AST：intrinsic / component / slot / Show / For / Index / Rust block
        │  ├─ tag-name string normalization (on:click / aria:label)
        │  └─ syn/quotes preserve Rust source spans
        ▼
literal class parser + unit/value normalizer (subsequent zenui-style module)
        │  compile errors on unknown class/invalid values
        ▼
Typed builder code + reactive data bindings + initialization plan
        │
        ▼
Attach nodes & effects to current Owner
```

### 为什么复用 rstml 而不是全手写 `!parser`

- 已有清晰的 Element/Fragment/Text/Rust code/attributes/slots 与 source spans。
- Rust 逗号、闭包、嵌套块、generics 是普通 syn AST，不必自造表达式文法。
- 保留 `proc-macro-error` / `syn::Error` / `quote_spanned!` 指向某个 class 或属性的能力。
- 建立独立的 `style` crate 供宏和对开发工具重用一个字面量→native enum 的函数；不复制 parse logic。

### 汇编语义验证

用 `trybuild` 或宏内部 `proc_macro2` 展开测试：

1. 关闭标签、字符串 HTML、片段、嵌套 Rust 表达式正确。
2. `<Show>` 不额外包装原始 props；`<For>` key 不丢失，重复 key 在 runtime dev-check 失败。
3. literal class 的 unknown 指向具体 class，而不是整个宏报“token error”。
4. `on:click` 保持 closure 表达式、参数与 `'static` captured values；不会运行两次。
5. dynamic 文本一个订阅，不发生 setup 每信号重跑，不出现相同字段多次订阅。
6. 自定义 `data-*` / `aria-*` 映射 typed 字段，特定 UI 的隐藏/语义信息没有 browser DOM assumption。
7. 未预留的 variable CSS class 在动态编译/显式 manifest 中诊断，而不是悄悄忽略。

## 6. async/resource / app services

```rust
let profile = Resource::new(move || user_id.get(), fetch_profile);

view! {
    <view class="flex flex-col gap-2">
        <Suspense fallback={LoadingSpinner}>
            <Profile data={profile} />
        </Suspense>
    </view>
}
```

`fetch_profile` 可运行在资源执行器或宿主 runtime；需要 worker parallelism 时只把可交换数据指定为 `Send + Sync`。完成的 next Value 通过 window/wakeup queue 发送 UI runtime。若旧 resource key 改变或 owner 被 dispose，老结果不再覆盖新状态，并用显式 generation 丢弃。

文件 dialog / clipboard / drag 分别是 service，不混进 UI 树伪造同步 side effect。native OS / IME / AccessKit 的回调变为带 WindowId/NodeId/generation 的 action，UI 主线程处理后提交。

我们不要求 `async move || ...` 在普通属性中自动可用。同步值、异步 resource、hasMany、事件 entry 是有物理区分的类型/语法位置。
