use std::any::Any;
use std::cell::RefCell;
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::JoinHandle;

use crate::context::{Phase, with_state, with_state_of};
use crate::file_dialog::{
    FileDialogError, FileDialogOutcome, FileDialogRequestError, PendingFileDialog,
};
use crate::runtime::{Runtime, State};
use crate::view::MountError;
use crate::window::WindowCtx;

pub(crate) type WakeCallback = Arc<dyn Fn() + Send + Sync>;

/// 任务由 worker 线程完成，还是由宿主服务完成。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TaskKind {
    Worker,
    FileDialog,
}

/// `App` 拥有的工作可以跨窗口关闭继续提交模型；
/// `Window` 拥有的工作在 activation 关闭时取消交付。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TaskOwner {
    App,
    Window(WindowCtx),
}

impl TaskOwner {
    /// 交付期间创建的 UI 资源归属其任务所有者，而不是上一次命令的 Owner。
    const fn owner_id(self) -> u32 {
        match self {
            Self::App => 0,
            Self::Window(window) => window.owner,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TaskScope {
    pub(crate) runtime: u32,
    owner: TaskOwner,
    _not_send: PhantomData<*const ()>,
}

impl TaskScope {
    pub(crate) const fn app(runtime: u32) -> Self {
        Self {
            runtime,
            owner: TaskOwner::App,
            _not_send: PhantomData,
        }
    }

    pub(crate) const fn for_window(runtime: u32, window: WindowCtx) -> Self {
        Self {
            runtime,
            owner: TaskOwner::Window(window),
            _not_send: PhantomData,
        }
    }

    pub(crate) const fn task_owner(self) -> TaskOwner {
        self.owner
    }

    /// worker 只接收可 Send 的 job；UI 句柄不满足 `Send`，在编译期被拒绝。
    ///
    /// ```compile_fail
    /// use zenui::prelude::*;
    ///
    /// fn capture_ui_handle(scope: TaskScope, draft: Signal<String>) {
    ///     let _ = scope.spawn(move || draft.get());
    /// }
    /// ```
    ///
    /// ```compile_fail
    /// use zenui::prelude::*;
    ///
    /// fn capture_store(scope: TaskScope, todos: Store) {
    ///     let _ = scope.spawn(move || todos.is_empty(Filter::All));
    /// }
    /// ```
    ///
    /// ```compile_fail
    /// use zenui::prelude::*;
    ///
    /// fn capture_window(scope: TaskScope, window: WindowCtx) {
    ///     let _ = scope.spawn(move || window.activation());
    /// }
    /// ```
    ///
    /// ```compile_fail
    /// use zenui::prelude::*;
    ///
    /// fn capture_memo(scope: TaskScope, label: Memo<String>) {
    ///     let _ = scope.spawn(move || label.get());
    /// }
    /// ```
    pub fn spawn<T, F>(self, job: F) -> Result<Task<T>, TaskError>
    where
        T: Send + 'static,
        F: FnOnce(TaskToken) -> T + Send + 'static,
    {
        if crate::context::installed_runtime().is_some_and(|current| current != self.runtime) {
            return Err(TaskError::WrongRuntime);
        }
        let TaskStart { id, sender, token } =
            with_state_of(self.runtime, |state| state.begin_task(self.owner))?;
        let wake_callback = with_state_of(self.runtime, |state| Arc::clone(&state.wake_callback));
        let cancellation = Arc::clone(&token);
        let worker = std::thread::spawn(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| job(TaskToken { cancelled: token })));
            let deliverable = outcome.is_ok();
            let message = match outcome {
                Ok(value) => WorkerMessage::Value(Box::new(value)),
                Err(_) => WorkerMessage::Panicked,
            };
            let sent = sender.send(message).is_ok();
            // 回调在结果入队之后、State 借用之外执行；已观察到取消或无法交付的结果不唤醒。
            if deliverable && sent && !cancellation.load(Ordering::SeqCst) {
                let wake = wake_callback.read().ok().and_then(|slot| slot.clone());
                if let Some(wake) = wake {
                    let _ = catch_unwind(AssertUnwindSafe(move || wake()));
                }
            }
        });
        with_state_of(self.runtime, |state| state.attach_worker(id, worker));
        Ok(Task {
            runtime: self.runtime,
            id,
            _marker: PhantomData,
            _not_send: PhantomData,
        })
    }
}

#[derive(Clone, Debug)]
pub struct TaskToken {
    cancelled: Arc<AtomicBool>,
}

impl TaskToken {
    /// 窗口关闭后，窗口所有权的工作会看到取消标志；worker 可据此提前结束。
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskStatus {
    Running,
    /// worker 已返回或宿主服务已给出结果，值等待 continuation 或交付检查。
    Completed,
    /// continuation 已在 delivery command turn 中执行。
    Delivered,
    /// 所属窗口在交付前关闭，或启动它的失败窗口 setup 被撤销。
    Cancelled,
    /// 完成结果在交付时因目标失效或任务已取消而被丢弃。
    Stale,
    /// worker 发生 panic；结果不会进入交付。
    Panicked,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TaskError {
    WrongRuntime,
    /// 目标窗口 activation 已关闭；拒绝而不是空成功。
    TargetClosed,
    AlreadySettled,
    Context {
        operation: &'static str,
        phase: Phase,
    },
}

impl fmt::Display for TaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongRuntime => {
                f.write_str("a task handle from another ZenUI Runtime cannot access this one")
            }
            Self::TargetClosed => f.write_str("the task target is no longer live"),
            Self::AlreadySettled => f.write_str("the task already has a continuation or delivery"),
            Self::Context { operation, phase } => {
                write!(f, "{operation} is not allowed during {phase:?}")
            }
        }
    }
}

impl Error for TaskError {}

/// 一次 `drive_tasks` 批次的交付报告。
///
/// 每个批次是一个 Delivery turn：后续 step 在首个失败处停止，已提交的写入
/// 在批次末统一 flush。continuation 失败、上下文 fault、写入错误与批次末
/// flush 错误都保留在 `errors` 中，调用方读取报告而不是把它当成会话级错误。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeliveryReport {
    pub delivered: usize,
    pub discarded: usize,
    errors: Vec<MountError>,
}

impl DeliveryReport {
    /// 本批次遇到的错误，按发生顺序。
    pub fn errors(&self) -> &[MountError] {
        &self.errors
    }

    /// 本批次是否没有错误。
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    fn push_error(&mut self, error: MountError) {
        self.errors.push(error);
    }

    fn absorb(&mut self, mut other: Self) {
        self.delivered += other.delivered;
        self.discarded += other.discarded;
        self.errors.append(&mut other.errors);
    }
}

pub struct Task<T> {
    runtime: u32,
    id: u32,
    _marker: PhantomData<fn() -> T>,
    _not_send: PhantomData<*const ()>,
}

impl<T> Copy for Task<T> {}
impl<T> Clone for Task<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Task<T> {
    /// 宿主任务由运行时创建句柄；字段保持私有，调用方只能通过公开入口取得。
    pub(crate) const fn from_id(runtime: u32, id: u32) -> Self {
        Self {
            runtime,
            id,
            _marker: PhantomData,
            _not_send: PhantomData,
        }
    }
}

impl<T> fmt::Debug for Task<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Task")
            .field("runtime", &self.runtime)
            .field("id", &self.id)
            .finish()
    }
}

impl<T: Send + 'static> Task<T> {
    /// continuation 在 UI 线程的 delivery command turn 中运行；
    /// 它可以提交模型或写仍存活的 UI Signal，但不具有 worker 侧的类型入口。
    pub fn on_done<F>(self, f: F) -> Result<(), TaskError>
    where
        F: FnOnce(T) -> Result<(), MountError> + 'static,
    {
        if crate::context::installed_runtime().is_some_and(|current| current != self.runtime) {
            return Err(TaskError::WrongRuntime);
        }
        let continuation: Continuation = Box::new(move |value: Box<dyn Any + Send>| {
            let value: T = *value
                .downcast::<T>()
                .map_err(|_| MountError::Delivery("task result has a different type"))?;
            f(value)
        });
        with_state_of(self.runtime, |state| {
            state.register_continuation(self.id, continuation)
        })
    }
}

pub(crate) type Continuation = Box<dyn FnOnce(Box<dyn Any + Send>) -> Result<(), MountError>>;

pub(crate) enum WorkerMessage {
    Value(Box<dyn Any + Send>),
    Panicked,
}

pub(crate) struct TaskStart {
    id: u32,
    sender: Sender<WorkerMessage>,
    token: Arc<AtomicBool>,
}

pub(crate) struct TaskRecord {
    owner: TaskOwner,
    /// 宿主服务与 worker 使用同一交付机制，但完成入口不同。
    kind: TaskKind,
    /// 启动位置的 Owner；失败的挂载会取消在该 setup 子树中创建的任务。
    origin: Option<u32>,
    status: TaskStatus,
    token: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    receiver: Option<Receiver<WorkerMessage>>,
    completion: Option<Box<dyn Any + Send>>,
    continuation: Option<Continuation>,
}

impl TaskRecord {
    fn accept(&mut self, message: WorkerMessage) {
        match message {
            WorkerMessage::Value(value) => match self.status {
                TaskStatus::Running => {
                    self.completion = Some(value);
                    self.status = TaskStatus::Completed;
                }
                TaskStatus::Cancelled => {
                    self.completion = Some(value);
                }
                _ => {}
            },
            WorkerMessage::Panicked => {
                self.status = TaskStatus::Panicked;
                self.completion = None;
                self.continuation = None;
            }
        }
    }
}

pub(crate) enum DeliveryStep {
    Discard,
    Run {
        id: u32,
        owner: u32,
        value: Box<dyn Any + Send>,
        continuation: Continuation,
    },
}

impl State {
    pub(crate) fn begin_task(&mut self, owner: TaskOwner) -> Result<TaskStart, TaskError> {
        if matches!(self.phase, Phase::Binding | Phase::Memo) {
            return Err(TaskError::Context {
                operation: "TaskScope::spawn",
                phase: self.phase,
            });
        }
        if !self.task_target_live(owner) {
            return Err(TaskError::TargetClosed);
        }
        let id = self.next_task;
        self.next_task += 1;
        let (sender, receiver) = channel();
        let token = Arc::new(AtomicBool::new(false));
        self.tasks.insert(
            id,
            TaskRecord {
                owner,
                kind: TaskKind::Worker,
                origin: self.current_owner,
                status: TaskStatus::Running,
                token: Arc::clone(&token),
                worker: None,
                receiver: Some(receiver),
                completion: None,
                continuation: None,
            },
        );
        Ok(TaskStart { id, sender, token })
    }

    /// 创建一个由宿主服务完成的任务；没有 worker channel，因此不会被当成
    /// 已断开的 worker 标记为 Panicked。
    pub(crate) fn begin_host_task(
        &mut self,
        owner: TaskOwner,
        kind: TaskKind,
    ) -> Result<u32, TaskError> {
        if matches!(self.phase, Phase::Binding | Phase::Memo) {
            return Err(TaskError::Context {
                operation: "host task",
                phase: self.phase,
            });
        }
        if !self.task_target_live(owner) {
            return Err(TaskError::TargetClosed);
        }
        let id = self.next_task;
        self.next_task += 1;
        self.tasks.insert(
            id,
            TaskRecord {
                owner,
                kind,
                origin: self.current_owner,
                status: TaskStatus::Running,
                token: Arc::new(AtomicBool::new(false)),
                worker: None,
                receiver: None,
                completion: None,
                continuation: None,
            },
        );
        Ok(id)
    }

    /// 宿主结果只写入完成值；continuation 仍由 `drive_tasks` 在 Delivery turn
    /// 中执行，且不会覆盖已有状态。
    pub(crate) fn complete_host_task(
        &mut self,
        id: u32,
        kind: TaskKind,
        value: Box<dyn Any + Send>,
    ) -> Result<(), FileDialogRequestError> {
        let Some(record) = self.tasks.get_mut(&id) else {
            return Err(FileDialogRequestError::Stale);
        };
        if record.kind != kind {
            return Err(FileDialogRequestError::Stale);
        }
        match record.status {
            TaskStatus::Running if record.completion.is_none() => {
                record.completion = Some(value);
                record.status = TaskStatus::Completed;
                Ok(())
            }
            TaskStatus::Cancelled | TaskStatus::Stale => Err(FileDialogRequestError::Stale),
            _ => Err(FileDialogRequestError::AlreadySettled),
        }
    }

    /// 锚点已经不可用时结算一个排队请求：窗口所有权取消，应用所有权把
    /// `AnchorClosed` 作为正常结果交付，避免 continuation 永远等待。
    pub(crate) fn settle_orphaned_file_dialog(&mut self, pending: PendingFileDialog) {
        let Some(record) = self.tasks.get_mut(&pending.task) else {
            return;
        };
        match record.owner {
            TaskOwner::App => {
                if record.status == TaskStatus::Running && record.completion.is_none() {
                    record.completion = Some(Box::new(FileDialogOutcome::Failed(
                        FileDialogError::AnchorClosed,
                    )));
                    record.status = TaskStatus::Completed;
                }
            }
            TaskOwner::Window(_) => {
                if record.status == TaskStatus::Running {
                    record.status = TaskStatus::Cancelled;
                    record.token.store(true, Ordering::SeqCst);
                    record.continuation = None;
                    record.completion = None;
                }
            }
        }
    }

    pub(crate) fn attach_worker(&mut self, id: u32, worker: JoinHandle<()>) {
        if let Some(record) = self.tasks.get_mut(&id) {
            record.worker = Some(worker);
        }
    }

    pub(crate) fn register_continuation(
        &mut self,
        id: u32,
        continuation: Continuation,
    ) -> Result<(), TaskError> {
        let Some((owner, status, registered)) = self
            .tasks
            .get(&id)
            .map(|record| (record.owner, record.status, record.continuation.is_some()))
        else {
            return Err(TaskError::AlreadySettled);
        };
        if !self.task_target_live(owner) {
            return Err(TaskError::TargetClosed);
        }
        if registered || !matches!(status, TaskStatus::Running | TaskStatus::Completed) {
            return Err(TaskError::AlreadySettled);
        }
        self.tasks
            .get_mut(&id)
            .expect("task is registered")
            .continuation = Some(continuation);
        Ok(())
    }

    /// 失败的窗口计划会取消在它 setup 子树中启动的所有任务，包括 app 所有权任务。
    pub(crate) fn cancel_tasks_started_under(&mut self, ancestor: u32) {
        let ids: Vec<u32> = self.tasks.keys().copied().collect();
        for id in ids {
            let cancel = self.tasks.get(&id).is_some_and(|record| {
                matches!(record.status, TaskStatus::Running | TaskStatus::Completed)
                    && record
                        .origin
                        .is_some_and(|origin| self.owner_is_within(origin, ancestor))
            });
            if cancel {
                let record = self.tasks.get_mut(&id).expect("task is registered");
                record.status = TaskStatus::Cancelled;
                record.token.store(true, Ordering::SeqCst);
                record.continuation = None;
            }
        }
        let pending = std::mem::take(&mut self.pending_file_dialogs);
        let tasks = &self.tasks;
        self.pending_file_dialogs = pending
            .into_iter()
            .filter(|request| {
                tasks
                    .get(&request.task)
                    .is_some_and(|record| record.status == TaskStatus::Running)
            })
            .collect();
    }

    pub(crate) fn cancel_window_tasks(&mut self, key: &'static str, activation: u32) {
        for record in self.tasks.values_mut() {
            if !matches!(record.status, TaskStatus::Running | TaskStatus::Completed) {
                continue;
            }
            if let TaskOwner::Window(window) = record.owner
                && window.key == key
                && window.activation == activation
            {
                record.status = TaskStatus::Cancelled;
                record.token.store(true, Ordering::SeqCst);
                record.continuation = None;
            }
        }
        let pending = std::mem::take(&mut self.pending_file_dialogs);
        let mut kept = Vec::with_capacity(pending.len());
        for request in pending {
            if request.anchor.key == key && request.anchor.activation == activation {
                self.settle_orphaned_file_dialog(request);
            } else {
                kept.push(request);
            }
        }
        self.pending_file_dialogs = kept;
    }

    fn task_target_live(&self, owner: TaskOwner) -> bool {
        match owner {
            TaskOwner::App => true,
            TaskOwner::Window(window) => {
                self.window_is_live(window.key, window.activation, window.owner)
            }
        }
    }

    /// 每次只取一个可执行的交付，避免在 State 借用内调用 continuation。
    pub(crate) fn next_delivery(&mut self) -> Option<DeliveryStep> {
        let candidates: Vec<u32> = self
            .tasks
            .iter()
            .filter(|(_, record)| record.completion.is_some())
            .map(|(id, _)| *id)
            .collect();
        for id in candidates {
            let (owner, status, registered) = {
                let record = self.tasks.get(&id)?;
                (record.owner, record.status, record.continuation.is_some())
            };
            if status == TaskStatus::Panicked {
                continue;
            }
            if status == TaskStatus::Cancelled || !self.task_target_live(owner) {
                let record = self.tasks.get_mut(&id)?;
                record.completion = None;
                record.continuation = None;
                record.status = TaskStatus::Stale;
                return Some(DeliveryStep::Discard);
            }
            if registered {
                let owner = owner.owner_id();
                let record = self.tasks.get_mut(&id)?;
                let value = record.completion.take()?;
                let continuation = record.continuation.take()?;
                return Some(DeliveryStep::Run {
                    id,
                    owner,
                    value,
                    continuation,
                });
            }
        }
        None
    }

    pub(crate) fn mark_delivered(&mut self, id: u32) {
        if let Some(record) = self.tasks.get_mut(&id) {
            record.status = TaskStatus::Delivered;
            record.completion = None;
            record.continuation = None;
            record.receiver = None;
        }
    }

    pub(crate) fn mark_panicked(&mut self, id: u32) {
        if let Some(record) = self.tasks.get_mut(&id) {
            record.status = TaskStatus::Panicked;
            record.completion = None;
            record.continuation = None;
            record.receiver = None;
        }
    }

    /// 非阻塞地接收已经返回的 worker 结果，供真实事件循环轮询。
    pub(crate) fn accept_ready_tasks(&mut self) {
        for record in self.tasks.values_mut() {
            if record.completion.is_some() || record.receiver.is_none() {
                continue;
            }
            let outcome = record.receiver.as_ref().expect("checked").try_recv();
            match outcome {
                Ok(message) => {
                    record.receiver = None;
                    record.accept(message);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    record.receiver = None;
                    record.status = TaskStatus::Panicked;
                }
            }
        }
    }
}

/// continuation 可能 panic；恢复执行上下文不能依赖调用方正常返回。
struct DeliveryContext {
    state: Rc<RefCell<State>>,
    owner: Option<u32>,
    read_only: u32,
    phase: Phase,
}

impl Drop for DeliveryContext {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.try_borrow_mut() {
            state.current_owner = self.owner;
            state.read_only = self.read_only;
            state.phase = self.phase;
        }
    }
}

impl Runtime {
    pub fn task_scope(&self) -> TaskScope {
        TaskScope::app(self.state.borrow().id)
    }

    /// 设置 worker 结果就绪时的唤醒回调。
    ///
    /// 回调在结果发送到交付通道之后由 worker 线程调用，因此必须线程安全，且不能
    /// 借用或修改运行时状态；它只通知宿主调用 `drive_tasks`，不交付结果。
    /// 运行时不做合并：每个已发送结果且未观察到取消的任务唤醒一次，worker panic
    /// 不唤醒。回调 panic 会被捕获，已发送的结果仍可交付。替换回调会影响之后
    /// 完成的任务。
    pub fn set_wake_callback(&mut self, callback: impl Fn() + Send + Sync + 'static) {
        let slot = Arc::clone(&self.state.borrow().wake_callback);
        let mut slot = slot
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *slot = Some(Arc::new(callback));
    }

    pub fn task_status<T>(&self, task: Task<T>) -> Option<TaskStatus> {
        let state = self.state.borrow();
        if task.runtime != state.id {
            return None;
        }
        state.tasks.get(&task.id).map(|record| record.status)
    }

    /// headless 驱动入口：没有平台事件循环时可直接创建 app 所有权任务。
    pub fn spawn_task<T, F>(&mut self, scope: TaskScope, job: F) -> Result<Task<T>, TaskError>
    where
        T: Send + 'static,
        F: FnOnce(TaskToken) -> T + Send + 'static,
    {
        let state = self.state.clone();
        if scope.runtime != state.borrow().id {
            return Err(TaskError::WrongRuntime);
        }
        crate::context::install(&state, || scope.spawn(job))
    }

    pub fn task_done<T, F>(&mut self, task: Task<T>, f: F) -> Result<(), TaskError>
    where
        T: Send + 'static,
        F: FnOnce(T) -> Result<(), MountError> + 'static,
    {
        let state = self.state.clone();
        if task.runtime != state.borrow().id {
            return Err(TaskError::WrongRuntime);
        }
        crate::context::install(&state, || task.on_done(f))
    }

    /// 等待所有 worker 返回，包括结果发送之后的唤醒步骤；窗口已取消的任务也会
    /// 等到其完成，以便确认结果被丢弃。宿主任务（例如文件选择器）没有 worker
    /// channel，不在这里等待，由对应后端通过完成入口给出结果。
    pub fn wait_for_tasks(&mut self) {
        loop {
            let next = {
                let mut state = self.state.borrow_mut();
                let id = state
                    .tasks
                    .iter()
                    .find(|(_, record)| record.receiver.is_some() && record.completion.is_none())
                    .map(|(id, _)| *id);
                let Some(id) = id else {
                    break;
                };
                let receiver = state
                    .tasks
                    .get_mut(&id)
                    .and_then(|record| record.receiver.take());
                match receiver {
                    Some(receiver) => (id, receiver),
                    None => continue,
                }
            };
            let message = next.1.recv();
            let worker = {
                let mut state = self.state.borrow_mut();
                let Some(record) = state.tasks.get_mut(&next.0) else {
                    continue;
                };
                match message {
                    Ok(message) => record.accept(message),
                    Err(_) => record.status = TaskStatus::Panicked,
                }
                record.worker.take()
            };
            if let Some(worker) = worker {
                let _ = worker.join();
            }
        }
    }

    /// 在当前线程交付已完成的结果；旧 activation 的结果在交付前被丢弃。
    ///
    /// 所有已就绪的 continuation 按 FIFO 在同一个 Delivery turn 中运行；首个
    /// 失败停止后续 step，已写入的状态仍在批次末 flush 一次。错误随报告返回，
    /// 因此调用方可以继续运行运行时。
    pub fn drive_tasks(&mut self) -> Result<DeliveryReport, MountError> {
        let state = self.state.clone();
        crate::context::install(&state, || {
            with_state(State::begin_turn);
            with_state(State::accept_ready_tasks);
            let mut report = DeliveryReport::default();
            let mut ran_step = false;
            let mut failed = false;
            while !failed {
                let Some(step) = with_state(State::next_delivery) else {
                    break;
                };
                match step {
                    DeliveryStep::Discard => {
                        report.discarded += 1;
                    }
                    DeliveryStep::Run {
                        id,
                        owner,
                        value,
                        continuation,
                    } => {
                        ran_step = true;
                        let previous = with_state(|state| {
                            let previous = (state.current_owner, state.read_only, state.phase);
                            state.current_owner = Some(owner);
                            state.phase = Phase::Delivery;
                            state.fault = None;
                            state.write_error = None;
                            previous
                        });
                        let context = DeliveryContext {
                            state: Rc::clone(&state),
                            owner: previous.0,
                            read_only: previous.1,
                            phase: previous.2,
                        };
                        let outcome = zenui_core::catch_panic(|| continuation(value));
                        let (fault, write_error) =
                            with_state(|state| (state.fault.take(), state.write_error.take()));
                        drop(context);
                        match outcome {
                            Ok(result) => {
                                with_state(|state| state.mark_delivered(id));
                                report.delivered += 1;
                                if let Err(error) = result {
                                    report.push_error(error);
                                    failed = true;
                                }
                            }
                            Err(panic) => {
                                with_state(|state| state.mark_panicked(id));
                                report.push_error(MountError::Panic(panic));
                                failed = true;
                            }
                        }
                        if let Some(fault) = fault {
                            report.push_error(MountError::Context {
                                operation: fault.operation,
                                phase: fault.phase,
                            });
                            failed = true;
                        }
                        if let Some(error) = write_error {
                            report.push_error(MountError::Write(error));
                            failed = true;
                        }
                    }
                }
            }
            // step 之间的结构、绑定与焦点结算必须留到批次末；一次批次只 flush
            // 一次，后续 continuation 看不到中间已提交结构。
            if ran_step
                && with_state(State::has_tree)
                && let Err(error) = self.refresh_internal()
            {
                report.push_error(error);
            }
            Ok(report)
        })
    }

    /// headless 驱动入口：等待 worker 并交付结果。宿主任务不会让这里阻塞；
    /// 后端完成宿主任务后仍需调用 [`Runtime::drive_tasks`]（或本方法）执行
    /// continuation。
    pub fn settle_tasks(&mut self) -> Result<DeliveryReport, MountError> {
        let mut report = DeliveryReport::default();
        loop {
            self.wait_for_tasks();
            let delivered = self.drive_tasks()?;
            let failed = !delivered.is_ok();
            report.absorb(delivered);
            if failed || !self.has_pending_tasks() {
                break;
            }
        }
        Ok(report)
    }

    fn has_pending_tasks(&self) -> bool {
        self.state
            .borrow()
            .tasks
            .values()
            .any(|record| record.receiver.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_task_accepts_exactly_one_completion() {
        let app = Runtime::new();
        let mut state = app.state.borrow_mut();
        let id = state
            .begin_host_task(TaskOwner::App, TaskKind::FileDialog)
            .expect("begin the host task");
        assert!(
            state
                .complete_host_task(
                    id,
                    TaskKind::FileDialog,
                    Box::new(FileDialogOutcome::Cancelled)
                )
                .is_ok()
        );
        assert_eq!(
            state.complete_host_task(
                id,
                TaskKind::FileDialog,
                Box::new(FileDialogOutcome::Cancelled)
            ),
            Err(FileDialogRequestError::AlreadySettled)
        );
        assert_eq!(state.tasks[&id].status, TaskStatus::Completed);
    }

    #[test]
    fn a_worker_task_cannot_be_completed_as_a_host_task() {
        let app = Runtime::new();
        let mut state = app.state.borrow_mut();
        let worker = state.begin_task(TaskOwner::App).expect("begin the worker");
        assert_eq!(
            state.complete_host_task(
                worker.id,
                TaskKind::FileDialog,
                Box::new(FileDialogOutcome::Cancelled)
            ),
            Err(FileDialogRequestError::Stale)
        );
    }
}
