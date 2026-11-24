//! OLE 文件拖放。
//!
//! 每个可见且注册了 `on_drop` 的窗口拥有一个 `IDropTarget`。COM 回调只检查
//! 启用状态、`CF_HDROP` 与允许的 effect，把路径放进共享队列并向窗口投递一条
//! 宿主消息；真正的用户处理器在消息循环的 UI 线程上由宿主 drain 调用。关闭
//! 或隐藏窗口时撤销注册，旧 activation 的排队数据随 registration 一起释放。

use std::cell::RefCell;
use std::ffi::OsString;
use std::fmt;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::rc::Rc;

use windows::Win32::Foundation::{E_FAIL, HWND, POINTL};
use windows::Win32::System::Com::{
    DVASPECT_CONTENT, FORMATETC, IDataObject, STGMEDIUM, TYMED_HGLOBAL,
};
use windows::Win32::System::Ole::{
    CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl,
    RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{IsWindow, IsWindowVisible};
use windows::core::{Ref, Result as WinResult, implement};

use crate::ole::OleError;

/// 注册 OLE drop target 失败。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DragDropError {
    /// 当前 UI 线程不能承载 OLE 拖放（例如已经处于另一个 COM 套间）。
    Unsupported(&'static str),
    /// `OleInitialize` 失败。
    Ole(String),
    /// `RegisterDragDrop` 失败。
    Register(String),
}

impl DragDropError {
    pub(crate) fn from_ole(error: OleError) -> Self {
        match error {
            OleError::ChangedMode => Self::Unsupported(
                "OLE drag-and-drop requires the UI thread's single-threaded apartment",
            ),
            OleError::Failed(message) => Self::Ole(message),
        }
    }
}

impl fmt::Display for DragDropError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(operation) => f.write_str(operation),
            Self::Ole(message) => write!(f, "OLE initialization failed: {message}"),
            Self::Register(message) => write!(f, "RegisterDragDrop failed: {message}"),
        }
    }
}

impl std::error::Error for DragDropError {}

/// 窗口拖放注册；析构时撤销 `IDropTarget`。
///
/// 宿主在销毁 HWND 前释放本结构，`_target` 保持 COM 引用直到 `RevokeDragDrop`
/// 完成。
pub(crate) struct DropRegistration {
    hwnd: HWND,
    _target: IDropTarget,
    queue: Rc<RefCell<DropQueue>>,
}

impl DropRegistration {
    /// 在已初始化的 OLE 套间里为 HWND 注册文件 drop target。
    pub(crate) fn register(hwnd: HWND) -> Result<Self, DragDropError> {
        let queue = Rc::new(RefCell::new(DropQueue::default()));
        let target: IDropTarget = DropTarget {
            hwnd,
            queue: Rc::clone(&queue),
        }
        .into();
        // 安全：调用方先初始化 OLE；HWND 由窗口条目持有，registration 随它释放。
        unsafe { RegisterDragDrop(hwnd, &target) }
            .map_err(|error| DragDropError::Register(error.to_string()))?;
        Ok(Self {
            hwnd,
            _target: target,
            queue,
        })
    }

    /// 取走一次已经排队的 drop；没有 drop 时返回 `None`。
    pub(crate) fn take_pending(&self) -> Option<Vec<PathBuf>> {
        self.queue.borrow_mut().pending.take()
    }
}

impl Drop for DropRegistration {
    fn drop(&mut self) {
        // 安全：只撤销本 registration 注册的 HWND；窗口仍在或即将销毁都可调用。
        unsafe {
            let _ = RevokeDragDrop(self.hwnd);
        }
    }
}

#[derive(Default)]
struct DropQueue {
    accepting: bool,
    pending: Option<Vec<PathBuf>>,
}

#[implement(IDropTarget)]
struct DropTarget {
    hwnd: HWND,
    queue: Rc<RefCell<DropQueue>>,
}

impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(
        &self,
        data: Ref<IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        _point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> WinResult<()> {
        let accepted = drag_accepts(
            data.as_ref(),
            effect_value(effect),
            window_accepts_drop(self.hwnd),
        );
        self.queue.borrow_mut().accepting = accepted;
        set_effect(effect, accepted);
        Ok(())
    }

    fn DragOver(
        &self,
        _keys: MODIFIERKEYS_FLAGS,
        _point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> WinResult<()> {
        let accepted = self.queue.borrow().accepting && window_accepts_drop(self.hwnd);
        set_effect(effect, accepted);
        Ok(())
    }

    fn DragLeave(&self) -> WinResult<()> {
        self.queue.borrow_mut().accepting = false;
        Ok(())
    }

    fn Drop(
        &self,
        data: Ref<IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        _point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> WinResult<()> {
        let ready =
            window_accepts_drop(self.hwnd) && effect_value(effect).contains(DROPEFFECT_COPY);
        let paths = ready.then(|| collect_paths(data.as_ref())).flatten();
        let Some(paths) = paths.filter(|paths| !paths.is_empty()) else {
            self.queue.borrow_mut().accepting = false;
            set_effect(effect, false);
            return Ok(());
        };
        {
            let mut queue = self.queue.borrow_mut();
            queue.accepting = false;
            queue.pending = Some(paths);
        }
        set_effect(effect, true);
        if !crate::host::post_drop_drain(self.hwnd) {
            self.queue.borrow_mut().pending = None;
            set_effect(effect, false);
            return Err(E_FAIL.into());
        }
        Ok(())
    }
}

/// COM 回调使用的启用检查；隐藏、销毁与禁用窗口都不接收拖放。
fn window_accepts_drop(hwnd: HWND) -> bool {
    // 安全：只查询句柄状态；窗口可能在查询期间销毁，失败按不可用处理。
    unsafe {
        IsWindow(Some(hwnd)).as_bool()
            && IsWindowVisible(hwnd).as_bool()
            && IsWindowEnabled(hwnd).as_bool()
    }
}

/// 只有启用窗口、允许 COPY 且数据对象提供 `CF_HDROP` 时才接受。
fn drag_accepts(data: Option<&IDataObject>, allowed: DROPEFFECT, enabled: bool) -> bool {
    enabled && allowed.contains(DROPEFFECT_COPY) && data.is_some_and(has_hdrop)
}

fn has_hdrop(data: &IDataObject) -> bool {
    // 安全：QueryGetData 只查询格式是否可用，不转移 medium。
    unsafe { data.QueryGetData(&hdrop_format()).is_ok() }
}

fn hdrop_format() -> FORMATETC {
    FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

fn effect_value(effect: *const DROPEFFECT) -> DROPEFFECT {
    if effect.is_null() {
        DROPEFFECT_NONE
    } else {
        // 安全：调用方保证指针在本回调期间可读。
        unsafe { *effect }
    }
}

fn set_effect(effect: *mut DROPEFFECT, accepted: bool) {
    if !effect.is_null() {
        // 安全：调用方保证指针在本回调期间可写。
        unsafe {
            *effect = if accepted {
                DROPEFFECT_COPY
            } else {
                DROPEFFECT_NONE
            };
        }
    }
}

/// 从 `CF_HDROP` 提取有序路径；失败与空列表都返回 `None`。
fn collect_paths(data: Option<&IDataObject>) -> Option<Vec<PathBuf>> {
    let data = data?;
    let mut medium = unsafe { data.GetData(&hdrop_format()).ok() }?;
    let paths = unsafe { paths_from_medium(&medium) };
    // 安全：medium 由 GetData 返回，所有路径都已复制。
    unsafe { ReleaseStgMedium(&mut medium) };
    paths
}

unsafe fn paths_from_medium(medium: &STGMEDIUM) -> Option<Vec<PathBuf>> {
    if medium.tymed != TYMED_HGLOBAL.0 as u32 {
        return None;
    }
    // 安全：tymed 是 HGLOBAL；CF_HDROP 的 hGlobal 就是 HDROP。
    let handle = unsafe { medium.u.hGlobal };
    if handle.0.is_null() {
        return None;
    }
    let hdrop = HDROP(handle.0);
    // 安全：hdrop 在本函数调用期间有效，由 medium 持有。
    let count = unsafe { DragQueryFileW(hdrop, u32::MAX, None) };
    if count == 0 {
        return None;
    }
    let mut paths = Vec::with_capacity(count as usize);
    for index in 0..count {
        // 安全：先查询长度，再用足够大的缓冲区提取同一路径。
        let length = unsafe { DragQueryFileW(hdrop, index, None) };
        if length == 0 {
            return None;
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let written = unsafe { DragQueryFileW(hdrop, index, Some(&mut buffer)) };
        if written == 0 {
            return None;
        }
        // 不用 `from_utf16_lossy`：Windows 路径是 UTF-16，未配对代理项也属于
        // 文件名的一部分，必须原样交给 `OsString`，不能替换成 U+FFFD。
        paths.push(PathBuf::from(OsString::from_wide(
            &buffer[..written as usize],
        )));
    }
    Some(paths)
}

/// 原生宿主是否应该为窗口注册 drop target。
pub(crate) fn should_register(show_windows: bool, visible: bool, has_handler: bool) -> bool {
    show_windows && visible && has_handler
}

#[cfg(test)]
mod tests {
    use std::mem::ManuallyDrop;

    use windows::Win32::Foundation::{DV_E_FORMATETC, E_NOTIMPL, GlobalFree, HGLOBAL, S_OK};
    use windows::Win32::System::Com::{
        IAdviseSink, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA, STGMEDIUM_0,
    };
    use windows::Win32::System::Memory::{
        GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock,
    };
    use windows::Win32::UI::Shell::DROPFILES;
    use windows::core::{BOOL, Error as WinError, HRESULT};

    use super::*;

    #[implement(IDataObject)]
    struct FakeDataObject {
        paths: Option<Vec<Vec<u16>>>,
    }

    impl IDataObject_Impl for FakeDataObject_Impl {
        fn GetData(&self, format: *const FORMATETC) -> WinResult<STGMEDIUM> {
            // 安全：调用方传入的 FORMATETC 在调用期间有效。
            let format =
                unsafe { format.as_ref() }.ok_or_else(|| WinError::from(DV_E_FORMATETC))?;
            if format.cfFormat != CF_HDROP.0 {
                return Err(DV_E_FORMATETC.into());
            }
            let Some(paths) = &self.paths else {
                return Err(DV_E_FORMATETC.into());
            };
            let memory = build_hdrop(paths);
            Ok(STGMEDIUM {
                tymed: TYMED_HGLOBAL.0 as u32,
                u: STGMEDIUM_0 { hGlobal: memory },
                pUnkForRelease: ManuallyDrop::new(None),
            })
        }

        fn GetDataHere(&self, _format: *const FORMATETC, _medium: *mut STGMEDIUM) -> WinResult<()> {
            Err(E_NOTIMPL.into())
        }

        fn QueryGetData(&self, format: *const FORMATETC) -> HRESULT {
            // 安全：调用方传入的 FORMATETC 在调用期间有效。
            let format = unsafe { format.as_ref() };
            if self.paths.is_some() && format.is_some_and(|format| format.cfFormat == CF_HDROP.0) {
                S_OK
            } else {
                DV_E_FORMATETC
            }
        }

        fn GetCanonicalFormatEtc(
            &self,
            _input: *const FORMATETC,
            _output: *mut FORMATETC,
        ) -> HRESULT {
            E_NOTIMPL
        }

        fn SetData(
            &self,
            _format: *const FORMATETC,
            _medium: *const STGMEDIUM,
            _release: BOOL,
        ) -> WinResult<()> {
            Err(E_NOTIMPL.into())
        }

        fn EnumFormatEtc(&self, _direction: u32) -> WinResult<IEnumFORMATETC> {
            Err(E_NOTIMPL.into())
        }

        fn DAdvise(
            &self,
            _format: *const FORMATETC,
            _flags: u32,
            _sink: Ref<IAdviseSink>,
        ) -> WinResult<u32> {
            Err(E_NOTIMPL.into())
        }

        fn DUnadvise(&self, _connection: u32) -> WinResult<()> {
            Err(E_NOTIMPL.into())
        }

        fn EnumDAdvise(&self) -> WinResult<IEnumSTATDATA> {
            Err(E_NOTIMPL.into())
        }
    }

    fn build_hdrop(paths: &[Vec<u16>]) -> HGLOBAL {
        let mut text: Vec<u16> = Vec::new();
        for path in paths {
            text.extend_from_slice(path);
            text.push(0);
        }
        text.push(0);
        let header_size = std::mem::size_of::<DROPFILES>();
        let bytes = header_size + text.len() * 2;
        // 安全：分配一块可移动的全局内存；失败返回空句柄，由调用路径释放。
        let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes) }
            .expect("allocate drop memory");
        // 安全：锁定的内存在 GlobalUnlock 前有效。
        let pointer = unsafe { GlobalLock(memory) } as *mut u8;
        assert!(!pointer.is_null(), "lock drop memory");
        // 安全：目标缓冲至少 header + text 字节。
        unsafe {
            let header = DROPFILES {
                pFiles: header_size as u32,
                pt: Default::default(),
                fNC: BOOL(0),
                fWide: BOOL(1),
            };
            std::ptr::copy_nonoverlapping(
                &header as *const DROPFILES as *const u8,
                pointer,
                header_size,
            );
            std::ptr::copy_nonoverlapping(
                text.as_ptr() as *const u8,
                pointer.add(header_size),
                text.len() * 2,
            );
            let _ = GlobalUnlock(memory);
        }
        memory
    }

    fn fake(paths: Option<Vec<&str>>) -> IDataObject {
        fake_wide(paths.map(|paths| {
            paths
                .into_iter()
                .map(|path| path.encode_utf16().collect())
                .collect()
        }))
    }

    fn fake_wide(paths: Option<Vec<Vec<u16>>>) -> IDataObject {
        FakeDataObject { paths }.into()
    }

    #[test]
    fn a_foreign_apartment_maps_to_unsupported_and_other_failures_keep_ole_context() {
        assert!(matches!(
            DragDropError::from_ole(OleError::ChangedMode),
            DragDropError::Unsupported(_)
        ));
        assert!(matches!(
            DragDropError::from_ole(OleError::Failed(String::from("boom"))),
            DragDropError::Ole(message) if message == "boom"
        ));
    }

    #[test]
    fn unicode_paths_are_extracted_in_order_with_duplicates() {
        let data = fake(Some(vec![
            "C:/证据/你好 🙂.txt",
            "duplicate.txt",
            "duplicate.txt",
        ]));
        assert_eq!(
            collect_paths(Some(&data)).expect("extract paths"),
            [
                PathBuf::from("C:/证据/你好 🙂.txt"),
                PathBuf::from("duplicate.txt"),
                PathBuf::from("duplicate.txt")
            ]
        );
    }

    #[test]
    fn unpaired_surrogates_are_preserved_in_paths() {
        let raw = vec![0x0041, 0xD800, 0x0042];
        let data = fake_wide(Some(vec![raw.clone()]));
        assert_eq!(
            collect_paths(Some(&data)).expect("extract paths"),
            [PathBuf::from(OsString::from_wide(&raw))]
        );
    }

    #[test]
    fn missing_or_empty_hdrop_is_rejected() {
        let empty = fake(Some(Vec::new()));
        assert_eq!(collect_paths(Some(&empty)), None);
        let missing = fake(None);
        assert_eq!(collect_paths(Some(&missing)), None);
        assert_eq!(collect_paths(None), None);
    }

    #[test]
    fn effects_require_copy_an_enabled_window_and_a_file_list() {
        let data = fake(Some(vec!["one.txt"]));
        assert!(drag_accepts(Some(&data), DROPEFFECT_COPY, true));
        assert!(!drag_accepts(Some(&data), DROPEFFECT(2), true));
        assert!(!drag_accepts(Some(&data), DROPEFFECT_COPY, false));
        assert!(!drag_accepts(None, DROPEFFECT_COPY, true));
        let missing = fake(None);
        assert!(!drag_accepts(Some(&missing), DROPEFFECT_COPY, true));
    }

    #[test]
    fn only_visible_handled_windows_register_in_a_visible_host() {
        assert!(should_register(true, true, true));
        assert!(!should_register(false, true, true));
        assert!(!should_register(true, false, true));
        assert!(!should_register(true, true, false));
    }

    #[test]
    fn drop_memory_is_released_by_the_medium_contract() {
        // 这条测试只验证测试数据构造与释放不泄漏；真实释放由 provider 路径完成。
        let memory = build_hdrop(&[String::from("a.txt").encode_utf16().collect()]);
        // 安全：测试自己拥有这块分配。
        let _ = unsafe { GlobalFree(Some(memory)) };
    }
}
