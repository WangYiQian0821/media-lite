use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::UI::{Shell::*, WindowsAndMessaging::*};

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

pub fn pick_file(owner: Option<isize>, title: &str, patterns: &str, filename: Option<&str>) -> Option<PathBuf> {
    use windows_sys::Win32::UI::Controls::Dialogs::*;
    let label = wide(title);
    let filter = wide(&format!("{title}\0{patterns}\0"));
    let extension = filename.and_then(|name| name.rsplit_once('.').map(|(_, ext)| wide(ext)));
    let mut file = vec![0u16; 32768];
    if let Some(filename) = filename {
        let default = wide(filename);
        file[..default.len()].copy_from_slice(&default);
    }
    let save = filename.is_some();
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner.unwrap_or(0) as HWND,
        lpstrFilter: filter.as_ptr(),
        lpstrFile: file.as_mut_ptr(),
        nMaxFile: file.len() as u32,
        lpstrTitle: label.as_ptr(),
        lpstrDefExt: extension.as_ref().map_or(null(), |ext| ext.as_ptr()),
        Flags: OFN_EXPLORER | OFN_NOCHANGEDIR | OFN_PATHMUSTEXIST |
            if save { OFN_OVERWRITEPROMPT } else { OFN_FILEMUSTEXIST },
        ..Default::default()
    };
    let ok = unsafe { if save { GetSaveFileNameW(&mut dialog) } else { GetOpenFileNameW(&mut dialog) } };
    if ok == 0 {
        return None;
    }
    let end = file.iter().position(|&word| word == 0)?;
    Some(OsString::from_wide(&file[..end]).into())
}

pub fn pick_folder(owner: Option<isize>, title: &str) -> Option<PathBuf> {
    let title = wide(title);
    let mut file = [0u16; 260];
    let info = BROWSEINFOW {
        hwndOwner: owner.unwrap_or(0) as HWND,
        pszDisplayName: file.as_mut_ptr(),
        lpszTitle: title.as_ptr(),
        ulFlags: BIF_RETURNONLYFSDIRS,
        ..Default::default()
    };
    unsafe {
        let item = SHBrowseForFolderW(&info);
        if item.is_null() {
            return None;
        }
        let ok = SHGetPathFromIDListW(item, file.as_mut_ptr());
        windows_sys::Win32::System::Com::CoTaskMemFree(item.cast());
        if ok == 0 {
            return None;
        }
    }
    let end = file.iter().position(|&word| word == 0)?;
    Some(OsString::from_wide(&file[..end]).into())
}

pub fn open_in_shell(path: &Path) -> std::io::Result<()> {
    let value: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let operation = wide("open");
    let rc = unsafe { ShellExecuteW(null_mut(), operation.as_ptr(), value.as_ptr(), null(), null(), SW_SHOWNORMAL) };
    if rc as isize > 32 { Ok(()) } else { Err(std::io::Error::other("Windows could not open the path")) }
}

static INHERITED_HTTP_PROXY: OnceLock<Option<OsString>> = OnceLock::new();

pub fn capture_http_proxy() {
    INHERITED_HTTP_PROXY.get_or_init(|| {
        // libmpv reads this variable as a fallback. Route its requests explicitly.
        let value = std::env::var_os("HTTP_PROXY");
        std::env::remove_var("HTTP_PROXY");
        value
    });
}

pub fn inherited_http_proxy() -> Option<&'static OsStr> {
    INHERITED_HTTP_PROXY.get().and_then(Option::as_deref)
}

pub fn system_proxy(https: bool) -> Option<String> {
    let proxy = if https {
        std::env::var("HTTPS_PROXY").ok()
    } else {
        inherited_http_proxy().and_then(OsStr::to_str).map(str::to_owned)
    };
    if let Some(proxy) = proxy.filter(|value| !value.trim().is_empty())
        .or_else(|| std::env::var("ALL_PROXY").ok().filter(|value| !value.trim().is_empty()))
    {
        return Some(if proxy.contains("://") { proxy } else { format!("http://{proxy}") });
    }
    use windows_sys::Win32::System::Registry::*;
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings");
    let mut enabled: u32 = 0;
    let mut size: u32 = 4;
    let mut server = [0u16; 2048];
    let ok = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), wide("ProxyEnable").as_ptr(), RRF_RT_REG_DWORD,
            null_mut(), std::ptr::from_mut(&mut enabled).cast(), &mut size)
    };
    if ok != 0 || enabled == 0 {
        return None;
    }
    size = std::mem::size_of_val(&server) as u32;
    if unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), wide("ProxyServer").as_ptr(), RRF_RT_REG_SZ,
        null_mut(), server.as_mut_ptr().cast(), &mut size) } != 0
    {
        return None;
    }
    let end = server.iter().position(|&word| word == 0)?;
    let server = String::from_utf16_lossy(&server[..end]);
    let value = if server.contains('=') {
        let scheme = if https { "https" } else { "http" };
        server.split(';').filter_map(|pair| pair.split_once('='))
            .find(|(key, _)| key.eq_ignore_ascii_case(scheme))?.1.trim()
    } else {
        server.trim()
    };
    if value.is_empty() { None } else { Some(if value.contains("://") { value.to_owned() } else { format!("http://{value}") }) }
}

const TRAY_MESSAGE: u32 = WM_APP + 37;
static TRAY_EXIT: AtomicBool = AtomicBool::new(false);

pub struct Tray { hwnd: HWND }

impl Tray {
    pub fn hide(owner: Option<isize>) -> Option<Self> {
        let hwnd = owner? as HWND;
        let mut info = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: TRAY_MESSAGE,
            hIcon: unsafe { LoadIconW(null_mut(), IDI_APPLICATION) },
            ..Default::default()
        };
        let name = wide("Media Lite");
        info.szTip[..name.len()].copy_from_slice(&name);
        let tray = Self { hwnd };
        unsafe {
            if SetWindowSubclass(hwnd, Some(tray_event), 1, 0) == 0 || Shell_NotifyIconW(NIM_ADD, &info) == 0 {
                return None;
            }
            ShowWindow(hwnd, SW_HIDE);
        }
        Some(tray)
    }

    pub fn exiting() -> bool {
        TRAY_EXIT.load(Ordering::Relaxed)
    }

    pub fn restored(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd) != 0 }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let info = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            ..Default::default()
        };
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &info);
            RemoveWindowSubclass(self.hwnd, Some(tray_event), 1);
        }
    }
}

unsafe extern "system" fn tray_event(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
    unsafe {
        if msg == TRAY_MESSAGE {
            let chosen = match lp as u32 {
                WM_LBUTTONUP => 1,
                WM_RBUTTONUP => {
                    let menu = CreatePopupMenu();
                    let show = wide(&crate::i18n::bilingual("Open Media Lite", "打开 Media Lite"));
                    let quit = wide(&crate::i18n::bilingual("Quit", "退出"));
                    AppendMenuW(menu, MF_STRING, 1, show.as_ptr());
                    AppendMenuW(menu, MF_STRING, 2, quit.as_ptr());
                    let mut point = POINT::default();
                    GetCursorPos(&mut point);
                    SetForegroundWindow(hwnd);
                    let chosen = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON,
                        point.x, point.y, 0, hwnd, null());
                    DestroyMenu(menu);
                    chosen
                }
                _ => 0,
            };
            if chosen == 1 {
                ShowWindow(hwnd, SW_RESTORE);
                SetForegroundWindow(hwnd);
            } else if chosen == 2 {
                TRAY_EXIT.store(true, Ordering::Relaxed);
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
            }
            return 0;
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}
