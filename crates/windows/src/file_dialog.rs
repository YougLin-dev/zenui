//! Native open-file dialogs for the Windows host.
//!
//! The host owns the loop-level request drain; this module owns the presenter
//! seam, the STA COM guard and the `IFileOpenDialog` mapping. Save and folder
//! requests are rejected by the host before reaching this module; their native
//! mapping is a later change. Tests inject a presenter, so hidden/CI runs never
//! open a real dialog.

use std::ffi::{OsString, c_void};
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use windows::Win32::Foundation::{HWND, RPC_E_CHANGED_MODE};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance,
    CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FILEOPENDIALOGOPTIONS, FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM,
    FOS_PATHMUSTEXIST, FileOpenDialog, IFileOpenDialog, IShellItem, IShellItemArray,
    SIGDN_FILESYSPATH,
};
use windows::core::{HRESULT, PCWSTR};
use zenui::prelude::*;

/// The host presents one request at a time on the UI thread.
///
/// The default presenter is the native `IFileOpenDialog`; tests install a
/// closure with [`crate::Host::set_file_dialog_presenter`]. Only open-file
/// requests reach a presenter: save and folder requests complete
/// `Unsupported` in the Host before this seam. A presenter must return an
/// outcome and must not retain the anchor after it returns.
pub trait FileDialogPresenter {
    /// Present an open-file picker for `anchor` and return the user's choice.
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

/// Lazy STA apartment for the native presenter.
///
/// `S_OK` and `S_FALSE` both require exactly one `CoUninitialize`; a thread
/// already in MTA must not be uninitialized and the dialog reports Unsupported.
pub(crate) struct ComApartment;

impl ComApartment {
    /// Initialize the UI thread as an STA. Hidden and injected presenters never
    /// call this.
    pub(crate) fn initialize() -> Result<Self, FileDialogError> {
        // 安全：pvReserved 必须为空；UI 线程在 Host 存活期间不跨线程。
        let result =
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        if result.is_ok() {
            Ok(Self)
        } else if result == RPC_E_CHANGED_MODE {
            Err(FileDialogError::Unsupported)
        } else {
            Err(FileDialogError::DialogFailed(format!(
                "CoInitializeEx failed with {result:?}"
            )))
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        // 安全：只与成功的 CoInitializeEx 配对；Host 在 UI 线程上释放。
        unsafe {
            CoUninitialize();
        }
    }
}

/// The built-in presenter; the Host ensures COM before calling it.
pub(crate) struct NativeFileDialog;

impl FileDialogPresenter for NativeFileDialog {
    fn present(&mut self, anchor: HWND, options: &FileDialogOptions) -> FileDialogOutcome {
        present_native(anchor, options)
    }
}

struct DialogPlan {
    flags: FILEOPENDIALOGOPTIONS,
    title: Vec<u16>,
    /// Owning buffers for the COMDLG_FILTERSPEC pointers below; only read
    /// through those raw pointers.
    #[expect(
        dead_code,
        reason = "the buffers keep the COMDLG_FILTERSPEC pointers alive"
    )]
    wide: Vec<Vec<u16>>,
    filters: Vec<COMDLG_FILTERSPEC>,
}

fn plan(options: &FileDialogOptions) -> DialogPlan {
    let mut flags = FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST | FOS_PATHMUSTEXIST;
    if options.allows_multiple() {
        flags |= FOS_ALLOWMULTISELECT;
    }
    let title = wide_null(options.title());
    let mut wide = Vec::with_capacity(options.filters().len() * 2);
    for filter in options.filters() {
        let patterns = filter
            .extensions()
            .iter()
            .map(|extension| format!("*.{extension}"))
            .collect::<Vec<_>>()
            .join(";");
        wide.push(wide_null(filter.name()));
        wide.push(wide_null(&patterns));
    }
    // Pointers are built after every buffer is in place; the plan owns the
    // buffers for as long as the pointers are used.
    let filters = wide
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[name, spec]| COMDLG_FILTERSPEC {
            pszName: PCWSTR(name.as_ptr()),
            pszSpec: PCWSTR(spec.as_ptr()),
        })
        .collect();
    DialogPlan {
        flags,
        title,
        wide,
        filters,
    }
}

fn configure(dialog: &IFileOpenDialog, plan: &DialogPlan) -> windows::core::Result<()> {
    // 安全：SetOptions 只替换本对话框的选项；GetOptions 先读当前集合。
    let current = unsafe { dialog.GetOptions()? };
    unsafe {
        dialog.SetOptions(current | plan.flags)?;
    }
    if !plan.filters.is_empty() {
        unsafe {
            dialog.SetFileTypes(&plan.filters)?;
            // The shell treats the type index as 1-based.
            dialog.SetFileTypeIndex(1)?;
        }
    }
    if !plan.title.is_empty() {
        unsafe {
            dialog.SetTitle(PCWSTR(plan.title.as_ptr()))?;
        }
    }
    Ok(())
}

fn present_native(anchor: HWND, options: &FileDialogOptions) -> FileDialogOutcome {
    let dialog: IFileOpenDialog =
        match unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) } {
            Ok(dialog) => dialog,
            Err(error) => {
                return FileDialogOutcome::Failed(FileDialogError::DialogFailed(format!(
                    "CoCreateInstance failed: {error}"
                )));
            }
        };
    let plan = plan(options);
    if let Err(error) = configure(&dialog, &plan) {
        return FileDialogOutcome::Failed(FileDialogError::DialogFailed(format!(
            "configuring the dialog failed: {error}"
        )));
    }
    // 安全：anchor 在 Host 中存活；Show 的模态循环由 shell 拥有。
    match unsafe { dialog.Show(Some(anchor)) } {
        Ok(()) => collect(&dialog, options.allows_multiple()),
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

fn collect(dialog: &IFileOpenDialog, multiple: bool) -> FileDialogOutcome {
    let result = if multiple {
        // 安全：GetResults 返回 shell 拥有的数组；路径在读取时复制。
        unsafe { dialog.GetResults() }.and_then(|items: IShellItemArray| collect_array(&items))
    } else {
        // 安全：GetResult 返回 shell 拥有的单项。
        unsafe { dialog.GetResult() }
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
        plan.filters
            .iter()
            .map(|filter| {
                // 安全：指针指向 plan.wide 中仍存活的 NUL 结尾字符串。
                let name = unsafe { filter.pszName.to_string().expect("filter name") };
                let spec = unsafe { filter.pszSpec.to_string().expect("filter spec") };
                (name, spec)
            })
            .collect()
    }

    #[test]
    fn single_and_multi_flags_include_required_options() {
        let single = plan(&FileDialogOptions::open_file());
        assert!(single.flags.contains(FOS_FORCEFILESYSTEM));
        assert!(single.flags.contains(FOS_FILEMUSTEXIST));
        assert!(single.flags.contains(FOS_PATHMUSTEXIST));
        assert!(!single.flags.contains(FOS_ALLOWMULTISELECT));

        let multi = plan(&FileDialogOptions::open_files());
        assert!(multi.flags.contains(FOS_ALLOWMULTISELECT));
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
