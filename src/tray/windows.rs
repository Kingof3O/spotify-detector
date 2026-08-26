use std::cell::RefCell;
use std::mem::size_of;
use std::thread::{self, JoinHandle};

use tokio::sync::watch;
use windows::core::{w, Error, Result, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    ShellExecuteW, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
    NIM_SETVERSION, NIN_SELECT, NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW, PostMessageW, PostQuitMessage,
    RegisterClassW, SetForegroundWindow, TrackPopupMenu, TranslateMessage, HMENU, IDI_APPLICATION,
    MF_SEPARATOR, MF_STRING, MSG, SW_SHOWNORMAL, TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_APP, WM_COMMAND, WM_CONTEXTMENU, WM_DESTROY, WM_LBUTTONDBLCLK, WM_NULL, WM_RBUTTONUP,
    WNDCLASSW,
};

const TRAY_ICON_ID: u32 = 1;
const TRAY_CALLBACK_MESSAGE: u32 = WM_APP + 1;
const MENU_OPEN_OVERLAY: usize = 1_001;
const MENU_STOP: usize = 1_002;
const MENU_SETUP: usize = 1_003;

thread_local! {
    static CONTEXT: RefCell<Option<TrayContext>> = const { RefCell::new(None) };
}

struct TrayContext {
    menu: HMENU,
    overlay_url: Vec<u16>,
    setup_url: Vec<u16>,
    shutdown_tx: watch::Sender<bool>,
}

pub fn spawn(
    overlay_url: String,
    setup_url: String,
    shutdown_tx: watch::Sender<bool>,
) -> Option<JoinHandle<()>> {
    match thread::Builder::new()
        .name("spotify-overlay-tray".to_owned())
        .stack_size(256 * 1024)
        .spawn(move || {
            if let Err(error) = run(&overlay_url, &setup_url, shutdown_tx) {
                tracing::warn!(?error, "notification-area icon could not be started");
            }
        }) {
        Ok(handle) => Some(handle),
        Err(error) => {
            tracing::warn!(?error, "notification-area thread could not be started");
            None
        }
    }
}

fn run(overlay_url: &str, setup_url: &str, shutdown_tx: watch::Sender<bool>) -> Result<()> {
    // The hidden window receives shell callbacks while the thread sleeps in GetMessageW.
    let class_name = wide("SpotifyOverlayTrayWindow");
    let instance = unsafe { GetModuleHandleW(PCWSTR::null()) }?;
    let instance: HINSTANCE = instance.into();
    let icon = unsafe { LoadIconW(None, IDI_APPLICATION) }?;

    let window_class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };

    if unsafe { RegisterClassW(&window_class) } == 0 {
        return Err(Error::from_thread());
    }

    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class_name.as_ptr()),
            w!("Spotify OBS Overlay"),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )
    }?;

    let menu = match create_menu() {
        Ok(menu) => menu,
        Err(error) => {
            let _ = unsafe { DestroyWindow(hwnd) };
            return Err(error);
        }
    };

    CONTEXT.with(|slot| {
        slot.replace(Some(TrayContext {
            menu,
            overlay_url: wide(overlay_url),
            setup_url: wide(setup_url),
            shutdown_tx,
        }));
    });

    let mut icon_data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ICON_ID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: TRAY_CALLBACK_MESSAGE,
        hIcon: icon,
        ..Default::default()
    };
    copy_wide(&mut icon_data.szTip, "Spotify OBS Overlay");

    if !unsafe { Shell_NotifyIconW(NIM_ADD, &icon_data) }.as_bool() {
        cleanup(hwnd, menu, None);
        return Err(Error::from_thread());
    }

    icon_data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    let _ = unsafe { Shell_NotifyIconW(NIM_SETVERSION, &icon_data) };

    tracing::info!("notification-area icon ready");
    let mut message = MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
        if result == -1 {
            cleanup(hwnd, menu, Some(&icon_data));
            return Err(Error::from_thread());
        }
        if result == 0 {
            break;
        }

        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    cleanup(hwnd, menu, Some(&icon_data));
    Ok(())
}

fn create_menu() -> Result<HMENU> {
    let menu = unsafe { CreatePopupMenu() }?;
    let result = unsafe {
        AppendMenuW(menu, MF_STRING, MENU_OPEN_OVERLAY, w!("Open overlay"))
            .and_then(|_| AppendMenuW(menu, MF_STRING, MENU_SETUP, w!("Open Stream Manager Setup")))
            .and_then(|_| AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()))
            .and_then(|_| AppendMenuW(menu, MF_STRING, MENU_STOP, w!("Stop Spotify Overlay")))
    };

    if let Err(error) = result {
        let _ = unsafe { DestroyMenu(menu) };
        return Err(error);
    }

    Ok(menu)
}

fn cleanup(hwnd: HWND, menu: HMENU, icon_data: Option<&NOTIFYICONDATAW>) {
    if let Some(icon_data) = icon_data {
        let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, icon_data) };
    }
    CONTEXT.with(|slot| {
        slot.take();
    });
    let _ = unsafe { DestroyMenu(menu) };
    let _ = unsafe { DestroyWindow(hwnd) };
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        TRAY_CALLBACK_MESSAGE => {
            let notification = (lparam.0 as u32) & 0xffff;
            match notification {
                WM_CONTEXTMENU | WM_RBUTTONUP => show_context_menu(hwnd),
                WM_LBUTTONDBLCLK | NIN_SELECT => open_overlay(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            match wparam.0 & 0xffff {
                MENU_OPEN_OVERLAY => open_overlay(hwnd),
                MENU_SETUP => open_setup(hwnd),
                MENU_STOP => {
                    CONTEXT.with(|slot| {
                        if let Some(context) = slot.borrow().as_ref() {
                            let _ = context.shutdown_tx.send(true);
                        }
                    });
                    let _ = unsafe { DestroyWindow(hwnd) };
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn show_context_menu(hwnd: HWND) {
    let menu = CONTEXT.with(|slot| slot.borrow().as_ref().map(|context| context.menu));
    let Some(menu) = menu else {
        return;
    };

    let mut cursor = POINT::default();
    if unsafe { GetCursorPos(&mut cursor) }.is_err() {
        return;
    }

    unsafe {
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, cursor.x, cursor.y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
    }
}

fn open_overlay(hwnd: HWND) {
    let overlay_url = CONTEXT.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|context| context.overlay_url.clone())
    });
    let Some(overlay_url) = overlay_url else {
        return;
    };

    unsafe {
        let _ = ShellExecuteW(
            Some(hwnd),
            w!("open"),
            PCWSTR(overlay_url.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

fn open_setup(hwnd: HWND) {
    let setup_url = CONTEXT.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|context| context.setup_url.clone())
    });
    let Some(setup_url) = setup_url else {
        return;
    };

    unsafe {
        let _ = ShellExecuteW(
            Some(hwnd),
            w!("open"),
            PCWSTR(setup_url.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn copy_wide<const N: usize>(destination: &mut [u16; N], value: &str) {
    for (output, input) in destination
        .iter_mut()
        .zip(value.encode_utf16().take(N.saturating_sub(1)))
    {
        *output = input;
    }
}
