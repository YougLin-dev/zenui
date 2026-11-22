//! Native file dialogs for the Windows host.
//!
//! The host owns the loop-level request drain and the OLE apartment; this
//! module owns the presenter seam and the `IFileOpenDialog` / `IFileSaveDialog`
//! mapping. Tests inject a presenter, so hidden/CI runs never open a real
//! dialog.

use std::ffi::{OsString, c_void};
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FILEOPENDIALOGOPTIONS, FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM,
    FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, FileOpenDialog, FileSaveDialog,
    IFileDialog, IFileOpenDialog, IFileSaveDialog, IShellItem, IShellItemArray, SIGDN_FILESYSPATH,
};
use windows::core::{HRESULT, Interface, PCWSTR};
use zenui::prelude::*;

/// The host presents one request at a time on the UI thread.
///
/// The default presenter is the native shell dialog selected by
/// [`FileDialogOptions::kind`]; tests install a closure with
/// [`crate::Host::set_file_dialog_presenter`]. A presenter must return an
/// outcome and must not retain the anchor after it returns.
pub trait FileDialogPresenter {
    /// Present the requested picker for `anchor` and return the user's choice.
    fn present(&mut self, anchor: HWND, options: &FileDialogOptions) -> FileDialogOutcome;
}

impl<F> FileDialogPresenter for F
where
    F: FnMut(HWND, &FileDialogOptions) -> FileDialogOutcome,
{
    fn present(&mut self, anchor: HWND, options: &FileDialogOptions) -> FileDialogOutcome {
        self(anchor, options)
    }
}

/// `IFileOpenDialog::Show` reports user cancellation as `ERROR_CANCELLED`.
const ERROR_CANCELLED_HRESULT: HRESULT = HRESULT(0x8007_04C7_u32 as i32);

/// The built-in presenter; the Host ensures OLE before calling it.
pub(crate) struct NativeFileDialog;

impl FileDialogPresenter for NativeFileDialog {
    fn present(&mut self, anchor: HWND, options: &FileDialogOptions) -> FileDialogOutcome {
        present_native(anchor, options)
    }
}

struct DialogPlan {
    flags: FILEOPENDIALOGOPTIONS,
    title: Vec<u16>,
    /// Owning buffers for the COMDLG_FILTERSPEC pointers built at call time.
    filter_wide: Vec<Vec<u16>>,
    suggested_name: Vec<u16>,
    default_extension: Vec<u16>,
    type_index: u32,
}

fn plan(options: &FileDialogOptions) -> DialogPlan {
    let flags = match options.kind() {
        FileDialogKind::OpenFile => FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST | FOS_PATHMUSTEXIST,
        FileDialogKind::OpenFiles => {
            FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST | FOS_PATHMUSTEXIST | FOS_ALLOWMULTISELECT
        }
        FileDialogKind::SaveFile => FOS_FORCEFILESYSTEM | FOS_OVERWRITEPROMPT | FOS_PATHMUSTEXIST,
        FileDialogKind::PickFolder => FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST,
    };
    let title = if options.title().is_empty() {
        Vec::new()
    } else {
        wide_null(options.title())
    };
    let mut filter_wide = Vec::with_capacity(options.filters().len() * 2);
    for filter in options.filters() {
        let patterns = filter
            .extensions()
            .iter()
            .map(|extension| format!("*.{extension}"))
            .collect::<Vec<_>>()
            .join(";");
        filter_wide.push(wide_null(filter.name()));
        filter_wide.push(wide_null(&patterns));
    }
    let type_index = if filter_wide.is_empty() { 0 } else { 1 };
    let suggested_name =
        if options.kind() == FileDialogKind::SaveFile && !options.suggested_name().is_empty() {
            wide_null(options.suggested_name())
        } else {
            Vec::new()
        };
    let default_extension =
        if options.kind() == FileDialogKind::SaveFile && !options.default_extension().is_empty() {
            wide_null(options.default_extension())
        } else {
            Vec::new()
        };
    DialogPlan {
        flags,
        title,
        filter_wide,
        suggested_name,
        default_extension,
        type_index,
    }
}

/// Build the shell filter specs at the call site so every pointer targets a
/// buffer that is still owned by the plan for the duration of `SetFileTypes`.
fn filter_specs(plan: &DialogPlan) -> Vec<COMDLG_FILTERSPEC> {
    plan.filter_wide
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[name, spec]| COMDLG_FILTERSPEC {
            pszName: PCWSTR(name.as_ptr()),
            pszSpec: PCWSTR(spec.as_ptr()),
        })
        .collect()
}

fn configure(dialog: &IFileDialog, plan: &DialogPlan) -> windows::core::Result<()> {
    // 安全：SetOptions 只替换本对话框的选项；GetOptions 先读当前集合。
    let current = unsafe { dialog.GetOptions()? };
    unsafe {
        dialog.SetOptions(current | plan.flags)?;
    }
    if !plan.title.is_empty() {
        unsafe {
            dialog.SetTitle(PCWSTR(plan.title.as_ptr()))?;
        }
    }
    if !plan.filter_wide.is_empty() {
        let filters = filter_specs(plan);
        unsafe {
            dialog.SetFileTypes(&filters)?;
            // The shell treats the type index as 1-based.
            dialog.SetFileTypeIndex(plan.type_index)?;
        }
    }
    if !plan.suggested_name.is_empty() {
        unsafe {
            dialog.SetFileName(PCWSTR(plan.suggested_name.as_ptr()))?;
        }
    }
    if !plan.default_extension.is_empty() {
        unsafe {
            dialog.SetDefaultExtension(PCWSTR(plan.default_extension.as_ptr()))?;
        }
    }
    Ok(())
}

enum NativeDialog {
    Open(IFileOpenDialog),
    Save(IFileSaveDialog),
}

impl NativeDialog {
    fn base(&self) -> windows::core::Result<IFileDialog> {
        match self {
            Self::Open(dialog) => dialog.cast(),
            Self::Save(dialog) => dialog.cast(),
        }
    }

    fn open(&self) -> Option<&IFileOpenDialog> {
        match self {
            Self::Open(dialog) => Some(dialog),
            Self::Save(_) => None,
        }
    }
}

fn create_dialog(kind: FileDialogKind) -> windows::core::Result<NativeDialog> {
    match kind {
        FileDialogKind::SaveFile => {
            // 安全：类工厂由 shell 注册；CoCreateInstance 只创建进程内对象。
            let dialog: IFileSaveDialog =
                unsafe { CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER) }?;
            Ok(NativeDialog::Save(dialog))
        }
        FileDialogKind::OpenFile | FileDialogKind::OpenFiles | FileDialogKind::PickFolder => {
            // 安全：类工厂由 shell 注册；CoCreateInstance 只创建进程内对象。
            let dialog: IFileOpenDialog =
                unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }?;
            Ok(NativeDialog::Open(dialog))
        }
    }
}

fn present_native(anchor: HWND, options: &FileDialogOptions) -> FileDialogOutcome {
    let dialog = match create_dialog(options.kind()) {
        Ok(dialog) => dialog,
        Err(error) => {
            return FileDialogOutcome::Failed(FileDialogError::DialogFailed(format!(
                "CoCreateInstance failed: {error}"
            )));
        }
    };
    let base = match dialog.base() {
        Ok(base) => base,
        Err(error) => {
            return FileDialogOutcome::Failed(FileDialogError::DialogFailed(format!(
                "querying the dialog interface failed: {error}"
            )));
        }
    };
    let plan = plan(options);
    if let Err(error) = configure(&base, &plan) {
        return FileDialogOutcome::Failed(FileDialogError::DialogFailed(format!(
            "configuring the dialog failed: {error}"
        )));
    }
    // 安全：anchor 在 Host 中存活；Show 的模态循环由 shell 拥有。
    match unsafe { base.Show(Some(anchor)) } {
        Ok(()) => collect(&dialog, options),
        Err(error) => show_outcome(&error),
    }
}

fn show_outcome(error: &windows::core::Error) -> FileDialogOutcome {
    if error.code() == ERROR_CANCELLED_HRESULT {
        FileDialogOutcome::Cancelled
    } else {
        FileDialogOutcome::Failed(FileDialogError::DialogFailed(format!(
            "showing the dialog failed: {error}"
        )))
    }
}

fn collect(dialog: &NativeDialog, options: &FileDialogOptions) -> FileDialogOutcome {
    let result = if options.allows_multiple() {
        let Some(open) = dialog.open() else {
            return FileDialogOutcome::Failed(FileDialogError::DialogFailed(String::from(
                "the selected dialog does not support multiple results",
            )));
        };
        // 安全：GetResults 返回 shell 拥有的数组；路径在读取时复制。
        unsafe { open.GetResults() }.and_then(|items: IShellItemArray| collect_array(&items))
    } else {
        let base = match dialog.base() {
            Ok(base) => base,
            Err(error) => {
                return FileDialogOutcome::Failed(FileDialogError::DialogFailed(format!(
                    "querying the dialog interface failed: {error}"
                )));
            }
        };
        // 安全：GetResult 返回 shell 拥有的单项。
        unsafe { base.GetResult() }
            .and_then(|item: IShellItem| item_path(&item).map(|path| vec![path]))
    };
    match result {
        Ok(paths) if !paths.is_empty() => FileDialogOutcome::Picked(paths),
        Ok(_) => FileDialogOutcome::Failed(FileDialogError::DialogFailed(String::from(
            "the dialog returned no paths",
        ))),
        Err(error) => FileDialogOutcome::Failed(FileDialogError::DialogFailed(format!(
            "reading the dialog result failed: {error}"
        ))),
    }
}

fn collect_array(items: &IShellItemArray) -> windows::core::Result<Vec<PathBuf>> {
    let count = unsafe { items.GetCount()? };
    let mut paths = Vec::with_capacity(count as usize);
    for index in 0..count {
        let item = unsafe { items.GetItemAt(index)? };
        paths.push(item_path(&item)?);
    }
    Ok(paths)
}

fn item_path(item: &IShellItem) -> windows::core::Result<PathBuf> {
    // 安全：返回的 PWSTR 由 CoTaskMem 分配，下面成对释放。
    let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH)? };
    if name.0.is_null() {
        return Err(windows::core::Error::from(HRESULT(0x8000_4005_u32 as i32)));
    }
    let path = path_from_wide(unsafe { name.as_wide() });
    // 安全：GetDisplayName 分配 CoTaskMem，调用方负责释放。
    unsafe {
        CoTaskMemFree(Some(name.0.cast::<c_void>()));
    }
    Ok(path)
}

fn path_from_wide(wide: &[u16]) -> PathBuf {
    PathBuf::from(OsString::from_wide(wide))
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_strings(plan: &DialogPlan) -> Vec<(String, String)> {
        filter_specs(plan)
            .iter()
            .map(|filter| {
                // 安全：指针指向 plan.filter_wide 中仍存活的 NUL 结尾字符串。
                let name = unsafe { filter.pszName.to_string().expect("filter name") };
                let spec = unsafe { filter.pszSpec.to_string().expect("filter spec") };
                (name, spec)
            })
            .collect()
    }

    fn take_task_string(value: windows::core::PWSTR) -> String {
        if value.0.is_null() {
            return String::new();
        }
        // 安全：GetFileName 返回 CoTaskMem 分配的 NUL 结尾字符串。
        let string = unsafe { value.to_string() }.unwrap_or_default();
        // 安全：与 GetFileName 的 CoTaskMem 分配配对释放。
        unsafe {
            CoTaskMemFree(Some(value.0.cast::<c_void>()));
        }
        string
    }

    #[test]
    fn flags_are_kind_specific_and_never_add_forbidden_bits() {
        let single = plan(&FileDialogOptions::open_file());
        assert!(single.flags.contains(FOS_FORCEFILESYSTEM));
        assert!(single.flags.contains(FOS_FILEMUSTEXIST));
        assert!(single.flags.contains(FOS_PATHMUSTEXIST));
        assert!(!single.flags.contains(FOS_ALLOWMULTISELECT));

        let multi = plan(&FileDialogOptions::open_files());
        assert!(multi.flags.contains(FOS_ALLOWMULTISELECT));

        let save = plan(&FileDialogOptions::save_file());
        assert!(save.flags.contains(FOS_FORCEFILESYSTEM));
        assert!(save.flags.contains(FOS_OVERWRITEPROMPT));
        assert!(save.flags.contains(FOS_PATHMUSTEXIST));
        assert!(!save.flags.contains(FOS_FILEMUSTEXIST));
        assert!(!save.flags.contains(FOS_ALLOWMULTISELECT));
        assert!(!save.flags.contains(FOS_PICKFOLDERS));

        let folder = plan(&FileDialogOptions::pick_folder());
        assert!(folder.flags.contains(FOS_PICKFOLDERS));
        assert!(folder.flags.contains(FOS_FORCEFILESYSTEM));
        assert!(folder.flags.contains(FOS_PATHMUSTEXIST));
        assert!(!folder.flags.contains(FOS_FILEMUSTEXIST));
        assert!(!folder.flags.contains(FOS_ALLOWMULTISELECT));
    }

    #[test]
    fn save_plan_carries_wide_fields_and_one_based_type_index() {
        let options = FileDialogOptions::save_file()
            .titled("Save tasks")
            .filter("Text files", ["txt", "md"])
            .with_suggested_name("tasks.txt")
            .with_default_extension(".txt");
        let plan = plan(&options);
        assert_eq!(plan.title, wide_null("Save tasks"));
        assert_eq!(plan.suggested_name, wide_null("tasks.txt"));
        assert_eq!(plan.default_extension, wide_null("txt"));
        assert_eq!(plan.type_index, 1);
        assert_eq!(
            spec_strings(&plan),
            [(String::from("Text files"), String::from("*.txt;*.md"))]
        );
    }

    #[test]
    fn folder_plan_omits_filters_and_save_fields() {
        let options = FileDialogOptions::pick_folder()
            .titled("Pick a folder")
            .filter("Ignored", ["txt"])
            .with_suggested_name("ignored.txt")
            .with_default_extension("txt");
        let plan = plan(&options);
        assert_eq!(plan.title, wide_null("Pick a folder"));
        assert!(plan.filter_wide.is_empty());
        assert!(plan.suggested_name.is_empty());
        assert!(plan.default_extension.is_empty());
        assert_eq!(plan.type_index, 0);
    }

    #[test]
    fn filters_keep_names_and_wildcard_patterns() {
        let options = FileDialogOptions::open_file()
            .titled("Import tasks")
            .filter("Text files", ["txt", "md"])
            .filter("All files", ["*"]);
        let plan = plan(&options);
        assert_eq!(plan.title, wide_null("Import tasks"));
        assert_eq!(
            spec_strings(&plan),
            [
                (String::from("Text files"), String::from("*.txt;*.md")),
                (String::from("All files"), String::from("*.*")),
            ]
        );
    }

    #[test]
    fn native_save_and_folder_configuration_is_field_specific() {
        let _apartment = crate::ole::OleApartment::initialize().expect("initialize OLE");

        let save_options = FileDialogOptions::save_file()
            .titled("Save tasks")
            .filter("Text files", ["txt"])
            .with_suggested_name("tasks.txt")
            .with_default_extension(".txt");
        let save = create_dialog(save_options.kind()).expect("create the save dialog");
        assert!(
            matches!(save, NativeDialog::Save(_)),
            "save uses the FileSaveDialog class"
        );
        let base = save.base().expect("the save dialog base interface");
        configure(&base, &plan(&save_options)).expect("configure save");
        let flags = unsafe { base.GetOptions() }.expect("save options");
        assert!(flags.contains(FOS_FORCEFILESYSTEM));
        assert!(flags.contains(FOS_OVERWRITEPROMPT));
        assert!(flags.contains(FOS_PATHMUSTEXIST));
        assert_eq!(
            unsafe { base.GetFileTypeIndex() }.expect("save type index"),
            1
        );
        assert_eq!(
            take_task_string(unsafe { base.GetFileName() }.expect("save file name")),
            "tasks.txt"
        );

        let folder_options = FileDialogOptions::pick_folder().titled("Pick a folder");
        let folder = create_dialog(folder_options.kind()).expect("create the folder dialog");
        assert!(
            matches!(folder, NativeDialog::Open(_)),
            "folder uses the FileOpenDialog class"
        );
        let base = folder.base().expect("the folder dialog base interface");
        configure(&base, &plan(&folder_options)).expect("configure folder");
        let flags = unsafe { base.GetOptions() }.expect("folder options");
        assert!(flags.contains(FOS_PICKFOLDERS));
        assert!(flags.contains(FOS_FORCEFILESYSTEM));
        assert!(flags.contains(FOS_PATHMUSTEXIST));
        assert_eq!(
            unsafe { base.GetFileTypeIndex() }.expect("folder type index"),
            0
        );
        let folder_name = unsafe { base.GetFileName() };
        assert!(
            folder_name.is_err()
                || take_task_string(folder_name.expect("folder file name")).is_empty(),
            "a folder dialog has no suggested file name"
        );
    }

    #[test]
    fn show_errors_distinguish_cancellation() {
        let cancelled = windows::core::Error::from(ERROR_CANCELLED_HRESULT);
        assert_eq!(show_outcome(&cancelled), FileDialogOutcome::Cancelled);
        let other = windows::core::Error::from(HRESULT(0x8000_4005_u32 as i32));
        assert!(matches!(
            show_outcome(&other),
            FileDialogOutcome::Failed(FileDialogError::DialogFailed(_))
        ));
    }

    #[test]
    fn wide_paths_preserve_non_bmp_units() {
        let wide: Vec<u16> = "路径/𠀀.txt".encode_utf16().collect();
        assert_eq!(path_from_wide(&wide), PathBuf::from("路径/𠀀.txt"));
    }
}
