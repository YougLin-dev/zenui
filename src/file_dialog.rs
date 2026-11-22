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

/// 文件对话框的用途：打开文件、保存文件或选择文件夹。
///
/// 打开与保存使用标题和过滤器；保存还使用建议文件名与默认扩展名；选择
/// 文件夹只使用标题。保存与选择文件夹都只返回一个路径。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileDialogKind {
    /// 打开单个文件。
    OpenFile,
    /// 打开多个文件。
    OpenFiles,
    /// 保存文件；只返回一个路径。
    SaveFile,
    /// 选择文件夹；只返回一个路径。
    PickFolder,
}

/// 应用交给宿主文件对话框的只读选项；框架不预设标题、过滤器、建议文件名或默认扩展名。
///
/// 字段按 [`FileDialogKind`] 生效：打开与保存使用标题和过滤器；保存还使用
/// 建议文件名与默认扩展名；选择文件夹只使用标题，其余字段被忽略。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDialogOptions {
    kind: FileDialogKind,
    title: String,
    filters: Vec<FileFilter>,
    suggested_name: String,
    default_extension: String,
}

impl FileDialogOptions {
    #[must_use]
    pub fn open_file() -> Self {
        Self {
            kind: FileDialogKind::OpenFile,
            title: String::new(),
            filters: Vec::new(),
            suggested_name: String::new(),
            default_extension: String::new(),
        }
    }

    #[must_use]
    pub fn open_files() -> Self {
        Self {
            kind: FileDialogKind::OpenFiles,
            ..Self::open_file()
        }
    }

    /// 保存文件；建议文件名与默认扩展名为空表示不提供。
    #[must_use]
    pub fn save_file() -> Self {
        Self {
            kind: FileDialogKind::SaveFile,
            ..Self::open_file()
        }
    }

    /// 选择文件夹；过滤器、建议文件名与默认扩展名在此 kind 下被忽略。
    #[must_use]
    pub fn pick_folder() -> Self {
        Self {
            kind: FileDialogKind::PickFolder,
            ..Self::open_file()
        }
    }

    #[must_use]
    pub fn titled(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// 扩展名可以带前导点；空扩展名被忽略。选择文件夹时忽略过滤器。
    #[must_use]
    pub fn filter(
        mut self,
        name: impl Into<String>,
        extensions: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        if !Self::uses_filters(self.kind) {
            return self;
        }
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

    /// 设置保存对话框的建议文件名；空表示不提供。其他 kind 忽略该值。
    /// 读取使用 [`Self::suggested_name`]。
    #[must_use]
    pub fn with_suggested_name(mut self, name: impl Into<String>) -> Self {
        if Self::uses_save_fields(self.kind) {
            self.suggested_name = name.into();
        }
        self
    }

    /// 设置保存对话框的默认扩展名；前导点会被去掉，空值表示不提供。
    /// 其他 kind 忽略该值。读取使用 [`Self::default_extension`]。
    #[must_use]
    pub fn with_default_extension(mut self, extension: impl Into<String>) -> Self {
        if Self::uses_save_fields(self.kind) {
            let extension = extension.into();
            self.default_extension = extension.trim_start_matches('.').to_owned();
        }
        self
    }

    pub fn kind(&self) -> FileDialogKind {
        self.kind
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// 打开与保存对话框的过滤器；选择文件夹时为空。
    pub fn filters(&self) -> &[FileFilter] {
        &self.filters
    }

    /// 保存对话框的建议文件名；不适用或未设置时为空。
    pub fn suggested_name(&self) -> &str {
        &self.suggested_name
    }

    /// 保存对话框的默认扩展名（不含前导点）；不适用或未设置时为空。
    pub fn default_extension(&self) -> &str {
        &self.default_extension
    }

    /// 只有 [`FileDialogKind::OpenFiles`] 允许返回多个路径。
    pub const fn allows_multiple(&self) -> bool {
        matches!(self.kind, FileDialogKind::OpenFiles)
    }

    const fn uses_filters(kind: FileDialogKind) -> bool {
        matches!(
            kind,
            FileDialogKind::OpenFile | FileDialogKind::OpenFiles | FileDialogKind::SaveFile
        )
    }

    const fn uses_save_fields(kind: FileDialogKind) -> bool {
        matches!(kind, FileDialogKind::SaveFile)
    }
}

impl Default for FileDialogOptions {
    fn default() -> Self {
        Self::open_file()
    }
}

/// 一次文件对话框的结果；用户取消不是错误。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileDialogOutcome {
    Picked(Vec<PathBuf>),
    Cancelled,
    Failed(FileDialogError),
}

/// 宿主无法完成文件对话框的原因；对话框内的普通错误保留平台文本。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileDialogError {
    /// 当前宿主或线程不提供系统文件对话框，或该对话框类型尚未实现。
    Unsupported,
    /// 弹窗时锚点窗口已关闭、隐藏或暂停输入。
    AnchorClosed,
    /// 系统对话框自身失败。
    DialogFailed(String),
}

impl fmt::Display for FileDialogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => {
                f.write_str("the host does not support the requested native file dialog")
            }
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
    /// 请求宿主以本窗口为锚点显示文件对话框。
    ///
    /// 对话框的用途、标题、过滤器、建议文件名与默认扩展名由 `options` 决定；
    /// 哪些字段生效取决于 [`FileDialogKind`]。平台宿主决定是否实现该 kind：
    /// 当前 Windows 宿主只呈现打开文件请求，保存与选择文件夹请求以
    /// [`FileDialogError::Unsupported`] 完成，不会打开替代对话框。
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
    pub fn request_file_dialog(
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

#[cfg(test)]
mod tests {
    use super::*;

    fn configured(options: FileDialogOptions) -> FileDialogOptions {
        options
            .titled("Dialog")
            .filter("Text", [".txt", "md"])
            .with_suggested_name("notes")
            .with_default_extension(".md")
    }

    fn extensions(options: &FileDialogOptions) -> Vec<&str> {
        options.filters()[0]
            .extensions()
            .iter()
            .map(String::as_str)
            .collect()
    }

    #[test]
    fn each_kind_uses_only_its_documented_fields() {
        for options in [
            configured(FileDialogOptions::open_file()),
            configured(FileDialogOptions::open_files()),
        ] {
            assert_eq!(options.title(), "Dialog");
            assert_eq!(extensions(&options), ["txt", "md"]);
            assert_eq!(options.suggested_name(), "");
            assert_eq!(options.default_extension(), "");
        }

        let save = configured(FileDialogOptions::save_file());
        assert_eq!(save.title(), "Dialog");
        assert_eq!(save.filters()[0].name(), "Text");
        assert_eq!(extensions(&save), ["txt", "md"]);
        assert_eq!(save.suggested_name(), "notes");
        assert_eq!(save.default_extension(), "md");

        let folder = configured(FileDialogOptions::pick_folder());
        assert_eq!(folder.title(), "Dialog");
        assert!(folder.filters().is_empty());
        assert_eq!(folder.suggested_name(), "");
        assert_eq!(folder.default_extension(), "");
    }

    #[test]
    fn only_open_files_allows_multiple_paths() {
        assert!(!FileDialogOptions::open_file().allows_multiple());
        assert!(FileDialogOptions::open_files().allows_multiple());
        assert!(!FileDialogOptions::save_file().allows_multiple());
        assert!(!FileDialogOptions::pick_folder().allows_multiple());
    }

    #[test]
    fn default_extension_strips_leading_dots_and_empty_means_absent() {
        assert_eq!(
            FileDialogOptions::save_file()
                .with_default_extension(".txt")
                .default_extension(),
            "txt"
        );
        assert_eq!(
            FileDialogOptions::save_file()
                .with_default_extension("..md")
                .default_extension(),
            "md"
        );
        assert_eq!(
            FileDialogOptions::save_file()
                .with_default_extension(".")
                .default_extension(),
            ""
        );
        assert_eq!(
            FileDialogOptions::save_file()
                .with_suggested_name("")
                .suggested_name(),
            ""
        );
    }
}
