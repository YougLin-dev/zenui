use std::any::Any;
use std::collections::BTreeSet;
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

use zenui_core::{PanicInfo, catch_panic};

use crate::context::{DepKey, Phase, release_read_only, with_state, with_state_of};

/// 可参与响应式图的值：可克隆，并有值语义相等。
///
/// `Signal`、`Memo` 与 `Read` 都用这个 bound。相等判断发生在类型擦除之后
/// 的重算边界：值相等的 memo 不推进 revision，也不标脏下游。
pub trait ReactiveValue: Clone + 'static {
    /// 值语义相等；用于 memo 的下游抑制。
    fn equivalent(&self, other: &Self) -> bool;
}

impl<T: Clone + PartialEq + 'static> ReactiveValue for T {
    fn equivalent(&self, other: &Self) -> bool {
        self == other
    }
}

/// 一次 memo 重算的完整契约：类型化调用与类型擦除相等。
///
/// [`memo`] 用 `T: ReactiveValue` 生成这个适配器；内部缓存只持有
/// `Box<dyn Any>`，因此下游抑制不依赖调用方再次提供类型。
pub trait MemoCompute {
    /// 执行一次计算，返回值以类型擦除形式交给 memo 缓存。
    fn call(&self) -> Box<dyn Any>;

    /// 比较同一 memo 的新旧值；任一值类型不符都是内部契约错误。
    fn eq(&self, old: &dyn Any, new: &dyn Any) -> bool;
}

struct TypedMemoCompute<T> {
    compute: Box<dyn Fn() -> T>,
}

impl<T: ReactiveValue> MemoCompute for TypedMemoCompute<T> {
    fn call(&self) -> Box<dyn Any> {
        Box::new((self.compute)())
    }

    fn eq(&self, old: &dyn Any, new: &dyn Any) -> bool {
        let old = old.downcast_ref::<T>().expect("memo old value type");
        let new = new.downcast_ref::<T>().expect("memo new value type");
        old.equivalent(new)
    }
}

pub struct Signal<T> {
    runtime: u32,
    id: u32,
    _marker: PhantomData<fn() -> T>,
    _not_send: PhantomData<*const ()>,
}

impl<T> Copy for Signal<T> {}
impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Signal<T> {
    /// 绑定槽保存的是来源引用；求值时按引用还原同一句柄。
    pub(crate) fn from_source(source: crate::node::SourceRef) -> Self {
        Self {
            runtime: source.runtime,
            id: source.id,
            _marker: PhantomData,
            _not_send: PhantomData,
        }
    }

    /// 绑定槽的来源描述；只携带 runtime 身份与槽位。
    pub(crate) fn source(self) -> crate::node::SourceRef {
        crate::node::SourceRef {
            runtime: self.runtime,
            id: self.id,
        }
    }
}

impl<T: ReactiveValue> Signal<T> {
    pub fn get(self) -> T {
        match with_state_of(self.runtime, |state| state.try_read_signal(self.id, true)) {
            Ok(value) => value,
            Err(crate::view::ReadFault::Disposed) => {
                panic!("signal belongs to a disposed owner")
            }
            Err(fault) => panic!("signal read failed: {fault}"),
        }
    }

    /// setup 阶段的一次性读取：不注册依赖，读取失败返回 typed fault。
    pub(crate) fn try_get_once(self) -> Result<T, crate::view::ReadFault> {
        with_state_of(self.runtime, |state| state.try_read_signal(self.id, false))
    }

    pub fn set(self, value: T) {
        with_state_of(self.runtime, |state| state.write_signal(self.id, value));
    }

    /// 只读作用域：内部的 setter 与模型事务在改动前被拒绝。
    pub fn with<R>(self, f: impl FnOnce(&T) -> R) -> R {
        struct Guard {
            runtime: u32,
        }
        impl Drop for Guard {
            fn drop(&mut self) {
                release_read_only(self.runtime);
            }
        }

        let _guard = Guard {
            runtime: self.runtime,
        };
        let value = with_state_of(self.runtime, |state| {
            state.read_only += 1;
            state.read_signal(self.id, true)
        });
        f(&value)
    }
}

impl<T> fmt::Debug for Signal<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Signal")
            .field("runtime", &self.runtime)
            .field("id", &self.id)
            .finish()
    }
}

pub struct Memo<T> {
    runtime: u32,
    id: u32,
    _marker: PhantomData<fn() -> T>,
    _not_send: PhantomData<*const ()>,
}

impl<T> Copy for Memo<T> {}
impl<T> Clone for Memo<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Memo<T> {
    /// 绑定槽保存的是来源引用；求值时按引用还原同一句柄。
    pub(crate) fn from_source(source: crate::node::SourceRef) -> Self {
        Self {
            runtime: source.runtime,
            id: source.id,
            _marker: PhantomData,
            _not_send: PhantomData,
        }
    }

    /// 绑定槽的来源描述；只携带 runtime 身份与槽位。
    pub(crate) fn source(self) -> crate::node::SourceRef {
        crate::node::SourceRef {
            runtime: self.runtime,
            id: self.id,
        }
    }
}

impl<T: ReactiveValue> Memo<T> {
    pub fn get(self) -> T {
        match self.read_result(true) {
            Ok(value) => value,
            Err(crate::view::ReadFault::Disposed) => {
                panic!("memo belongs to a disposed owner")
            }
            Err(crate::view::ReadFault::GraphCycle) => {
                panic!("memo {} was read while computing (cycle)", self.id)
            }
            Err(crate::view::ReadFault::ComputedPanicked) => {
                // 没有最后值时保留 compute panic 的诊断文本，调用方仍能看到
                // 用户闭包失败的原因；可失败读取则返回 typed ComputedPanicked。
                if let Some(panic) = self.last_panic() {
                    panic!("memo {} is poisoned: {panic}", self.id)
                }
                panic!("memo {} is poisoned and has no committed value", self.id)
            }
            Err(crate::view::ReadFault::FieldMissing(error)) => {
                unreachable!("a memo read cannot produce a field fault: {error}")
            }
        }
    }

    /// setup 阶段的一次性读取：不向上游 memo 注册依赖，失败返回 typed fault。
    pub(crate) fn try_get_once(self) -> Result<T, crate::view::ReadFault> {
        self.read_result(false)
    }

    /// memo 读取的唯一实现；`track` 决定是否向正在计算的父 memo 注册依赖。
    fn read_result(self, track: bool) -> Result<T, crate::view::ReadFault> {
        // 差分 oracle：绕过生产 memo 缓存与 dirty 标记，按 oracle 自己的
        // 拓扑重算值；生产缓存必须保持原样，否则比较的两个结果会互相污染。
        #[cfg(test)]
        if let Some(result) = crate::oracle::memo_read(self.runtime, self.id) {
            return result;
        }
        let ready = with_state_of(self.runtime, |state| state.memo_try_ready::<T>(self.id))?;
        if let Some(value) = ready {
            if track && with_state_of(self.runtime, |state| state.phase) == Phase::Memo {
                with_state_of(self.runtime, |state| state.note_dep(DepKey::Memo(self.id)));
            }
            return Ok(value);
        }
        let (compute, previous) = with_state_of(self.runtime, |state| state.begin_memo(self.id));
        match catch_panic(|| compute.call()) {
            Ok(value) => {
                with_state_of(self.runtime, |state| {
                    state.finish_memo(self.id, value, previous, compute)
                });
                with_state_of(self.runtime, |state| {
                    if track && state.phase == Phase::Memo {
                        state.note_dep(DepKey::Memo(self.id));
                    }
                    Ok(state.memo_value(self.id))
                })
            }
            Err(panic) => {
                with_state_of(self.runtime, |state| {
                    state.abort_memo(self.id, panic, previous);
                });
                match with_state_of(self.runtime, |state| state.memo_try_ready::<T>(self.id))? {
                    Some(value) => Ok(value),
                    None => Err(crate::view::ReadFault::ComputedPanicked),
                }
            }
        }
    }

    /// 该 memo 最近一次被捕获的 compute panic。
    ///
    /// 毒化只影响这个 memo：依赖变化或成功重算会清除它。读取仍遵循
    /// [`Memo::get`] 的最后值规则，本方法是这个切片的 typed 诊断出口。
    pub fn last_panic(self) -> Option<PanicInfo> {
        with_state_of(self.runtime, |state| {
            state.memos[state.memo_index(self.id)].poisoned.clone()
        })
    }
}

impl<T> fmt::Debug for Memo<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Memo")
            .field("runtime", &self.runtime)
            .field("id", &self.id)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Once<T> {
    value: T,
}

impl<T> Once<T> {
    pub fn into_inner(self) -> T {
        self.value
    }
}

pub fn signal<T: ReactiveValue>(value: T) -> Signal<T> {
    let (runtime, id) = with_state(|state| {
        let id = state.create_signal(Box::new(value));
        (state.id, id)
    });
    Signal {
        runtime,
        id,
        _marker: PhantomData,
        _not_send: PhantomData,
    }
}

pub fn memo<T: ReactiveValue>(compute: impl Fn() -> T + 'static) -> Memo<T> {
    let erased: Rc<dyn MemoCompute> = Rc::new(TypedMemoCompute {
        compute: Box::new(compute),
    });
    let (runtime, id) = with_state(|state| {
        let id = state.create_memo(erased);
        (state.id, id)
    });
    Memo {
        runtime,
        id,
        _marker: PhantomData,
        _not_send: PhantomData,
    }
}

pub fn once<T>(value: T) -> Once<T> {
    Once { value }
}

pub(crate) struct SignalCell {
    pub(crate) value: Rc<dyn Any>,
    /// 应用写入代数。相等写入同样推进：文本编辑器据此判断处理器是否处理过
    /// 一次编辑，而不是比较写入前后的值。
    pub(crate) writes: u64,
    pub(crate) disposed: bool,
}

pub(crate) struct MemoCell {
    pub(crate) compute: Option<Rc<dyn MemoCompute>>,
    pub(crate) value: Option<Rc<dyn Any>>,
    pub(crate) deps: Vec<DepKey>,
    /// 计算值变化时推进的代数。值绑定用它近似“处理器写过绑定”；
    /// 相等写入无法与推迟写入区分。
    pub(crate) revision: u64,
    pub(crate) dirty: bool,
    pub(crate) computing: bool,
    pub(crate) disposed: bool,
    /// compute panic 的诊断；依赖变化后清除，成功重算也会覆盖。
    pub(crate) poisoned: Option<PanicInfo>,
}

impl MemoCell {
    pub(crate) fn new(compute: Rc<dyn MemoCompute>) -> Self {
        Self {
            compute: Some(compute),
            value: None,
            deps: Vec::new(),
            revision: 0,
            dirty: true,
            computing: false,
            disposed: false,
            poisoned: None,
        }
    }
}

impl crate::runtime::State {
    fn signal_index(&self, id: u32) -> usize {
        id as usize
    }

    fn memo_index(&self, id: u32) -> usize {
        id as usize
    }

    pub(crate) fn create_signal(&mut self, value: Box<dyn Any>) -> u32 {
        let id = u32::try_from(self.signals.len()).expect("signal arena exhausted");
        self.signals.push(SignalCell {
            value: Rc::from(value),
            writes: 0,
            disposed: false,
        });
        self.attach_to_owner(crate::runtime::Resource::Signal(id));
        id
    }

    pub(crate) fn read_signal<T: ReactiveValue>(&mut self, id: u32, track: bool) -> T {
        match self.try_read_signal(id, track) {
            Ok(value) => value,
            Err(crate::view::ReadFault::Disposed) => {
                panic!("signal belongs to a disposed owner")
            }
            Err(fault) => panic!("signal read failed: {fault}"),
        }
    }

    pub(crate) fn try_read_signal<T: ReactiveValue>(
        &mut self,
        id: u32,
        track: bool,
    ) -> Result<T, crate::view::ReadFault> {
        let cell = &self.signals[self.signal_index(id)];
        if cell.disposed {
            return Err(crate::view::ReadFault::Disposed);
        }
        let value = cell
            .value
            .clone()
            .downcast::<T>()
            .unwrap_or_else(|_| panic!("signal {id} has a different value type"));
        if track && self.phase == Phase::Memo {
            self.note_dep(DepKey::Signal(id));
        }
        Ok((*value).clone())
    }

    pub(crate) fn write_signal<T: ReactiveValue>(&mut self, id: u32, value: T) {
        if !matches!(self.phase, Phase::Event | Phase::Delivery) || self.read_only > 0 {
            self.record_fault("Signal::set", self.phase);
            return;
        }
        let index = self.signal_index(id);
        if self.signals[index].disposed {
            self.record_fault("Signal::set", self.phase);
            return;
        }
        let cell = &self.signals[index];
        let current = cell
            .value
            .clone()
            .downcast::<T>()
            .unwrap_or_else(|_| panic!("signal {id} has a different value type"));
        // 相等写入不改变值，但仍是应用对这次写入的处理；编辑器依赖这个区别。
        self.signals[index].writes = self.signals[index].writes.wrapping_add(1);
        if current.equivalent(&value) {
            return;
        }
        self.signals[index].value = Rc::new(value);
        self.bump(DepKey::Signal(id));
    }

    pub(crate) fn create_memo(&mut self, compute: Rc<dyn MemoCompute>) -> u32 {
        let id = u32::try_from(self.memos.len()).expect("memo arena exhausted");
        self.memos.push(MemoCell::new(compute));
        self.attach_to_owner(crate::runtime::Resource::Memo(id));
        id
    }

    /// `memo_ready` 的可失败版本；setup 读取与绑定求值用它拿到 typed fault。
    pub(crate) fn memo_try_ready<T: ReactiveValue>(
        &mut self,
        id: u32,
    ) -> Result<Option<T>, crate::view::ReadFault> {
        let cell = &self.memos[self.memo_index(id)];
        if cell.disposed {
            return Err(crate::view::ReadFault::Disposed);
        }
        if cell.computing {
            return Err(crate::view::ReadFault::GraphCycle);
        }
        if cell.poisoned.is_some() {
            // 毒化后不重试 compute：只有依赖变化能清除 poison。仍有最后值时
            // 返回它，界面因此保持最后一次已提交状态。
            if let Some(value) = &cell.value {
                return Ok(Some(
                    (*value.clone().downcast::<T>().expect("memo value type")).clone(),
                ));
            }
            return Err(crate::view::ReadFault::ComputedPanicked);
        }
        if cell.dirty {
            return Ok(None);
        }
        // 失效只标直接消费者，因此一个仍 clean 的下游可能藏着已脏的上游。
        // 读取时沿 memo 边确认上游已结算；flush 的 worklist 已结算的路径不会
        // 触发重算，只在处理期直接读取或结构挂载绕过了 worklist 时才补救。
        if self.memo_deps_unsettled(id) {
            let index = self.memo_index(id);
            self.memos[index].dirty = true;
            return Ok(None);
        }
        Ok(cell
            .value
            .as_ref()
            .map(|value| (*value.clone().downcast::<T>().expect("memo value type")).clone()))
    }

    /// 该 memo 的 memo 依赖（传递闭包）里是否还有未结算的节点。
    ///
    /// 失效只标直接消费者；flush 的 worklist 会把变化逐级传播。事件阶段读取
    /// 一个仍 clean 的下游 memo 时，必须沿订阅边确认上游已经结算，否则会拿到
    /// 旧缓存。这里只遍历 memo 边，源（Signal/Field/Rows）的变化已经直接标脏。
    fn memo_deps_unsettled(&self, id: u32) -> bool {
        let mut stack: Vec<u32> = self.memos[self.memo_index(id)]
            .deps
            .iter()
            .filter_map(|key| match key {
                DepKey::Memo(dep) => Some(*dep),
                _ => None,
            })
            .collect();
        let mut visited = BTreeSet::new();
        while let Some(memo) = stack.pop() {
            if !visited.insert(memo) {
                continue;
            }
            let cell = &self.memos[self.memo_index(memo)];
            if cell.disposed {
                return true;
            }
            if cell.poisoned.is_some() {
                // 毒化 memo 不重试 compute，并保留最后值；已计算的下游与它
                // 一致，直到依赖变化清除 poison。没有最后值则下游无法安全读取。
                if cell.value.is_none() {
                    return true;
                }
                continue;
            }
            if cell.dirty {
                return true;
            }
            stack.extend(cell.deps.iter().filter_map(|key| match key {
                DepKey::Memo(dep) => Some(*dep),
                _ => None,
            }));
        }
        false
    }

    /// flush 前的 memo worklist：反复结算脏 memo，直到没有可重算的节点。
    ///
    /// 毒化且依赖未变的 memo 跳过；依赖变化会先清除 poison 再标脏。
    pub(crate) fn next_dirty_live_memo(&self, live: &BTreeSet<u32>) -> Option<u32> {
        live.iter().copied().find(|id| {
            let cell = &self.memos[self.memo_index(*id)];
            cell.dirty && !cell.disposed && !cell.computing && cell.poisoned.is_none()
        })
    }

    pub(crate) fn memo_value<T: ReactiveValue>(&mut self, id: u32) -> T {
        let cell = &self.memos[self.memo_index(id)];
        let value = cell.value.as_ref().expect("memo was not computed").clone();
        (*value.downcast::<T>().expect("memo value type")).clone()
    }

    pub(crate) fn begin_memo(&mut self, id: u32) -> (Rc<dyn MemoCompute>, Phase) {
        let index = self.memo_index(id);
        let cell = &mut self.memos[index];
        assert!(!cell.disposed, "memo belongs to a disposed owner");
        assert!(!cell.computing, "memo {id} is already computing");
        let compute = cell
            .compute
            .as_ref()
            .expect("memo compute closure is present")
            .clone();
        cell.computing = true;
        let previous = self.phase;
        self.phase = Phase::Memo;
        self.computing.push((id, Vec::new()));
        (compute, previous)
    }

    /// 本轮依赖集整体替换旧依赖；只有值变化才使下游失效。
    /// 依赖边用于定向标脏，避免无关节点重新求值。
    pub(crate) fn finish_memo(
        &mut self,
        id: u32,
        value: Box<dyn Any>,
        previous_phase: Phase,
        compute: Rc<dyn MemoCompute>,
    ) {
        self.phase = previous_phase;
        let (_, deps) = self.computing.pop().expect("memo compute stack");
        let index = self.memo_index(id);
        let cell = &mut self.memos[index];
        cell.computing = false;
        cell.dirty = false;
        // 相等比较由 compute 的类型擦除适配器完成；缓存只保存 Any。
        let changed = match &cell.value {
            Some(current) => !compute.eq(&**current, &*value),
            None => true,
        };
        cell.compute = Some(compute);
        cell.poisoned = None;
        if changed {
            cell.revision = cell.revision.wrapping_add(1);
        }
        cell.value = Some(Rc::from(value));

        for key in &cell.deps {
            if let Some(dependents) = self.reverse.get_mut(key) {
                dependents.remove(&id);
            }
        }
        for key in &deps {
            self.reverse.entry(*key).or_default().insert(id);
        }
        cell.deps = deps;
        if changed {
            self.invalidate_dependents(DepKey::Memo(id));
        }
    }

    /// compute panic 后恢复求值栈与 phase，并把 memo 标为毒化。
    ///
    /// 类型擦除的计算闭包始终保留在 `MemoCell`；最后已提交值留在 `value`
    /// 中，[`Memo::get`] 在毒化期间返回它，依赖变化后再重新求值。
    pub(crate) fn abort_memo(&mut self, id: u32, panic: PanicInfo, previous_phase: Phase) {
        self.phase = previous_phase;
        if matches!(self.computing.last(), Some((top, _)) if *top == id) {
            self.computing.pop();
        } else {
            // 防御性恢复：嵌套 panic 可能已经弹出更深的栈项；不要留下该 memo。
            self.computing.retain(|(memo, _)| *memo != id);
        }
        let index = self.memo_index(id);
        let cell = &mut self.memos[index];
        cell.computing = false;
        cell.dirty = true;
        cell.poisoned = Some(panic);
    }

    pub(crate) fn note_dep(&mut self, key: DepKey) {
        if let Some((_, deps)) = self.computing.last_mut() {
            deps.push(key);
        }
    }

    pub(crate) fn bump(&mut self, key: DepKey) {
        let epoch = self.write_epochs.entry(key).or_insert(0);
        *epoch = epoch.wrapping_add(1);
        self.invalidate_dependents(key);
    }

    /// 字段写入尝试代数：每次应用写入都推进，相等写入也算；
    /// 与脏依赖代数分开，避免相等写入让依赖失效。
    pub(crate) fn field_write_epoch(
        &self,
        id: crate::store::TodoId,
        kind: crate::context::FieldKind,
    ) -> u64 {
        self.field_writes.get(&(id, kind)).copied().unwrap_or(0)
    }

    /// 记录一次应用字段写入尝试；由 `Store` 事务在提交时调用。
    pub(crate) fn note_field_write(
        &mut self,
        id: crate::store::TodoId,
        kind: crate::context::FieldKind,
    ) {
        let epoch = self.field_writes.entry((id, kind)).or_insert(0);
        *epoch = epoch.wrapping_add(1);
    }

    /// 绑定槽来源的写入代数；已释放或不存在的槽按 0 处理。
    pub(crate) fn source_write_epoch(&self, source: crate::node::SourceRef) -> u64 {
        self.signals
            .get(source.id as usize)
            .map_or(0, |cell| cell.writes)
    }

    /// 绑定槽来源 memo 的已提交 revision；相等重算不推进。
    pub(crate) fn source_memo_revision(&self, source: crate::node::SourceRef) -> u64 {
        self.memos
            .get(source.id as usize)
            .map_or(0, |cell| cell.revision)
    }

    fn invalidate_dependents(&mut self, key: DepKey) {
        let dependents: BTreeSet<u32> = self.reverse.get(&key).cloned().unwrap_or_default();
        for memo in dependents {
            let index = self.memo_index(memo);
            if self.memos[index].disposed || self.memos[index].computing {
                continue;
            }
            // 只标直接消费者。它们重算后若值变化，再自行传播；相等重算因此
            // 不会把下游 memo/binding 拉进 worklist。
            //
            // 依赖变化也是毒化的唯一解药：清除 poison 并允许下一次结算重算。
            self.memos[index].poisoned = None;
            self.memos[index].dirty = true;
        }
    }

    /// 记录一次绑定读取失败；同一刷新内相同 fault 只报告一次。
    pub(crate) fn record_read_fault(&mut self, fault: crate::view::ReadFault) {
        if !self.read_faults.contains(&fault) {
            self.read_faults.push(fault);
        }
    }

    pub(crate) fn record_fault(&mut self, operation: &'static str, phase: Phase) {
        if self.fault.is_none() {
            self.fault = Some(crate::context::ContextFault { operation, phase });
        }
    }
}
