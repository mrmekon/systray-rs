use {SystrayEvent, SystrayAction, SystrayError, Callback, make_callback};
use std;
use std::cell::{Cell, RefCell};
use std::sync::mpsc::{channel, Sender, Receiver, TryRecvError};
use std::os::windows::ffi::OsStrExt;
use std::ffi::OsStr;
use std::thread;
use std::slice;
use std::collections::HashMap;
use encoding::{Encoding, EncoderTrap};
use encoding::all::UTF_16LE;
use winapi;
use winapi::{MENUITEMINFOW, LPMENUITEMINFOA, LPMENUITEMINFOW, c_int, RECT, UINT, BOOL, ULONG_PTR, CHAR, GUID, WCHAR};
use user32;
use kernel32;
use winapi::windef::{HWND, HMENU, HICON, HBRUSH, HBITMAP};
use winapi::winnt::{LPCWSTR};
use winapi::minwindef::{DWORD, WPARAM, LPARAM, LRESULT, HINSTANCE, TRUE, PBYTE};
use winapi::winuser::{WNDCLASSW, WS_OVERLAPPEDWINDOW, CW_USEDEFAULT, LR_DEFAULTCOLOR};

// Until winapi hits 0.3 on crates.io, add these so we can publish a crate.
macro_rules! UNION {
    ($base:ident, $field:ident, $variant:ident, $variantmut:ident, $fieldtype:ty) => {
        impl $base {
            #[inline]
            pub unsafe fn $variant(&self) -> &$fieldtype {
                ::std::mem::transmute(&self.$field)
            }
            #[inline]
            pub unsafe fn $variantmut(&mut self) -> &mut $fieldtype {
                ::std::mem::transmute(&mut self.$field)
            }
        }
    }
}

macro_rules! STRUCT {
    {$(#[$attrs:meta])* nodebug struct $name:ident { $($field:ident: $ftype:ty,)+ }} => {
        #[repr(C)] $(#[$attrs])*
        pub struct $name {
            $(pub $field: $ftype,)+
        }
        impl Copy for $name {}
        impl Clone for $name { fn clone(&self) -> $name { *self } }
    };
    {$(#[$attrs:meta])* struct $name:ident { $($field:ident: $ftype:ty,)+ }} => {
        #[repr(C)] #[derive(Debug)] $(#[$attrs])*
        pub struct $name {
            $(pub $field: $ftype,)+
        }
        impl Copy for $name {}
        impl Clone for $name { fn clone(&self) -> $name { *self } }
    };
}

extern "system" {
    pub fn GetMenuInfo(hMenu: HMENU, lpcmi: LPMENUINFO) -> BOOL;
    pub fn GetMenuItemCount(hMenu: HMENU) -> c_int;
    pub fn GetMenuItemID(hMenu: HMENU, nPos: c_int) -> UINT;
    pub fn GetMenuItemInfoA(hMenu: HMENU, uItem: UINT, fByPosition: BOOL, lpmii: LPMENUITEMINFOA) -> BOOL;
    pub fn GetMenuItemInfoW(hMenu: HMENU, uItem: UINT, fByPosition: BOOL, lpmii: LPMENUITEMINFOW) -> BOOL;
    pub fn SetMenuInfo(hMenu: HMENU, lpcmi: LPCMENUINFO) -> BOOL;
    pub fn TrackPopupMenu(hMenu: HMENU, uFlags: UINT, x: c_int, y: c_int, nReserved: c_int,
                      hWnd: HWND, prcRect: *const RECT);
    pub fn TrackPopupMenuEx(hMenu: HMENU, fuFlags: UINT, x: c_int, y: c_int, hWnd: HWND,
                            lptpm: LPTPMPARAMS);
    pub fn Shell_NotifyIconA(dwMessage: DWORD, lpData: PNOTIFYICONDATAA) -> BOOL;
    pub fn Shell_NotifyIconW(dwMessage: DWORD, lpData: PNOTIFYICONDATAW) -> BOOL;
}


pub const NIM_ADD: DWORD = 0x00000000;
pub const NIM_MODIFY: DWORD = 0x00000001;
pub const NIM_DELETE: DWORD = 0x00000002;
pub const NIM_SETFOCUS: DWORD = 0x00000003;
pub const NIM_SETVERSION: DWORD = 0x00000004;
pub const NIF_MESSAGE: UINT = 0x00000001;
pub const NIF_ICON: UINT = 0x00000002;
pub const NIF_TIP: UINT = 0x00000004;
pub const NIF_STATE: UINT = 0x00000008;
pub const NIF_INFO: UINT = 0x00000010;
pub const NIF_GUID: UINT = 0x00000020;
pub const NIF_REALTIME: UINT = 0x00000040;
pub const NIF_SHOWTIP: UINT = 0x00000080;
pub const NOTIFYICON_VERSION: UINT = 3;
pub const NOTIFYICON_VERSION_4: UINT = 4;

pub const MF_BYCOMMAND: UINT = 0x00000000;
pub const MF_BYPOSITION: UINT = 0x00000400;
pub const MF_UNCHECKED: UINT = 0x00000000;
pub const MF_CHECKED: UINT = 0x00000008;
pub const MF_ENABLED: UINT = 0x00000000;
pub const MF_GRAYED: UINT = 0x00000001;
pub const MF_DISABLED: UINT = 0x00000002;

STRUCT!{nodebug struct NOTIFYICONDATAA {
    cbSize: DWORD,
    hWnd: HWND,
    uID: UINT,
    uFlags: UINT,
    uCallbackMessage: UINT,
    hIcon: HICON,
    szTip: [CHAR; 128],
    dwState: DWORD,
    dwStateMask: DWORD,
    szInfo: [CHAR; 256],
    uTimeout: UINT,
    szInfoTitle: [CHAR; 64],
    dwInfoFlags: DWORD,
    guidItem: GUID,
    hBalloonIcon: HICON,
}}
UNION!(NOTIFYICONDATAA, uTimeout, uTimeout, uTimeout_mut, UINT);
UNION!(NOTIFYICONDATAA, uTimeout, uVersion, uVersion_mut, UINT);
pub type PNOTIFYICONDATAA = *mut NOTIFYICONDATAA;

STRUCT!{nodebug struct NOTIFYICONDATAW {
    cbSize: DWORD,
    hWnd: HWND,
    uID: UINT,
    uFlags: UINT,
    uCallbackMessage: UINT,
    hIcon: HICON,
    szTip: [WCHAR; 128],
    dwState: DWORD,
    dwStateMask: DWORD,
    szInfo: [WCHAR; 256],
    uTimeout: UINT,
    szInfoTitle: [WCHAR; 64],
    dwInfoFlags: DWORD,
    guidItem: GUID,
    hBalloonIcon: HICON,
}}
UNION!(NOTIFYICONDATAW, uTimeout, uTimeout, uTimeout_mut, UINT);
UNION!(NOTIFYICONDATAW, uTimeout, uVersion, uVersion_mut, UINT); // used with NIM_SETVERSION, values 0, 3 and 4

pub type PNOTIFYICONDATAW = *mut NOTIFYICONDATAW;
pub const MIIM_BITMAP: UINT = 0x00000080;
pub const MIIM_CHECKMARKS: UINT = 0x00000008;
pub const MIIM_DATA: UINT = 0x00000020;
pub const MIIM_FTYPE: UINT = 0x00000100;
pub const MIIM_ID: UINT = 0x00000002;
pub const MIIM_STATE: UINT = 0x00000001;
pub const MIIM_STRING: UINT = 0x00000040;
pub const MIIM_SUBMENU: UINT = 0x00000004;
pub const MIIM_TYPE: UINT = 0x00000010;

pub const MFT_BITMAP: UINT = 0x00000004;
pub const MFT_MENUBARBREAK: UINT = 0x00000020;
pub const MFT_MENUBREAK: UINT = 0x00000040;
pub const MFT_OWNERDRAW: UINT = 0x00000100;
pub const MFT_RADIOCHECK: UINT = 0x00000200;
pub const MFT_RIGHTJUSTIFY: UINT = 0x00004000;
pub const MFT_RIGHTORDER: UINT = 0x00002000;
pub const MFT_SEPARATOR: UINT = 0x00000800;
pub const MFT_STRING: UINT = 0x00000000;

pub const MFS_CHECKED: UINT = 0x00000008;
pub const MFS_DEFAULT: UINT = 0x00001000;
pub const MFS_DISABLED: UINT = 0x00000003;
pub const MFS_ENABLED: UINT = 0x00000000;
pub const MFS_GRAYED: UINT = 0x00000003;
pub const MFS_HILITE: UINT = 0x00000080;
pub const MFS_UNCHECKED: UINT = 0x00000000;
pub const MFS_UNHILITE: UINT = 0x00000000;

//pub const HBMMENU_CALLBACK: HBITMAP = -1 as HBITMAP;
pub const HBMMENU_MBAR_CLOSE: HBITMAP = 5 as HBITMAP;
pub const HBMMENU_MBAR_CLOSE_D: HBITMAP = 6 as HBITMAP;
pub const HBMMENU_MBAR_MINIMIZE: HBITMAP = 3 as HBITMAP;
pub const HBMMENU_MBAR_MINIMIZE_D: HBITMAP = 7 as HBITMAP;
pub const HBMMENU_MBAR_RESTORE: HBITMAP = 2 as HBITMAP;
pub const HBMMENU_POPUP_CLOSE: HBITMAP = 8 as HBITMAP;
pub const HBMMENU_POPUP_MAXIMIZE: HBITMAP = 10 as HBITMAP;
pub const HBMMENU_POPUP_MINIMIZE: HBITMAP = 11 as HBITMAP;
pub const HBMMENU_POPUP_RESTORE: HBITMAP = 9 as HBITMAP;
pub const HBMMENU_SYSTEM: HBITMAP = 1 as HBITMAP;

pub const MIM_MAXHEIGHT: UINT = 0x00000001;
pub const MIM_BACKGROUND: UINT = 0x00000002;
pub const MIM_HELPID: UINT = 0x00000004;
pub const MIM_MENUDATA: UINT = 0x00000008;
pub const MIM_STYLE: UINT = 0x00000010;
pub const MIM_APPLYTOSUBMENUS: UINT = 0x80000000;

pub const MNS_CHECKORBMP: UINT = 0x04000000;
pub const MNS_NOTIFYBYPOS: UINT = 0x08000000;
pub const MNS_AUTODISMISS: UINT = 0x10000000;
pub const MNS_DRAGDROP: UINT = 0x20000000;
pub const MNS_MODELESS: UINT = 0x40000000;
pub const MNS_NOCHECK: UINT = 0x80000000;

STRUCT!{struct MENUINFO {
    cbSize: DWORD,
    fMask: DWORD,
    dwStyle: DWORD,
    cyMax: UINT,
    hbrBack: HBRUSH,
    dwContextHelpID: DWORD,
    dwMenuData: ULONG_PTR,
}}
pub type LPMENUINFO = *mut MENUINFO;
pub type LPCMENUINFO = *const MENUINFO;

pub const TPM_LEFTALIGN: UINT = 0x0000;
pub const TPM_CENTERALIGN: UINT = 0x0004;
pub const TPM_RIGHTALIGN: UINT = 0x0008;
pub const TPM_TOPALIGN: UINT = 0x0000;
pub const TPM_VCENTERALIGN: UINT = 0x0010;
pub const TPM_BOTTOMALIGN: UINT = 0x0020;
pub const TPM_NONOTIFY: UINT = 0x0080;
pub const TPM_RETURNCMD: UINT = 0x0100;
pub const TPM_LEFTBUTTON: UINT = 0x0000;
pub const TPM_RIGHTBUTTON: UINT = 0x0002;
pub const TPM_HORNEGANIMATION: UINT = 0x0800;
pub const TPM_HORPOSANIMATION: UINT = 0x0400;
pub const TPM_NOANIMATION: UINT = 0x4000;
pub const TPM_VERNEGANIMATION: UINT = 0x2000;
pub const TPM_VERPOSANIMATION: UINT = 0x1000;

STRUCT!{struct TPMPARAMS {
    cbSize: UINT,
    rcExclude: RECT,
}}

pub type LPTPMPARAMS = *const TPMPARAMS;

pub enum MenuEnableFlag {
    Enabled,
    Disabled,
    Grayed,
}

fn to_wstring(str : &str) -> Vec<u16> {
    OsStr::new(str).encode_wide().chain(Some(0).into_iter()).collect::<Vec<_>>()
}

// Got this idea from glutin. Yay open source! Boo stupid winproc! Even more boo
// doing SetLongPtr tho.
thread_local!(static WININFO_STASH: RefCell<Option<WindowsLoopData>> = RefCell::new(None));

#[derive(Clone)]
struct WindowInfo {
    pub hwnd: HWND,
    pub hinstance: HINSTANCE,
    pub hmenu: HMENU,
}

unsafe impl Send for WindowInfo {}
unsafe impl Sync for WindowInfo {}

#[derive(Clone)]
struct WindowsLoopData {
    pub info: WindowInfo,
    pub tx: Sender<SystrayEvent>
}

unsafe fn get_win_os_error(msg: &str) -> SystrayError {
    SystrayError::OsError(format!("{}: {}", &msg, kernel32::GetLastError()))
}

unsafe extern "system" fn window_proc(h_wnd :HWND,
	                                    msg :UINT,
                                      w_param :WPARAM,
                                      l_param :LPARAM) -> LRESULT
{
    if msg == winapi::winuser::WM_MENUCOMMAND {
        WININFO_STASH.with(|stash| {
            let stash = stash.borrow();
            let stash = stash.as_ref();
            if let Some(stash) = stash {
                let menu_id = GetMenuItemID(stash.info.hmenu,
                                            w_param as i32) as i32;
                if menu_id != -1 {
                    stash.tx.send(SystrayEvent {
                        action: SystrayAction::SelectItem,
                        menu_index: menu_id as u32,
                    }).ok();
                }
            }
        });
    }

    if msg == winapi::winuser::WM_UNINITMENUPOPUP {
        WININFO_STASH.with(|stash| {
            let stash = stash.borrow();
            let stash = stash.as_ref();
            if let Some(stash) = stash {
                stash.tx.send(SystrayEvent {
                    action: SystrayAction::HideMenu,
                    menu_index: 0,
                }).ok();
            }
        });
    }

    if msg == winapi::winuser::WM_USER + 1 {
        if l_param as UINT == winapi::winuser::WM_LBUTTONUP ||
            l_param as UINT == winapi::winuser::WM_RBUTTONUP {
                let mut p = winapi::POINT {
                    x: 0,
                    y: 0
                };
                if user32::GetCursorPos(&mut p as *mut winapi::POINT) == 0 {
                    return 1;
                }
                user32::SetForegroundWindow(h_wnd);
                WININFO_STASH.with(|stash| {
                    let stash = stash.borrow();
                    let stash = stash.as_ref();
                    if let Some(stash) = stash {
                        stash.tx.send(SystrayEvent {
                            action: SystrayAction::DisplayMenu,
                            menu_index: 0,
                        }).ok();
                        TrackPopupMenu(stash.info.hmenu,
                                       0,
                                       p.x,
                                       p.y,
                                       (TPM_BOTTOMALIGN | TPM_LEFTALIGN) as i32,
                                       h_wnd,
                                       std::ptr::null_mut());
                    }
                });
            }
    }
    if msg == winapi::winuser::WM_DESTROY {
        user32::PostQuitMessage(0);
    }
    return user32::DefWindowProcW(h_wnd, msg, w_param, l_param);
}

fn get_nid_struct(hwnd : &HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as DWORD,
        hWnd: *hwnd,
        uID: 0x1 as UINT,
        uFlags: 0 as UINT,
        uCallbackMessage: 0 as UINT,
        hIcon: 0 as HICON,
        szTip: [0 as u16; 128],
        dwState: 0 as DWORD,
        dwStateMask: 0 as DWORD,
        szInfo: [0 as u16; 256],
        uTimeout: 0 as UINT,
        szInfoTitle: [0 as u16; 64],
        dwInfoFlags: 0 as UINT,
        guidItem: winapi::GUID {
            Data1: 0 as winapi::c_ulong,
            Data2: 0 as winapi::c_ushort,
            Data3: 0 as winapi::c_ushort,
            Data4: [0; 8]
        },
        hBalloonIcon: 0 as HICON
    }
}

fn get_menu_item_struct() -> MENUITEMINFOW {
    winapi::MENUITEMINFOW {
        cbSize: std::mem::size_of::<winapi::MENUITEMINFOW>() as UINT,
        fMask: 0 as UINT,
        fType: 0 as UINT,
        fState: 0 as UINT,
        wID: 0 as UINT,
        hSubMenu: 0 as HMENU,
        hbmpChecked: 0 as HBITMAP,
        hbmpUnchecked: 0 as HBITMAP,
        dwItemData: 0 as winapi::ULONG_PTR,
        dwTypeData: std::ptr::null_mut(),
        cch: 0 as u32,
        hbmpItem: 0 as HBITMAP
    }
}

unsafe fn init_window() -> Result<WindowInfo, SystrayError> {
    let class_name = to_wstring("my_window");
    let hinstance : HINSTANCE = kernel32::GetModuleHandleA(std::ptr::null_mut());
    let wnd = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(window_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: 0 as HINSTANCE,
        hIcon: user32::LoadIconW(0 as HINSTANCE,
                                 winapi::winuser::IDI_APPLICATION),
        hCursor: user32::LoadCursorW(0 as HINSTANCE,
                                     winapi::winuser::IDI_APPLICATION),
        hbrBackground: 16 as HBRUSH,
        lpszMenuName: 0 as LPCWSTR,
        lpszClassName: class_name.as_ptr(),
    };
    if user32::RegisterClassW(&wnd) == 0 {
        return Err(get_win_os_error("Error creating window class"));
    }
    let hwnd = user32::CreateWindowExW(0,
                                       class_name.as_ptr(),
                                       to_wstring("rust_systray_window").as_ptr(),
                                       WS_OVERLAPPEDWINDOW,
                                       CW_USEDEFAULT,
                                       0,
                                       CW_USEDEFAULT,
                                       0,
                                       0 as HWND,
                                       0 as HMENU,
                                       0 as HINSTANCE,
                                       std::ptr::null_mut());
    if hwnd == std::ptr::null_mut() {
        return Err(get_win_os_error("Error creating window"));
    }
    let mut nid = get_nid_struct(&hwnd);
    nid.uID = 0x1;
    nid.uFlags = winapi::NIF_MESSAGE;
    nid.uCallbackMessage = winapi::WM_USER + 1;
    if Shell_NotifyIconW(winapi::NIM_ADD,
                                  &mut nid as *mut NOTIFYICONDATAW) == 0 {
        return Err(get_win_os_error("Error adding menu icon"));
    }
    // Setup menu
    let hmenu = user32::CreatePopupMenu();
    let m = MENUINFO {
        cbSize: std::mem::size_of::<MENUINFO>() as DWORD,
        fMask: MIM_APPLYTOSUBMENUS | MIM_STYLE,
        dwStyle: MNS_NOTIFYBYPOS,
        cyMax: 0 as UINT,
        hbrBack: 0 as HBRUSH,
        dwContextHelpID: 0 as DWORD,
        dwMenuData: 0 as winapi::ULONG_PTR
    };
    if SetMenuInfo(hmenu, &m as *const MENUINFO) == 0 {
        return Err(get_win_os_error("Error setting up menu"));
    }

    Ok(WindowInfo {
        hwnd: hwnd,
        hmenu: hmenu,
        hinstance: hinstance,
    })
}

unsafe fn run_loop() {
    debug!("Running windows loop");
    // Run message loop
    let mut msg = winapi::winuser::MSG {
        hwnd: 0 as HWND,
        message: 0 as UINT,
        wParam: 0 as WPARAM,
        lParam: 0 as LPARAM,
        time: 0 as DWORD,
        pt: winapi::windef::POINT { x: 0, y: 0, },
    };
    loop {
        user32::GetMessageW(&mut msg, 0 as HWND, 0, 0);
        if msg.message == winapi::winuser::WM_QUIT {
            break;
        }
        user32::TranslateMessage(&mut msg);
        user32::DispatchMessageW(&mut msg);
    }
    debug!("Leaving windows run loop");
}

pub struct Window {
    info: WindowInfo,
    windows_loop: Option<thread::JoinHandle<()>>,
    menu_idx: Cell<u32>,
    callback: RefCell<HashMap<u32, Callback>>,
    pub rx: Receiver<SystrayEvent>,
    menu_displayed: Cell<bool>,
}

impl Window {
    pub fn new() -> Result<Window, SystrayError> {
        let (tx, rx) = channel();
        let (event_tx, event_rx) = channel();
        let windows_loop = thread::spawn(move || {
            unsafe {
                let i = init_window();
                let k;
                match i {
                    Ok(j) => {
                        tx.send(Ok(j.clone())).ok();
                        k = j;
                    }
                    Err(e) => {
                        // If creation didn't work, return out of the thread.
                        tx.send(Err(e)).ok();
                        return;
                    }
                };
                WININFO_STASH.with(|stash| {
                    let data = WindowsLoopData {
                        info: k,
                        tx: event_tx
                    };
                    (*stash.borrow_mut()) = Some(data);
                });
                run_loop();
            }
        });
        let info = match rx.recv().unwrap() {
            Ok(i) => i,
            Err(e) => {
                return Err(e);
            }
        };
        let w = Window {
            info: info,
            windows_loop: Some(windows_loop),
            rx: event_rx,
            menu_idx: Cell::new(0),
            callback: RefCell::new(HashMap::new()),
            menu_displayed: Cell::new(false),
        };
        Ok(w)
    }

    pub fn quit(&self) {
        unsafe {
            user32::PostMessageW(self.info.hwnd, winapi::WM_DESTROY,
                                 0 as WPARAM, 0 as LPARAM);
        }
    }


    pub fn set_tooltip(&self, tooltip: &String) -> Result<(), SystrayError> {
        // Add Tooltip
        debug!("Setting tooltip to {}", tooltip);
        let mut nid = get_nid_struct(&self.info.hwnd);
        // Gross way to convert String to UTF-16 [i16; 128]
        // TODO: Clean up conversion, test for length so we don't panic at runtime
        let mut v: Vec<u8> = UTF_16LE.encode(&tooltip, EncoderTrap::Strict).unwrap();
        v.push(0); v.push(0); // NUL-terminate
        let utf16: &[u16] = unsafe {
            slice::from_raw_parts(v.as_ptr() as *const _, v.len()/2)
        };
        for i in 0..std::cmp::min(utf16.len(), 128) {
            nid.szTip[i] = utf16[i];
        }
        nid.szTip[127] = 0; // NUL-terminate
        nid.uFlags = winapi::NIF_TIP;
        unsafe {
            if Shell_NotifyIconW(winapi::NIM_MODIFY,
                                          &mut nid as *mut NOTIFYICONDATAW) == 0 {
                return Err(get_win_os_error("Error setting tooltip"));
            }
        }
        Ok(())
    }

    pub fn select_menu_item(&self, item: u32) -> Result<u32, SystrayError> {
        unsafe {
            if user32::CheckMenuItem(self.info.hmenu,
                                     item,
                                     MF_BYPOSITION | MF_CHECKED) == 0 {
                return Err(get_win_os_error("Error checking menu item"));
            }
        }
        Ok(item)
    }

    pub fn enable_menu_item(&self, item: u32, enable: MenuEnableFlag) -> Result<u32, SystrayError> {
        let flags = MF_BYPOSITION | match enable {
            MenuEnableFlag::Enabled => MF_ENABLED,
            MenuEnableFlag::Disabled => MF_DISABLED,
            MenuEnableFlag::Grayed => MF_GRAYED,
        };
        unsafe {
            if user32::EnableMenuItem(self.info.hmenu,
                                     item,
                                     flags) == 0 {
                return Err(get_win_os_error("Error enabling menu item"));
            }
        }
        Ok(item)
    }

    pub fn unselect_menu_item(&self, item: u32) -> Result<u32, SystrayError> {
        unsafe {
            if user32::CheckMenuItem(self.info.hmenu,
                                     item,
                                     MF_BYPOSITION | MF_UNCHECKED) == 0 {
                return Err(get_win_os_error("Error unchecking menu item"));
            }
        }
        Ok(item)
    }

    fn add_menu_entry(&self, item_name: &String, checked: bool) -> Result<u32, SystrayError> {
        let mut st = to_wstring(item_name);
        let idx = self.menu_idx.get();
        self.menu_idx.set(idx + 1);
        let mut item = get_menu_item_struct();
        item.fMask = MIIM_FTYPE | MIIM_STRING | MIIM_ID | MIIM_STATE | MIIM_CHECKMARKS;
        if checked {
            item.fState = MFS_CHECKED;
        }
        item.fType = MFT_STRING;
        item.wID = idx;
        item.dwTypeData = st.as_mut_ptr();
        item.cch = (item_name.len() * 2) as u32;
        unsafe {
            if user32::InsertMenuItemW(self.info.hmenu,
                                       idx,
                                       1,
                                       &item as *const winapi::MENUITEMINFOW) == 0 {
                return Err(get_win_os_error("Error inserting menu item"));
            }
        }
        Ok(idx)
    }

    pub fn add_menu_separator(&self) -> Result<u32, SystrayError> {
        let idx = self.menu_idx.get();
        self.menu_idx.set(idx + 1);
        let mut item = get_menu_item_struct();
        item.fMask = MIIM_FTYPE;
        item.fType = MFT_SEPARATOR;
        item.wID = idx;
        unsafe {
            if user32::InsertMenuItemW(self.info.hmenu,
                                       idx,
                                       1,
                                       &item as *const winapi::MENUITEMINFOW) == 0 {
                return Err(get_win_os_error("Error inserting separator"));
            }
        }
        Ok(idx)
    }

    pub fn add_menu_item<F>(&self, item_name: &String, checked: bool, f: F) -> Result<u32, SystrayError>
        where F: std::ops::Fn(&Window) -> () + 'static {
        let idx = match self.add_menu_entry(item_name, checked) {
            Ok(i) => i,
            Err(e) => {
                return Err(e);
            }
        };
        let mut m = self.callback.borrow_mut();
        m.insert(idx, make_callback(f));
        Ok(idx)
    }

    pub fn clear_menu(&self) -> Result<(), SystrayError> {
        let mut idx = self.menu_idx.get();
        unsafe {
            while idx > 0 {
                if user32::DeleteMenu(self.info.hmenu,
                                      idx - 1,
                                      MF_BYPOSITION) == 0 {
                    return Err(get_win_os_error("Error clearing menu"));
                }
                idx = idx - 1;
            }
            self.menu_idx.set(0);
        }
        Ok(())
    }

    fn set_icon(&self, icon: HICON) -> Result<(), SystrayError> {
        unsafe {
            let mut nid = get_nid_struct(&self.info.hwnd);
            nid.uFlags = winapi::NIF_ICON;
            nid.hIcon = icon;
            if Shell_NotifyIconW(winapi::NIM_MODIFY,
                                          &mut nid as *mut NOTIFYICONDATAW) == 0 {
                return Err(get_win_os_error("Error setting icon"));
            }
        }
        Ok(())
    }

    pub fn wait_for_message(&mut self, blocking: bool) {
        loop {
            let msg;
            let ref rx_ref = self.rx;
            // Convert recv -> try_recv types
            let f: Box<Fn() -> Result<SystrayEvent, TryRecvError>> = match blocking {
                true => Box::new(|| { match rx_ref.recv() {
                    Ok(m) => Ok(m),
                    Err(_) => Err(TryRecvError::Disconnected),
                }}),
                false => Box::new(|| { rx_ref.try_recv() }),
            };
            match f() {
                Ok(m) => msg = m,
                Err(_) => {
                    if blocking {
                        // If self.rx fails, we're in thread shutdown. Join here.
                        if let Some(t) = self.windows_loop.take() {
                            t.join().ok();
                        }
                    }
                    break;
                }
            }
            match msg.action {
                SystrayAction::DisplayMenu => {
                    self.menu_displayed.set(true);
                },
                SystrayAction::HideMenu => {
                    self.menu_displayed.set(false);
                },
                SystrayAction::SelectItem => {
                    if (*self.callback.borrow()).contains_key(&msg.menu_index) {
                        let f = (*self.callback.borrow_mut()).remove(&msg.menu_index).unwrap();
                        f(&self);
                        (*self.callback.borrow_mut()).insert(msg.menu_index, f);
                    }
                },
            }
            if !blocking {
                break;
            }
        }
    }

    pub fn set_icon_from_resource(&self, resource_name: &String) -> Result<(), SystrayError> {
        let icon;
        unsafe {
            icon = user32::LoadImageW(self.info.hinstance,
                                      to_wstring(&resource_name).as_ptr(),
                                      winapi::IMAGE_ICON,
                                      64,
                                      64,
                                      0) as HICON;
            if icon == std::ptr::null_mut() as HICON {
                return Err(get_win_os_error("Error setting icon from resource"));
            }
        }
        self.set_icon(icon)
    }

    pub fn set_icon_from_file(&self, icon_file: &String) -> Result<(), SystrayError> {
        let wstr_icon_file = to_wstring(&icon_file);
        let hicon;
        unsafe {
            hicon = user32::LoadImageW(std::ptr::null_mut() as HINSTANCE, wstr_icon_file.as_ptr(),
                                       winapi::IMAGE_ICON, 64, 64, winapi::LR_LOADFROMFILE) as HICON;
            if hicon == std::ptr::null_mut() as HICON {
                return Err(get_win_os_error("Error setting icon from file"));
            }
        }
        self.set_icon(hicon)
    }

    pub fn set_icon_from_buffer(&self, buffer: &[u8], width: u32, height: u32) -> Result<(), SystrayError> {
        let offset = unsafe {
            user32::LookupIconIdFromDirectoryEx(
                buffer.as_ptr() as PBYTE,
                TRUE,
                width as i32,
                height as i32,
                LR_DEFAULTCOLOR
            )
        };

        if offset != 0 {
            let icon_data = &buffer[offset as usize ..];
            let hicon = unsafe {
                user32::CreateIconFromResourceEx(
                    icon_data.as_ptr() as PBYTE,
                    0,
                    TRUE,
                    0x30000,
                    width as i32,
                    height as i32,
                    LR_DEFAULTCOLOR
                )
            };

            if hicon == std::ptr::null_mut() as HICON {
                return Err( unsafe { get_win_os_error("Cannot load icon from the buffer") } );
            }

            self.set_icon(hicon)
        } else {
            Err( unsafe { get_win_os_error("Error setting icon from buffer") })
        }
    }

    pub fn menu_displayed(&self) -> bool {
        self.menu_displayed.get()
    }

    pub fn shutdown(&self) -> Result<(), SystrayError> {
        unsafe {
            let mut nid = get_nid_struct(&self.info.hwnd);
            nid.uFlags = winapi::NIF_ICON;
            if Shell_NotifyIconW(winapi::NIM_DELETE,
                                          &mut nid as *mut NOTIFYICONDATAW) == 0 {
                return Err(get_win_os_error("Error deleting icon from menu"));
            }
        }
        Ok(())
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        self.shutdown().ok();
    }
}
