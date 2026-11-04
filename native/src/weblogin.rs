//! Sign-in through the real discord.com page, in a window of its own (the system's WebView2).
//! The person logs in there (password, or the QR code the page itself shows, and any captcha);
//! the injected script hands the token back by navigating to a fake host, which we block, so
//! the token never goes over the network. Nothing from the page can reach Nimbus.
//!
//! The window is a plain Win32 one on its own thread with its own message loop: the app's
//! main window library (winit) is not shared with it.

use std::cell::RefCell;
use std::num::NonZeroIsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};
use windows::core::{w, HSTRING};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW, LoadCursorW, PostMessageW,
    PostQuitMessage, RegisterClassW, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, IDC_ARROW, MSG, SW_SHOW,
    WM_CLOSE, WM_DESTROY, WM_SIZE, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};
use wry::dpi::{PhysicalPosition, PhysicalSize};
use wry::{Rect, WebView, WebViewBuilder};

const SCRIPT: &str = include_str!("web_login.js");
const CALLBACK: &str = "https://nimbus.callback/#";

static OPEN: AtomicBool = AtomicBool::new(false);

thread_local! {
    static VIEW: RefCell<Option<WebView>> = const { RefCell::new(None) };
}

/// Opens the sign-in window (one at a time) and calls `done` with the token, or why there isn't one.
/// The window is wide on purpose: a narrow one hides the QR code from the login page.
pub fn open(done: impl FnOnce(Result<String, String>) + Send + 'static) {
    if OPEN.swap(true, Ordering::SeqCst) {
        return;
    }
    let spawned = std::thread::Builder::new().name("web-login".into()).spawn(move || {
        let result = run();
        OPEN.store(false, Ordering::SeqCst);
        done(result);
    });
    if spawned.is_err() {
        OPEN.store(false, Ordering::SeqCst);
    }
}

struct Host(isize);

impl HasWindowHandle for Host {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let handle = NonZeroIsize::new(self.0).ok_or(HandleError::Unavailable)?;
        let raw = RawWindowHandle::Win32(Win32WindowHandle::new(handle));
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

fn fit(hwnd: HWND) {
    let mut area = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut area) }.is_err() {
        return;
    }
    VIEW.with(|view| {
        if let Some(view) = view.borrow().as_ref() {
            let _ = view.set_bounds(Rect {
                position: PhysicalPosition::new(0, 0).into(),
                size: PhysicalSize::new((area.right - area.left).max(1) as u32, (area.bottom - area.top).max(1) as u32).into(),
            });
        }
    });
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_SIZE => {
            fit(hwnd);
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

fn run() -> Result<String, String> {
    let instance = unsafe { GetModuleHandleW(None) }.map_err(|e| e.to_string())?;
    let class = w!("NimbusWebLogin");
    let class_def = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(window_proc),
        hInstance: instance.into(),
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
        lpszClassName: class,
        ..Default::default()
    };
    // Registering twice just fails the second time, which is fine: the class is already there.
    unsafe { RegisterClassW(&class_def) };
    let hwnd = unsafe {
        CreateWindowExW(
            Default::default(),
            class,
            &HSTRING::from(crate::i18n::t("Вход через discord.com")),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1040,
            800,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }
    .map_err(|e| e.to_string())?;

    let token: Arc<Mutex<Option<String>>> = Arc::default();
    let sink = token.clone();
    let closer = hwnd.0 as isize;
    let view = WebViewBuilder::new()
        // Nothing of the session stays on disk once the window is closed.
        .with_incognito(true)
        .with_url("https://discord.com/login")
        .with_initialization_script(SCRIPT)
        .with_navigation_handler(move |url| {
            let Some(found) = url.strip_prefix(CALLBACK) else { return true };
            if !found.is_empty() {
                *sink.lock().unwrap() = Some(found.to_string());
            }
            // Close the window from its own message loop.
            let _ = unsafe { PostMessageW(Some(HWND(closer as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0)) };
            false
        })
        .build(&Host(hwnd.0 as isize))
        .map_err(|e| {
            let _ = unsafe { DestroyWindow(hwnd) };
            crate::i18n::tf("не удалось открыть окно входа: {}", &[&e])
        })?;
    VIEW.with(|slot| *slot.borrow_mut() = Some(view));
    fit(hwnd);
    let _ = unsafe { ShowWindow(hwnd, SW_SHOW) };

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    VIEW.with(|slot| slot.borrow_mut().take());
    let found = token.lock().unwrap().take();
    found.ok_or_else(|| crate::i18n::t("окно входа закрыто без входа"))
}
