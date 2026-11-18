---
doc-id: SPEC-RT
layer: spec
status: draft
date: 2026-11-18
owner: 顶级 Rust 开发工程师（10 年+）
sources:
  - archive/review/FINAL.md
  - archive/research/runtime-and-rendering.md
  - archive/review/VALIDATION.md
replaces: []
---

# SPEC-RT 运行时：权威、身份、Owner、响应式与帧事务

> 本文件定义**要建什么**，不声明已实现。事实状态归 `docs/evidence/` 的 `CLAIM-*` 与 `EVID-VAL-*`；本文件只给出目标类型形状与失败模式，供 A0 fixture 反向实现。
>
> 版本 pin 的唯一出处是 [spec/01-route.md](01-route.md)（`SPEC-ROUTE`）。本文提到的 crate 名与轮子名一律不携带版本号。
>
> 状态词只使用 `SPEC / PROBE / VERIFIED / WITHDRAWN / OPEN`；spec 层不使用 `VERIFIED`。历史材料 `archive/research/runtime-and-rendering.md` 与 `FINAL` 冲突时以 `FINAL` 为准，被取代的表述收在 §7。

## 0. 责任边界：谁拥有什么，谁不能碰谁

| 层 | 拥有 | 只接收 / 只读 | 禁止事项 |
|---|---|---|---|
| **运行时核心（zenui-runtime）** | retained View 树、`ViewId + generation`、控件状态（focus/scroll/selection/composition/hit mapping）、OwnerScope 树、响应式图、事务 journal 与提交 | 宏 / typed builder 产出的 `MountPlan`；布局 adapter 交来的几何与映射变更 | 解析 class 或文本、持有 GPU 资源、调用 present、决定断行 |
| **Store adapter** | 模型实体（row + 字段）、字段/查询依赖边、membership 候选索引、query cache revision | 业务写命令、tracked 读取 | 直接创建/修改 View、另跑全量 render、维护第二份 UI 副本 |
| **宏 / typed builder（SPEC-MACRO）** | 节点与属性分类、descriptor、绑定槽、编译诊断 | 运行时公开的 owner/binding API | mount 前求值绑定、直接建 View、自行提交事务 |
| **布局 adapter（见 SPEC-TEXT / SPEC-RENDER 边界表）** | `LayoutNodeId ↔ taffy::NodeId` 私有映射、box 几何、`GeometrySnapshot` | 已提交的 View 树投影 | 持有 View 所有权、读取响应式图内部、把 `NodeId` 暴露到业务层 |
| **文本（SPEC-TEXT）** | shaping、编辑会话、坐标转换 | committed 值 / origin ticket | 写 Store、提交 UI、决定 box 尺寸 |
| **渲染（SPEC-RENDER）** | retained paint page、SceneAdapter、能力探测 | 已提交帧的几何与绘制记录 | 驱动结构变更、创建 View、回滚业务事务 |

**三条不可越界**：

1. 事务之外的任何路径（事件回调、cleanup、Effect、worker 线程）MUST NOT 创建或销毁 View。
2. layout / paint 的任何结论 MUST NOT 作为业务状态回写；业务状态只能经 Source 写入。
3. 宏与 inspection 结构 MUST NOT 被提升为运行时权威（见 `SPEC-ROUTE-024`）。

## 1. 权威与身份

```text
Application
 ├── ReactiveRuntime（唯一 UI 主线程）
 │    ├── Owner 树：子作用域、context、cleanup
 │    ├── Signal / Memo / query arena + 订阅边
 │    └── 帧事务：Mark → 拓扑求值 → Binding → Preflight journal → Commit
 ├── Retained View 树（唯一 UI 权威）
 │    ├── ViewId + ViewGeneration + NodeKind + typed widget state
 │    ├── 字段来源与本地动态绑定
 │    ├── LayoutNodeId ↔ taffy::NodeId（adapter 私有）
 │    └── focus / pointer capture / scroll / IME owner
 └── 派生（不可独立提交）：StyleProgram 投影、几何快照、retained paint records
```

### SPEC-RT-001 单一 UI 权威：一棵 View 树 + 一张响应式图

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：框架 MUST 恰好有一棵 retained native View 树持有 `ViewId + generation` 与全部控件状态，且恰好有一张细粒度响应式图承担脏传播与 patch 计算。布局几何、样式投影、retained paint record 都是**派生/cache**，MUST NOT 成为可独立提交的第二权威。
- **MUST NOT**：引入 VDOM reconcile、组件重跑重建、或把 Vello Scene / inspection Node 当作可提交的 UI 单元。
- **失败模式**：同一字段更新出现两条 publish 路径 → trace 中一次写触发两次 publish，或 stale generation 被接受；A0 事务验收必挂。
- **来源**：FINAL §0 / §1.1；`runtime-and-rendering.md` §1；`SPEC-ROUTE-010` / `-020` / `-021`。

### SPEC-RT-002 身份域分离：每种资源一个 newtype，禁止裸 `u64` 兼任

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：以下身份域 MUST 各自为独立 newtype（内部字段私有，只允许运行时代码构造）：

| 域 | 类型 | 由谁拥有 | 生命周期 |
|---|---|---|---|
| 界面稳定身份 | `ViewId` | View 树 | unmount 前不变 |
| 作用域清理 | `OwnerId` | Owner 树 | 随 owner dispose 失效 |
| 依赖图源 | `SignalId` / `MemoId` / `QueryId` | 响应式图 | 随 owner dispose 失效 |
| 布局节点 | `LayoutNodeId`（内部映射 `taffy::NodeId`） | 布局 adapter | 随 View 删除 |
| 外部资源 | `ResourceId`（texture / worker 资产） | 资源表 | 随 owner generation 失效 |
| 无障碍节点 | AccessKit 节点 ID | 平台 adapter | 由 `ViewId` 稳定派生 |

- **MUST NOT** 用一个裸 `u64` / `usize` 同时充当上述任意两个域；MUST NOT 在公开 API 暴露 `taffy::NodeId` 或资源内部句柄（`SPEC-ROUTE-012`）。
- **失败模式**：把 `OwnerId` 当 `ViewId` 传入 → 释放错误作用域或静默空操作；裸整数复用让跨域误用在类型检查中不可见，只能靠运行时随机崩溃暴露。
- **来源**：FINAL §1.1 / §8；`SPEC-ROUTE-031`；`runtime-and-rendering.md` §1。

### SPEC-RT-003 稳定身份与 generation 校验链

- **状态**：SPEC
- **强度**：MUST
- **要求**：`ViewId` 在 reorder、resize、样式变化、字段更新时 MUST 保持不变；只有 unmount / remount 才产生新的 `ViewGeneration`。命令与结果的校验 MUST 按固定顺序进行：
  1. 事件、AT action、IME 命令先校验 UI `ViewId + generation`；
  2. 业务写再校验 Store row generation；
  3. 异步完成再校验 resource / owner ticket。
- **失败模式**：快速切换 filter 时一次点击命中刚被删除、但位置相同的行；过期的 worker 结果覆盖新请求的模型状态。
- **来源**：FINAL §1.1 末段 / §1.3；`SPEC-RENDER-002` / `-017`。

### SPEC-RT-004 三类失效必须解耦，不得互相代偿

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：
  - 过滤消失 MUST 只 dispose 对应 UI owner 与订阅，**不删除**模型 row；
  - 模型 row 删除 MUST 使该 row 的 field handle 失效（`InvalidHandle`），但 MUST NOT 复活已 dispose 的 UI；
  - View 结构删除 MUST 使旧 `ViewId/generation` 的命令失效，**不表示**模型删除。
- **MUST NOT** 用 `display:none` / `visibility:hidden` 冒充过滤卸载（呈现策略的独立语义见 `SPEC-RENDER-016`）。
- **失败模式**：筛选切走后输入框仍持有订阅与 IME 组合态（泄漏）；或模型删除后旧 handle 读到同 slot 的新行（别名）。
- **来源**：FINAL §1.1 / §1.3；`SPEC-ROUTE-026`。

### SPEC-RT-005 checked field handle 的形状与失败语义

- **状态**：SPEC
- **强度**：MUST / SHOULD
- **要求**：生产 field handle MUST 是携带 `{runtime_id, slot, generation}` 的小型 `Copy` 结构：

```rust
struct FieldHandle<T> {
    runtime_id: RuntimeId,
    slot: SlotIndex,
    generation: Generation,
    _marker: PhantomData<fn() -> T>,
}

impl<T: Copy> FieldHandle<T> {
    fn get(self) -> Result<T, InvalidHandle>;          // 只读 Copy 标量/enum
}
impl<T> FieldHandle<T> {
    fn with<R>(self, f: impl FnOnce(&T) -> R) -> Result<R, InvalidHandle>; // 非 Copy 借读
}
```

- lookup MUST 返回 `Result<_, InvalidHandle>`；**MUST NOT** 返回残留旧值、静默转入同 slot 的新对象、或静默丢弃写入。
- slot 复用 MUST 分配新 generation；跨 runtime 传递的 handle MUST 被拒绝。
- `Copy` 不等于永久有效，也不表示访问免费（`SPEC-ROUTE-025`）。
- **SHOULD**：内部可用 `Rc`/`RefCell` 实现，但 MUST NOT 把 `Rc<RefCell<T>>` 变成公开的生命周期工具。
- **失败模式**：删除行后 `get()` 返回 `Ok(上一行值)`；旧 handle 写入被静默吞掉，界面冻结且无诊断。
- **来源**：FINAL §1.1 / §8；VALIDATION §4.3；`SPEC-ROUTE-025`。

### SPEC-RT-006 读取错误是类型，不是默认值

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：`read_once()` 一类一次性读取 MUST 返回真实 `Result<String, InvalidHandle>`。失败 MUST 取消本次 mount（见 `SPEC-RT-013`），MUST NOT 用空串、默认标题或“上一个合法值”替代。数据 binding 可由 runtime adapter 把 `Result` 交给统一 sink（`SPEC-RT-057`），但 MUST NOT 隐式降级为旧值。
- **失败模式**：无效 handle 被渲染成空白标题，用户看到“字段消失”而无任何错误记录；`Result` 被塞进 text 节点显示 `Err(InvalidHandle)`。
- **来源**：FINAL §1.2 / §3.2。

## 2. 组件 mount 与 MountPlan

### SPEC-RT-010 组件实例只 mount 一次

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：组件的 setup 体 MUST 在实例 mount 时**恰好执行一次**。父级字段变化、样式变化、reorder MUST NOT 重跑 setup；结构变化只能经事务 patch 表达。
- **失败模式**：编辑一行标题导致整列组件 setup 重跑 → 编辑器选区/IME 状态丢失、订阅重复挂接、trace 出现同一组件两次 mount。
- **来源**：FINAL §0 / §1.2；`SPEC-ROUTE-010`。

### SPEC-RT-011 mount 返回可失败的 `MountResult<MountPlan>`

- **状态**：SPEC
- **强度**：MUST
- **要求**：组件 mount 的返回类型 MUST 是 `Result<MountPlan, MountError>`（别名 `MountResult`）。函数体中的 `?` MUST 保持普通 Rust 语义；`#[component]` 把无显式错误的函数体包成 `Ok(plan)` 的行为由 `OPEN-MACRO-03` 决定，两种形式 MUST 语义等价。`view!` 与 typed builder MUST 产出同一 golden IR（宏侧见 `SPEC-MACRO-008`）。
- **失败模式**：mount 返回部分 plan；宏把 `MountError` 转成 panic 或默认节点。
- **来源**：FINAL §1.2 / §4 / §8。

### SPEC-RT-012 MountPlan 的形状与提交边界

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：`MountPlan` MUST 只包含描述与绑定槽，不含已创建的 View：

```rust
struct MountPlan {
    kind: ViewKind,                    // intrinsic / 组件根
    initial_fields: InitialFields,     // 仅 mount 初值
    bindings: Vec<BindingSlot>,        // 待安装的 tracked binding
    children: ChildrenPlan,            // 惰性 factory，不是已执行结果
    listeners: Vec<ListenerSlot>,      // payload typed，不在 mount 执行
}
```

- descriptor MUST 在事务 commit 时才创建 retained View + `ViewGeneration` 并注册 binding；MUST NOT 支持单独 `fire` / `evaluate` / `render` 一个 descriptor。
- **MUST NOT** 把现有 inspection `Node` 改名为生产 `MountPlan`（`SPEC-ROUTE-024`）。
- **失败模式**：mount 阶段建 View → 后续失败留下孤儿节点；descriptor 被当作可渲染对象绕过事务。
- **来源**：FINAL §1.2。

### SPEC-RT-013 mount 失败的原子性

- **状态**：SPEC
- **强度**：MUST
- **要求**：mount 失败（`?` 早退、`InvalidHandle`、类型化能力缺失）MUST 使该次 mount 整体不提交：不产生节点、binding 槽、订阅、默认替代文本；错误 MUST 进入统一 sink。同事务内已提交的兄弟节点 MUST NOT 受影响。
- **失败模式**：半棵树（父节点存在、子节点缺失，布局与命中错位）；失败 mount 遗留订阅，父级后续更新仍触发其 compute。
- **来源**：FINAL §1.2 / §3.2 / §9 A0。

### SPEC-RT-014 props 与 children 的执行边界

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：
  - 普通组件实参 `{expr}` MUST 按 Rust 值/handle 原样传递（宏侧 `SPEC-MACRO-002`/`-007`），运行时 MUST NOT 二次求值或替换类型；
  - mount 期对 owned `String` 的读取 MUST 恰好一次（`SPEC-RT-045`）；
  - `ChildrenFactory` MUST 在父级分支活跃时按需执行；`Show` 为 false 时 MUST NOT 提前执行。
- **失败模式**：隐藏分支的 factory 提前执行 → 不可见控件仍产生副作用、订阅和 I/O；props 被二次求值导致 `Signal` handle 变成当前值。
- **来源**：FINAL §1.2 / §1.3。

### SPEC-RT-015 listener 的归属与调用语义

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：每个 listener MUST 归创建它的 Owner 所有，owner dispose 时 MUST 注销；mount MUST NOT dispatch；handler 体内的读取 MUST NOT 登记依赖（宏侧 `SPEC-MACRO-005`）。handler 写操作返回的错误 MUST 经 result-aware adapter 或应用 sink 处理，MUST NOT 被 `let _ =` 吞掉。
- **失败模式**：dispose 后仍收到 click 并写已释放的 handle；mount 期执行 handler 造成首帧副作用；handler 内 `.get()` 被当成 binding 依赖，一次点击触发额外 flush。
- **来源**：FINAL §1.2。

## 3. OwnerScope 与结构生命周期

### SPEC-RT-020 OwnerScope 是三类资源的唯一注销出口

- **状态**：SPEC
- **强度**：MUST
- **要求**：每个组件实例与每个结构子树 MUST 有独立 Owner。订阅、异步 ticket、cleanup handler（以及 owner 局部 context）MUST 归属其 Owner；owner dispose MUST 负责注销全部三类资源。MUST NOT 依赖 `Rc` 引用计数自然归零来隐式完成释放。
- **失败模式**：资源随 `Rc` 晚一帧释放，churn 后活跃订阅数单调增长；timer/resource handle 在 owner 已关闭后仍回调。
- **来源**：FINAL §1.1 / §1.3；`runtime-and-rendering.md` §1 / §2。

### SPEC-RT-021 递归释放顺序：后序 + 逆注册

- **状态**：SPEC（顺序语义已由 `ADR-0016` 固化；实现与 fixture 仍为 `OPEN-RT-07`）
- **强度**：MUST / MUST NOT
- **要求**：
  1. 释放 MUST 按 Owner 树**后序**执行：先子树，后父作用域；子 cleanup 因此不会读到已释放的父槽。
  2. 同一 owner 内多个 cleanup MUST 按**注册逆序**（LIFO）执行。
  3. cleanup 中 MUST NOT 发起 mount、batch 或 flush；cleanup 期间产生的资源 MUST 归同一 owner 或直接被拒绝。
  4. `dispose` MUST 幂等；重复 dispose MUST 无副作用。
  5. View 的删除 MUST NOT 在 cleanup 中直接改树，只入事务 journal（`SPEC-RT-052`）。
- **失败模式**：父作用域先释放，子 cleanup 随后读取已失效 handle 并写 sink → 每次卸载都报错；cleanup 内 mount 使释放过程中树增长（重入）；重复 dispose 导致双重注销。
- **来源**：FINAL §1.3 / §3.2；顺序规则见 [ADR-0016](../decisions/adr-0016-release-order.md)。

### SPEC-RT-022 所有权无环

- **状态**：SPEC
- **强度**：MUST
- **要求**：子作用域对父作用域 MUST 用弱引用；事件 handler MUST NOT 成为 effect 生命周期的延长理由；数据 effect MUST NOT 回指父 owner。dispose 时弱边 MUST 注销。
- **失败模式**：闭包环使 owner 永不释放，10⁴ 次 mount/unmount 后活跃 owner/edge 计数不回落（A0 churn fixture 必挂）。
- **来源**：FINAL §1.3；`runtime-and-rendering.md` §1。

### SPEC-RT-023 `Show` 创建/销毁 child owner

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：`Show` 的分支切换 MUST 实现为子 owner 的 mount/unmount，MUST NOT 编译为 `display:none` / `visibility:hidden`。重新变为 true MUST 创建新 owner 与新 generation；旧 click/IME/resource ticket MUST 永不复活。`fallback` MUST 惰性求值，仅在 false 分支活跃时执行（宏侧 `SPEC-MACRO-010`）。
- **失败模式**：用隐藏代替卸载 → 隐藏控件仍参与布局、命中、语义树，订阅与定时器泄漏。
- **来源**：FINAL §1.3；`SPEC-RENDER-016`。

### SPEC-RT-024 `For` 按 key 保留、reorder 与销毁

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：
  - key MUST 显式（宏侧 `SPEC-MACRO-011`）；MUST NOT 有 index fallback。
  - 重复 key MUST 在任何 native patch 之前被拒绝，并保留**上一个合法**的 View 状态（不允许部分 reorder）。
  - reorder MUST 保留 `ViewId`、generation、Owner、订阅与局部控件状态（caret/selection/focus/scroll）。
  - key 消失 MUST 只 dispose UI owner 与订阅；row 模型不归 `For` 拥有。同 key 再次出现 MUST 创建新 UI generation。
- **失败模式**：重复 key 部分应用 → 节点交错、焦点跳到错误行；reorder 重建行 → caret 回行首、输入法组合态丢失。
- **来源**：FINAL §1.3 / §9 A0。

### SPEC-RT-025 `Index` 只按位置更新

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：`Index` MUST 按位置保留行，把 item 暴露为 signal；长度变化 MUST 只增删尾部；内容变化 MUST 更新对应 item signal。MUST NOT 用于会重排的数据（宏侧 `SPEC-MACRO-012`）。
- **失败模式**：对重排列表使用 `Index` 编译通过但 caret/焦点跟错行——无编译错误、纯行为错误，必须由文档与 fixture 双向约束。
- **来源**：FINAL §1.3。

### SPEC-RT-026 `FocusRemovalIntent`：结构 mutation 与焦点结算分离

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：结构 mutation 前 MUST 捕获 `FocusRemovalIntent`：受影响控件、同 kind 的前后邻居、可用的输入回退、command generation。reducer MUST 只在 commit 后、且目标仍存活可见时结算，顺序为：同 kind 下一项 → 同 kind 上一项 → 可聚焦输入 → filter toolbar。批量 Clear MUST 只结算一个 intent。用户在 mutation 之后、dispatch 之前主动转移焦点时，原 intent MUST 取消。
- **MUST NOT** 在删除最后一个匹配项后伪造“下一个”目标；允许落到输入。
- **失败模式**：删除聚焦行后焦点跳回文档根或被抢回用户刚点击的输入框；批量删除触发 N 次焦点跳转。
- **来源**：FINAL §1.3；§9 A0。

## 4. 响应式：binding、Memo、batch 与异步

### SPEC-RT-030 tracked binding 的安装与依赖登记

- **状态**：SPEC
- **强度**：MUST
- **要求**：binding MUST 在 owner 内安装一次，求值 MUST 处于 tracked 上下文并登记 `(source, token)` 依赖；同一 binding 对同一字段 SHOULD 只产生一条边。binding 体 MUST 是纯计算：只产 typed patch，MUST NOT 写 Signal/Store、MUST NOT mount。
- **失败模式**：binding 内写 → `WriteDuringCompute`（`SPEC-RT-038`）；重复边 → 一次字段写触发同一 binding 多次 compute。
- **来源**：FINAL §1.2 / §3.1；`runtime-and-rendering.md` §2。

### SPEC-RT-031 拓扑求值：只在依赖稳定后发布

- **状态**：SPEC
- **强度**：MUST
- **要求**：脏标记 MUST 沿图传播，Memo/query MUST 在其依赖达到稳定值后求值；MUST NOT 先向下游发布中间值。diamond、长链、动态条件切换 MUST 不产生 glitch。验收 MUST 观测发布值与每个稳定 epoch 的可观测次数，MUST NOT 笼统要求“所有 Memo 恰好执行一次”。
- **失败模式**：`count` 与 `count * 2` 在一个 epoch 内出现不一致组合（UI 闪一帧 `2 / 2`）；动态切换后下游停留在旧分支值。
- **来源**：FINAL §3 / §9 A0。

### SPEC-RT-032 equality 抑制只属于 Memo

- **状态**：SPEC
- **强度**：MUST
- **要求**：相等性比较 MUST 由 Memo 负责；值相等时 MUST NOT 增加 revision、MUST NOT 通知下游。下游 MUST NOT 重复实现 equality 来“修正”传播。Effect 自身 MUST NOT 被赋予自动 equality 抑制来替代 Memo 语义。
- **失败模式**：无抑制 → 每帧全下游重算；比较实现错误（如忽略字段）→ UI 停在旧值且无诊断。
- **来源**：FINAL §3；`runtime-and-rendering.md` §2。

### SPEC-RT-033 `batch` 是嵌套深度事务

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：`batch` MUST 记录嵌套深度，只有最外层离开时才结算并提交一次。内层 MUST NOT 泄漏中间值；MUST NOT 在 binding 事务中同步 flush。
- **失败模式**：现有 flat PoC 在嵌套 batch 中实际产生过 `true` 中间值、随后才到 `false`（FINAL §9 A0 已记录）；一次交互提交两次。
- **来源**：FINAL §3.1 / §9 A0。

### SPEC-RT-034 不活跃分支必须解绑

- **状态**：SPEC
- **强度**：MUST
- **要求**：`Show=false`、条件分支退出、`For` 移除产生的依赖 MUST 解绑并释放边；空 subscriber bucket MUST NOT 计为活跃源；分支重新激活 MUST 重新登记而非复用陈旧边。
- **失败模式**：churn 后活跃边数单调增长；隐藏列表的 Store 写仍触发 compute（“看不见的刷新”）。
- **来源**：FINAL §2 / §9 A0；`runtime-and-rendering.md` §2。

### SPEC-RT-035 `untrack` 只关订阅登记

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：`untrack` MUST 只关闭本次读取的依赖登记，MUST NOT 作为 measure / layout / commit 阶段的写许可。
- **失败模式**：把 `untrack` 当写权限 → 结构在测量中被修改，几何与已提交状态不一致；或相反，在 untrack 中读取后期待更新（值永久冻结）。
- **来源**：FINAL §3.1；`runtime-and-rendering.md` §2。

### SPEC-RT-036 Effect 只在 postcommit 运行

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：effect MUST 在事务提交后运行，承担 I/O 与异步收尾；MUST NOT 承担本帧必须同步完成的 binding。需要同帧生效的写回 MUST 由 tracked binding 表达。
- **失败模式**：依赖 effect 回写 → UI 晚一帧，出现闪烁；或把 effect 当 binding 导致一帧内两次 restore。
- **来源**：FINAL §3.1；`runtime-and-rendering.md` §2。

### SPEC-RT-037 异步任务绑定 owner 与 generation

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：异步任务 MUST 携带 owner 与 ticket（generation）；完成时 MUST 校验 owner 存活与 generation，过期结果 MUST 丢弃。普通 UI 状态默认属于 UI 主线程；worker 结果 MUST 经 channel 唤醒 UI 线程后提交，MUST NOT 从 worker 直接写 Signal。线程边界 MUST 显式（专用 `SyncSignal` 或授权 API）。
- **MUST NOT** 把核心运行模型绑定到 Tokio 一类异步运行时（`SPEC-SCOPE-014`）。
- **失败模式**：快速切换请求时旧结果覆盖新结果；跨线程写触发数据竞争或 `RefCell` panic。
- **来源**：FINAL §1.1 / §3.2；`runtime-and-rendering.md` §2。

### SPEC-RT-038 写权限按阶段划分

- **状态**：SPEC
- **强度**：MUST
- **要求**：只有事件与业务命令可以写 Signal/Store。Memo、binding、measure、layout 阶段的写 MUST 被判定为领域错误（`WriteDuringCompute`、memo cycle、phase violation）并以 typed 错误返回；MUST NOT 让 `RefCell` borrow 文案泄漏给用户，MUST NOT panic。运行时的内部 privileged 更新路径 MUST 与用户写权限分离。
- **失败模式**：布局测量中写 → 无限重排；用户看到 `already borrowed: BorrowMutError` 而非可执行的领域错误。
- **来源**：FINAL §3.1；`runtime-and-rendering.md` §2。

## 5. Store：模型权威与查询接线

### SPEC-RT-040 Store 是模型权威，不是第二刷新权威

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：Store MUST 只拥有模型实体（row + 字段）与字段/查询依赖，作为响应式图的适配层；界面刷新唯一路径仍是 View 树 + 响应式图。MUST NOT 在 Store 侧另跑全量 render、MUST NOT 维护第二棵 UI 副本或整树 diff。
- **失败模式**：Store 变更触发 UI 重建 → 第二条刷新路径（`SPEC-RT-001`），单字段更新重跑组件 setup。
- **来源**：FINAL §1.1 / §2 / §8。

### SPEC-RT-041 查询读取必须发生在 tracked 域内

- **状态**：SPEC
- **强度**：MUST
- **要求**：行字段读取 MUST 在 tracked 上下文内发生并登记依赖；字段边与查询边 MUST 属于同一张响应式图。查询值 MUST 由共享 query cache 求值，MUST NOT 由每个消费者各持一份 `Vec`。
- **失败模式**：查询在 `untrack` 中求值 → `completed` 改变但列表不更新；两个消费者各持副本 → 两份现实（`SPEC-ROUTE-030`）。
- **来源**：FINAL §2.1 / §2.2；`SPEC-ROUTE-030`。

### SPEC-RT-042 membership 候选索引 + 实际读取字段的反向边

- **状态**：SPEC
- **强度**：MUST
- **要求**：query 求值时 MUST 同时登记：
  1. 适用的 membership / 候选行索引；
  2. 求值**实际读取**的 `(row, field)` 反向边。
  候选索引 MUST 覆盖“当前不匹配、但字段变更后会进入 filter”的行；只给已物化成员建边会导致非 member 的字段写不标脏查询。
- **失败模式**：Done 列表漏掉刚被 toggle 成 completed 的行；`replace/unregister` 后遗留空桶或误连旧 consumer，后续改动打到已释放节点。
- **来源**：FINAL §2.2 / §2.7；`SPEC-ROUTE-030`。

### SPEC-RT-043 每 `(store_epoch, filter)` 至多物化一次

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：query cache MUST 在拓扑稳定后、真实 list 首读时求值并物化一次，第二个消费者 MUST 共享同一不可变快照（`Arc<[RowHandle]>` 或等价实现）。field write MUST NOT 直接 materialize `RowSet`，只能标脏 cache。
- **失败模式**：两个列表各建一份投影（O(n) 翻倍）；toggle 一行就立即 `collect` 全表。
- **来源**：FINAL §2.4 / §2.5。

### SPEC-RT-044 空态必须零物化

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：`has_rows` MUST 来自 store counter；`is_empty(filter)` MUST 来自 query 的标量 empty。两者 MUST NOT 通过 row collect、遍历或 `Vec::get` 实现。验收 MUST 断言 `row_visits = 0`、`materialization = 0`、`String clones = 0`，而不只是布尔返回值正确。
- **失败模式**：空列表每帧遍历一万行——结果正确但代价错误；性能问题被“功能正常”掩盖。
- **来源**：FINAL §2.5 / §2.6 / §9 A0。

### SPEC-RT-045 `RowSet` 与不可变标题的读取次数

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：`RowSet` MUST 是共享有序数组，只含稳定 id / handle，**不含 `String`**；`For` MUST 直接消费已声明的 `Memo<RowSet>`（宏侧 `SPEC-MACRO-011`）。不可变 title MUST 在每次 UI mount 时以 `Result<String, InvalidHandle>` owned 读取一次：filter 返场伴随新 mount 可再读一次；reorder 或 `completed` 改变 MUST NOT 重读。
- **失败模式**：每帧 `clone` 标题字符串；reorder 后重新读取产生无谓 I/O 与分配；把 `String` 放进 Copy 模型行（`SPEC-ROUTE-025` 已撤销）。
- **来源**：FINAL §2.4 / §2.8。

### SPEC-RT-046 聚合派生与反向 membership 边

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：`remaining` / `has_completed` MUST 由字段感应聚合或同名 query 索引派生；成员插入/删除 MUST 更新反向 membership 边；字段写 MUST NOT 漏发聚合更新。一个列表的行字段变化 MUST NOT 影响另一个不相关列表的读取与订阅。
- **失败模式**：删除行后 `remaining` 不变；A 列表 toggle 触发 B 列表 compute（跨列表串扰）。
- **来源**：FINAL §2.7。

### SPEC-RT-047 查询合同的验收必须计数

- **状态**：OPEN（合同明确，计数未采集）
- **强度**：MUST
- **要求**：`SPEC-RT-041`–`-046` MUST 以可观测计数验收：query evaluation、ID materialization、row visits、`String` clones、活跃依赖边数、订阅数。仅凭类型名或布尔返回值 MUST NOT 判定通过。
- **失败模式**：边数与物化次数无法证伪 → “看起来对”的实现留到性能阶段才暴露。
- **来源**：FINAL §2 末段 / §9 A0。

## 6. 帧事务

### SPEC-RT-050 单 UI 主线程与固定阶段顺序

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：全部结构、订阅与提交操作 MUST 在单一 UI 主线程完成（worker 只能按 `SPEC-RT-037` 提交结果）。一帧 MUST 按以下顺序推进：

```text
Outside    核 UI / model / resource generation；用户事件写 Source
Mark       source / 字段 / 查询索引标脏；嵌套事务只增厚深度
Topo       Memo / query 到稳定值才传播
Binding    只产纯 typed patch（禁止外部世界写）
Preflight  key / topology / style / generation / focus / IME / resource /
           LayoutNodeId 映射 → 写 journal（在权威树之外演算）
Commit     一次推进 model scratch + view scratch + owner/resource epoch
Cleanup    释放 staging 中已删除 owner；不 flush、不 mount、不 dispose 子回调
Derived    layout scratch → GeometrySnapshot → retained paint → present（SPEC-RENDER）
```

- **MUST NOT** 跳过 preflight 直接改树，MUST NOT 在阶段之间插入用户回调。
- **失败模式**：任一阶段被观测到部分提交；事件回调在 Commit 前运行导致读到临时代。
- **来源**：FINAL §3；`runtime-and-rendering.md` §3；渲染尾部细节见 `SPEC-RENDER-001`。

### SPEC-RT-051 preflight journal 的必备内容

- **状态**：SPEC
- **强度**：MUST
- **要求**：preflight MUST 把字段、key/order、resource/generation ticket、owner/focus/IME、`ViewId → LayoutNodeId` 映射、widget state 队列、style patch 全部写入 journal，并在权威树之外演算。映射的**所有权**属于布局 adapter，事务只保证这些变更与其一起原子提交。
- **失败模式**：先改树后失败 → 无 journal 可回滚，只能整树重建。
- **来源**：FINAL §3.2；`SPEC-RENDER-002`（映射归属）。

### SPEC-RT-052 原子提交

- **状态**：SPEC
- **强度**：MUST
- **要求**：`CommitPlan` MUST 一次性推进 model scratch、view scratch 与 owner/resource epoch；提交完成后，pending snapshot、geometry epoch、focus pointer、ID 映射 MUST 与 journal 逐项一致。
- **失败模式**：model 已提交而 view 未提交 → 下一帧读到悬空 handle；已提交 view 指向旧 geometry epoch → 命中错位。
- **来源**：FINAL §3.2 / §9 A0。

### SPEC-RT-053 失败回滚必须完整

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：任一 preflight 失败（duplicate key、`InvalidGeneration`、`InvalidHandle`、read failure、phase violation）MUST 丢弃本事务的全部 model/derived scratch 与 View journal。pending snapshot、geometry epoch、focus pointer、ID 映射 MUST 与上一已提交状态**完全一致**；MUST NOT 只回滚一部分。
- **失败模式**：失败后残留半更新的 focus pointer → 下一次点击命中“幽灵”节点；ID 映射残留 → 布局访问已释放节点。
- **来源**：FINAL §3.2 / §9 A0。

### SPEC-RT-054 commit 阶段不执行用户回调

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：commit 时 MUST NOT 执行用户 setter、cleanup callback 或事件 handler；cleanup 与 clocks MUST 在提交后开始。cleanup 中 MUST NOT 再次触发 flush。
- **失败模式**：cleanup 内写 Signal → 提交过程中重入，产生第二次 publish 或死锁。
- **来源**：FINAL §3.2。

### SPEC-RT-055 失败后必须能恢复

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：事务失败 MUST 恢复 reactive phase 与队列 guard，使下一个干净事务仍可提交。panic MUST NOT 作为业务错误 API（只允许作为额外故障边界的测试手段）。
- **失败模式**：一次失败后 runtime 卡在 dirty/busy，后续所有事务被拒（需要重启窗口）。
- **来源**：FINAL §3.2 / §9 A0（“随后一个干净事务仍可提交”）。

### SPEC-RT-056 外部 I/O 只在 postcommit 启动

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：文件保存、clipboard、网络等外部 I/O MUST 只在提交后启动；已发出的外部 I/O MUST NOT 被声称为可回滚。I/O 失败 MUST 有独立 `Result` 与恢复路径；保存 MUST 做 generation 校验，并按需 temp/fsync/rename。MUST NOT 用“回滚 GPU 提交”之类措辞假装业务事务可逆。
- **失败模式**：事务失败但文件已落盘，用户看到“保存失败”却内容已变。
- **来源**：FINAL §3.2。

### SPEC-RT-057 统一领域错误 sink

- **状态**：SPEC
- **强度**：MUST / MUST NOT
- **要求**：读取、binding、事件写入、mount 的错误 MUST 进入同一领域错误 sink，类型至少覆盖 `InvalidHandle` / `MountError` / `Unsupported`。sink MUST NOT 静默替换为旧值，MUST NOT 把 `Result` 塞进文本节点，MUST NOT panic。应用可选择展示或记录；错误展示的 UI 约束见 `SPEC-MACRO-024`。
- **失败模式**：无效 handle 被吞 → 字段冻结无提示；错误只写日志，用户界面与实际状态不一致。
- **来源**：FINAL §1.2 / §3.2 / §4。

### SPEC-RT-058 layout / paint 失败不得改写已提交业务结果

- **状态**：SPEC
- **强度**：MUST NOT
- **要求**：layout / paint MUST 在差分 scratch 中执行；失败 MUST NOT 事后改变已提交的业务结果。已提交事务之后的 unsupported 能力 MUST 以 typed `Unsupported` 或 last-good-frame 处理，细节归 `SPEC-RENDER-006` / `-008`。
- **失败模式**：一次纹理分配失败导致业务状态回退或重复提交；或反过来，绘制失败被当成业务成功。
- **来源**：FINAL §3.2 / §7.2。

## 7. 历史材料中不得复活的表述（WITHDRAWN）

| ID | 曾提出的表述 | 状态 | 撤销依据 |
|---|---|---|---|
| WITHDRAWN-RT-01 | `RenderBinding` + deferred commit queue 作为与 View 并列的第二提交点 | `WITHDRAWN` | FINAL §0/§3：只有一套事务；binding 只产 patch，由同一 commit 提交 |
| WITHDRAWN-RT-02 | Effect 由异步 runtime tick 调度并承担同帧值绑定 | `WITHDRAWN` | FINAL §3.1：effect 只做 post-frame I/O，不承担本帧同步 binding |
| WITHDRAWN-RT-03 | 单个裸 `u64` 兼作 `ViewId`/`OwnerId`/`SignalId`/`NodeId` | `WITHDRAWN` | `SPEC-ROUTE-031`；见 `SPEC-RT-002` |
| WITHDRAWN-RT-04 | `Copy` 结构体承载 `String` 的模型行 | `WITHDRAWN` | `SPEC-ROUTE-025`；VALIDATION §4.3 已实际触发 E0204 |
| WITHDRAWN-RT-05 | 通用 `Memo<T>::get()` 对非 `Copy T` 返回 owned clone | `WITHDRAWN` | FINAL §2.4：`get` 只允许 `T: Copy`，非 Copy 用 `with` |
| WITHDRAWN-RT-06 | 事务失败后按 `display:none` / hide-store 保留被过滤行的编辑状态 | `WITHDRAWN` | `SPEC-ROUTE-026` / FINAL §1.3：默认 dispose；保留须显式有界 `retained_visibility` |

引用纪律：提及上表内容时 MUST 写“已撤销，见 `WITHDRAWN-RT-0x`”，MUST NOT 作为现行 API 或默认行为描述。

## 8. OPEN：运行时的未决项

| ID | 未决项 | 影响 | 解除条件 |
|---|---|---|---|
| OPEN-RT-01 | 生产 Owner arena / 作用域释放未实现（现 PoC 只有平坦 sink 图） | `SPEC-RT-020`–`-026` 全部只能 `SPEC` | A0 的 mount/unmount churn fixture 给出 owner/edge 回落计数（`EVID-VAL-*`） |
| OPEN-RT-02 | 事务 journal 与回滚未实现 | `SPEC-RT-051`–`-055` | A0 注入 duplicate key / `WriteDuringCompute` / cycle，断言与上次提交逐项一致 |
| OPEN-RT-03 | 拓扑 Memo 与 cycle 检测未实现 | `SPEC-RT-031` | A0 diamond/长链/动态切换的发布值与 epoch 计数 |
| OPEN-RT-04 | `For` / `Show` / `Index` 未实现（宏侧亦为 `SPEC`） | `SPEC-RT-023`–`-025` | A1 golden + focus/caret fixture |
| OPEN-RT-05 | query 字段边、membership 索引与回收未连线 | `SPEC-RT-041`–`-047` | A0 的 edge_count / materialization 计数 |
| OPEN-RT-06 | `FocusRemovalIntent` 未实现 | `SPEC-RT-026` | A0 delete-focused / bulk-clear trace |
| OPEN-RT-07 | 释放顺序已由 `ADR-0016` 定序；**实现与用例**仍缺：`dispose` 幂等、LIFO 顺序、cleanup 中 mount 被拒 | `SPEC-RT-021` | A0 的 mount/unmount churn fixture + 一个“cleanup 中 mount 被拒”的负例 |
| OPEN-RT-08 | 是否复用第三方 owner/arena 库（研究材料建议评估）未裁决 | 实现成本与借用模型 | 独立 ADR；无论是否复用，render binding 提交层必须自有（`WITHDRAWN-RT-01`） |
| OPEN-RT-09 | 显式线程边界 API（`SyncSignal` 形状）未定 | `SPEC-RT-037` | worker→UI 提交的最小 fixture + API 评审 |

## 9. 与其他规范的关系

- 版本 pin：只引用 [SPEC-ROUTE](01-route.md)，本文不重复。
- 唯一路线与撤销清单：`SPEC-ROUTE-010/011/012/013/024/025/030/031`。
- 宏表面（位置分类、`class`、`For`/`Show` 语法、诊断）：[spec/03-macro.md](03-macro.md)（`SPEC-MACRO-*`）。
- 样式投影与优先级：[spec/04-style.md](04-style.md)（`SPEC-STYLE-*`）。
- 文本 shaping 与 TextEdit 双相位 reducer：[spec/05-text.md](05-text.md)（`SPEC-TEXT-012`–`-015`）。
- 几何快照、绘制与 present：[spec/06-render.md](06-render.md)（`SPEC-RENDER-001/002/006`）。
- Gate 归属：A0 负责本文的事务/身份/query 验收，A1 负责 `MountPlan` 与控件行为；见 [spec/08-gates.md](08-gates.md)（`SPEC-GATE-A0`、`SPEC-GATE-A1`）。
