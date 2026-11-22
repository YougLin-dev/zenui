use std::fmt;
use std::path::PathBuf;

use crate::context::{Phase, installed_runtime, with_state_of};
use crate::runtime::Runtime;
use crate::task::{Task, TaskError, TaskKind, TaskOwner, TaskScope};
use crate::window::{WindowCtx, WindowError};

/// 一个文件类型过滤器；扩展名不含前导点。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileFilter {
    name: String,
    extensions: Vec<String>,
}

impl FileFilter {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn extensions(&self) -> &[String] {
        &self.extensions
    }
}

/// 打开单个文件或多个文件；保存对话框不在本里程碑内。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileDialogKind {
    OpenFile,
    OpenFiles,
}

/// 应用交给宿主文件选择器的只读选项；框架不预设标题、过滤器或扩展名。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDialogOptions {
    kind: FileDialogKind,
    title: String,
    filters: Vec<FileFilter>,
}

impl FileDialogOptions {
    #[must_use]
    pub fn open_file() -> Self {
        Self {
            kind: FileDialogKind::OpenFile,
            title: String::new(),
            filters: Vec::new(),
        }
    }

    #[must_use]
    pub fn open_files() -> Self {
        Self {
            kind: FileDialogKind::OpenFiles,
            ..Self::open_file()
        }
    }

    #[must_use]
    pub fn titled(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// 扩展名可以带前导点；空扩展名被忽略。
    #[must_use]
    pub fn filter(
        mut self,
        name: impl Into<String>,
        extensions: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        let extensions = extensions
            .into_iter()
            .map(Into::into)
            .map(|extension| extension.trim_start_matches('.').to_owned())
            .filter(|extension| !extension.is_empty())
            .collect();
        self.filters.push(FileFilter {
            name: name.into(),
            extensions,
        });
        self
    }

    pub fn kind(&self) -> FileDialogKind {
        self.kind
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn filters(&self) -> &[FileFilter] {
        &self.filters
    }

    pub const fn allows_multiple(&self) -> bool {
        matches!(self.kind, FileDialogKind::OpenFiles)
    }
}

impl Default for FileDialogOptions {
    fn default() -> Self {
        Self::open_file()
    }
}

/// 一次文件选择的结果；用户取消不是错误。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileDialogOutcome {
    Picked(Vec<PathBuf>),
    Cancelled,
    Failed(FileDialogError),
}

/// 宿主无法完成文件选择的原因；对话框内的普通错误保留平台文本。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileDialogError {
    /// 当前宿主或线程不提供系统文件对话框。
    Unsupported,
    /// 弹窗时锚点窗口已关闭、隐藏或暂停输入。
    AnchorClosed,
    /// 系统对话框自身失败。
    DialogFailed(String),
}

impl fmt::Display for FileDialogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("the host does not support a native file dialog"),
            Self::AnchorClosed => f.write_str("the anchor window is no longer available"),
            Self::DialogFailed(message) => write!(f, "the file dialog failed: {message}"),
        }
    }
}

impl std::error::Error for FileDialogError {}

/// 宿主完成一个已经取出的请求时的拒绝原因；请求本身不会重试。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileDialogRequestError {
    WrongRuntime,
    /// 任务已被取消、锚点已失效，或请求不属于这个运行时。
    Stale,
    /// 任务已经有结果或已经被交付。
    AlreadySettled,
}

impl fmt::Display for FileDialogRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongRuntime => {
                f.write_str("a file dialog request from another ZenUI Runtime cannot complete here")
            }
            Self::Stale => f.write_str("the file dialog request is no longer live"),
            Self::AlreadySettled => f.write_str("the file dialog request already has a result"),
        }
    }
}

impl std::error::Error for FileDialogRequestError {}

/// 宿主从 `Runtime::take_file_dialog_requests` 取出的待弹窗请求。
///
/// 不是 `Copy`/`Clone`/`Send`：每个请求只能完成一次，并且只能在创建它的 UI 线程
/// 上完成。只有宿主后端构造它。
pub struct FileDialogRequest {
    pub(crate) runtime: u32,
    pub(crate) task: u32,
    anchor: WindowCtx,
    options: FileDialogOptions,
}

impl FileDialogRequest {
    pub const fn anchor(&self) -> WindowCtx {
        self.anchor
    }

    pub fn options(&self) -> &FileDialogOptions {
        &self.options
    }

    pub const fn allows_multiple(&self) -> bool {
        self.options.allows_multiple()
    }
}

impl fmt::Debug for FileDialogRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileDialogRequest")
            .field("anchor", &self.anchor)
            .field("task", &self.task)
            .field("options", &self.options)
            .finish()
    }
}

/// 运行时内部排队、尚未交给宿主的请求。
pub(crate) struct PendingFileDialog {
    pub(crate) task: u32,
    pub(crate) anchor: WindowCtx,
    pub(crate) options: FileDialogOptions,
}

impl WindowCtx {
    /// 请求宿主以本窗口为锚点显示打开文件对话框。
    ///
    /// 只能从 `Event` 或 `Delivery` 处理器调用；返回的任务按 `scope` 决定生命
    /// 周期。`window.task_scope()` 在窗口关闭时取消交付，`app_task_scope()` 允许
    /// 选择结果在窗口关闭后继续到达应用级 continuation。任务在 `drive_tasks`
    /// 的 `Delivery` turn 中交付 `FileDialogOutcome`。与 worker 任务一样，已交付
    /// 的任务记录当前不从事务表回收。
    ///
    /// # Errors
    ///
    /// 窗口已关闭返回 [`WindowError::Stale`]，隐藏返回 [`WindowError::Hidden`]，
    /// 待决关闭或非法执行阶段返回 [`WindowError::State`]，跨运行时或跨窗口的
    /// scope 返回 [`WindowError::WrongRuntime`] / [`WindowError::State`]。
    pub fn pick_file(
        self,
        options: FileDialogOptions,
        scope: TaskScope,
    ) -> Result<Task<FileDialogOutcome>, WindowError> {
        if installed_runtime() != Some(self.runtime) {
            return Err(WindowError::WrongRuntime);
        }
        with_state_of(self.runtime, |state| {
            if !matches!(state.phase, Phase::Event | Phase::Delivery) {
                return Err(WindowError::State(
                    "file dialogs can only be requested from an event or delivery handler",
                ));
            }
            if !state.window_is_live(self.key, self.activation, self.owner) {
                return Err(WindowError::Stale);
            }
            if !state.window_is_visible(self.key) {
                return Err(WindowError::Hidden);
            }
            if !state.window_accepts_input(self.key) {
                return Err(WindowError::State("window input is suspended"));
            }
            if scope.runtime != self.runtime {
                return Err(WindowError::WrongRuntime);
            }
            if let TaskOwner::Window(anchor) = scope.task_owner()
                && anchor != self
            {
                return Err(WindowError::State(
                    "the file dialog scope belongs to another window",
                ));
            }
            let task = state
                .begin_host_task(scope.task_owner(), TaskKind::FileDialog)
                .map_err(|error| match error {
                    TaskError::TargetClosed => WindowError::Stale,
                    _ => WindowError::State("the file dialog task could not be created"),
                })?;
            state.pending_file_dialogs.push(PendingFileDialog {
                task,
                anchor: self,
                options,
            });
            Ok(Task::from_id(self.runtime, task))
        })
    }
}

impl Runtime {
    /// 取出当前所有可弹窗的文件请求；不可用锚点的请求在这里结算。
    ///
    /// 宿主在应用窗口请求之后调用。返回的请求按入队顺序排列；窗口所有权
    /// 请求的锚点已失效时任务被取消，应用所有权请求以 `AnchorClosed` 完成，
    /// 因此不会留下永远 Running 的任务。
    pub fn take_file_dialog_requests(&mut self) -> Vec<FileDialogRequest> {
        let mut state = self.state.borrow_mut();
        let runtime = state.id;
        let pending = std::mem::take(&mut state.pending_file_dialogs);
        let mut requests = Vec::with_capacity(pending.len());
        for request in pending {
            let anchor = request.anchor;
            let live = state.window_is_live(anchor.key, anchor.activation, anchor.owner)
                && state.window_is_visible(anchor.key)
                && state.window_accepts_input(anchor.key);
            if live {
                requests.push(FileDialogRequest {
                    runtime,
                    task: request.task,
                    anchor,
                    options: request.options,
                });
            } else {
                state.settle_orphaned_file_dialog(request);
            }
        }
        requests
    }

    /// 用宿主结果完成一个已经取出的请求；每个请求恰好一次。
    ///
    /// 只保存结果，不运行 continuation；宿主随后调用 [`Runtime::drive_tasks`]
    /// 在 `Delivery` turn 中交付。请求来自其他运行时、窗口所有权任务的
    /// activation 已关闭，或任务已经有结果时返回 [`FileDialogRequestError`]，
    /// 不会写入任何任务记录。应用所有权任务在锚点关闭后仍可完成。
    ///
    /// # Errors
    ///
    /// 见 [`FileDialogRequestError`]。
    pub fn complete_file_dialog_request(
        &mut self,
        request: FileDialogRequest,
        outcome: FileDialogOutcome,
    ) -> Result<(), FileDialogRequestError> {
        let mut state = self.state.borrow_mut();
        if request.runtime != state.id {
            return Err(FileDialogRequestError::WrongRuntime);
        }
        state.complete_host_task(request.task, TaskKind::FileDialog, Box::new(outcome))
    }
}
