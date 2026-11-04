//! Things that belong to Windows, not to Discord: one running copy, the tray icon,
//! and (only when the person turns it on) starting with Windows.

use std::os::windows::process::CommandExt;
use std::process::Command;

use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "NimbusNative";

/// True when this is the only running copy. The mutex lives until the process ends.
pub fn first_instance() -> bool {
    unsafe {
        let handle = CreateMutexW(None, true, w!("Local\\NimbusNativeSingleInstance"));
        let already = GetLastError() == ERROR_ALREADY_EXISTS;
        // Kept open on purpose: closing it would let a second copy start.
        std::mem::forget(handle);
        !already
    }
}

fn reg(args: &[&str]) -> Option<String> {
    let out = Command::new("reg").args(args).creation_flags(CREATE_NO_WINDOW).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Whether Nimbus starts with Windows (a read; nothing is changed).
pub fn autostart_enabled() -> bool {
    reg(&["query", RUN_KEY, "/v", RUN_VALUE]).is_some()
}

/// Turns starting with Windows on or off. Only called when the person flips the switch.
pub fn set_autostart(on: bool) -> bool {
    if on {
        let Ok(exe) = std::env::current_exe() else { return false };
        let command = format!("\"{}\"", exe.display());
        reg(&["add", RUN_KEY, "/v", RUN_VALUE, "/t", "REG_SZ", "/d", &command, "/f"]).is_some()
    } else {
        // Nothing to remove is also fine.
        reg(&["delete", RUN_KEY, "/v", RUN_VALUE, "/f"]);
        true
    }
}

/// Asks Windows 11 to round this window's corners (Windows 10 keeps them square, which also looks right).
pub fn round_corners(hwnd: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
    let round = DWMWCP_ROUND;
    unsafe {
        let _ = DwmSetWindowAttribute(HWND(hwnd as *mut _), DWMWA_WINDOW_CORNER_PREFERENCE, &round as *const _ as *const _, std::mem::size_of_val(&round) as u32);
    }
}

/// Keeps this window out of screen captures (`SetWindowDisplayAffinity`, Windows 10 2004 and later) while
/// a monitor is being shown: no mirror of Nimbus inside the stream when you look at your own share, and the
/// chats stay off the stream. Affects only the picture of this one window; no Windows setting changes.
pub fn exclude_from_capture(hwnd: isize, on: bool) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE};
    let affinity = if on { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
    unsafe {
        if let Err(e) = SetWindowDisplayAffinity(HWND(hwnd as *mut _), affinity) {
            crate::log_line(&format!("window capture exclusion: {e}"));
        }
    }
}
/// Puts a red number on the window's taskbar button (`count` mentions), or takes it off (0).
/// `hwnd` is the window's handle. Done only from the UI thread; nothing else on the system is touched.
pub fn set_overlay(hwnd: isize, count: u32) {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{ITaskbarList3, TaskbarList};
    use windows::Win32::UI::WindowsAndMessaging::{CreateIconFromResourceEx, DestroyIcon, HICON, LR_DEFAULTCOLOR};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let Ok(list) = CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None, CLSCTX_ALL) else { return };
        let _ = list.HrInit();
        let hwnd = HWND(hwnd as *mut _);
        if count == 0 {
            let _ = list.SetOverlayIcon(hwnd, HICON::default(), &HSTRING::new());
            return;
        }
        let png = crate::badge::png(count);
        let Ok(icon) = CreateIconFromResourceEx(&png, true, 0x0003_0000, 32, 32, LR_DEFAULTCOLOR) else { return };
        let _ = list.SetOverlayIcon(hwnd, icon, &HSTRING::from(crate::i18n::tf("Упоминаний: {}", &[&count])));
        let _ = DestroyIcon(icon);
    }
}