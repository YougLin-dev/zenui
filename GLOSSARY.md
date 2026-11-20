# ZenUI

ZenUI 是一个 Rust 原生桌面界面框架；这个上下文覆盖 target SDK、headless runtime 与 Windows 宿主之间的共享语言。

## Language

**Activation**:
一个窗口实例的身份；关闭后旧 activation 的交付被丢弃，重开产生新 activation。
_Avoid_: window instance, window generation

**Host**:
拥有原生窗口、事件循环、DPI 与 surface，并驱动 target runtime 的平台层。
_Avoid_: platform backend, shell

**Presentation snapshot**:
从已提交界面状态生成的一次性只读描述，含稳定 node ID、tag、渲染所需属性、文本与本地编辑状态；不含几何、闭包或句柄，也不是第二棵保留树。
_Avoid_: render tree, view tree, frame

**Scene**:
`zenui-render` 根据 presentation snapshot 产出的布局几何、文本 run 与绘制指令；与 GPU 后端无关。
_Avoid_: display list, frame

**Viewport**:
一次场景构建使用的逻辑视口尺寸与 Window scale；布局始终使用逻辑像素。
_Avoid_: screen size, resolution

**Text run**:
场景中同一字体、字号与 shaping 结果的一段文本，携带绝对字形位置；不是平台文本句柄。
_Avoid_: glyph line, paragraph

**Surface**:
一个顶层窗口的呈现目标；随窗口尺寸、DPI 或设备代际变化重建。
_Avoid_: swapchain, render target

**Node ID**:
presentation snapshot 中用于命中测试与事件派发的稳定元素身份；不是 label，也不是公开的内部数组下标。
_Avoid_: element index, view ID

**Text edit**:
在 caret/selection 上执行的平台无关编辑操作，例如插入、退格、删除；每次编辑派发一次 `on:input`。
_Avoid_: input event, keystroke

**Transaction frame**:
core 的一次事务提交单位；无修饰的 “frame” 不用于 presentation。
_Avoid_: frame

**Window scale**:
窗口逻辑坐标到物理像素的缩放；布局始终使用逻辑像素。
_Avoid_: DPI factor, zoom
