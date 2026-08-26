#[cfg(not(windows))]
use tokio::sync::mpsc::UnboundedSender;

#[cfg(not(windows))]
use crate::config::LeagueConfig;

use super::{LeagueObservation, WindowSignal};

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_impl {
    use std::{cell::RefCell, collections::HashMap, path::Path, thread::JoinHandle};

    use tokio::sync::mpsc::UnboundedSender;
    use windows::{
        core::{BOOL, PWSTR},
        Win32::{
            Foundation::{CloseHandle, HWND, LPARAM},
            System::Threading::{
                OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
            UI::{
                Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK},
                WindowsAndMessaging::{
                    EnumWindows, GetClassNameW, GetForegroundWindow, GetMessageW, GetWindowTextW,
                    GetWindowThreadProcessId, IsWindowVisible, CHILDID_SELF, EVENT_OBJECT_CLOAKED,
                    EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE,
                    EVENT_OBJECT_SHOW, EVENT_SYSTEM_FOREGROUND, MSG, OBJID_WINDOW,
                    WINEVENT_OUTOFCONTEXT,
                },
            },
        },
    };

    use crate::config::LeagueConfig;

    use super::{LeagueObservation, WindowSignal};
    use crate::automation::WindowSignalKind;

    thread_local! {
        static SIGNAL_TX: RefCell<Option<UnboundedSender<WindowSignal>>> = const { RefCell::new(None) };
    }

    const HOOK_EVENTS: [u32; 6] = [
        EVENT_OBJECT_CREATE,
        EVENT_OBJECT_DESTROY,
        EVENT_OBJECT_SHOW,
        EVENT_OBJECT_HIDE,
        EVENT_OBJECT_CLOAKED,
        EVENT_SYSTEM_FOREGROUND,
    ];

    pub fn spawn(signal_tx: UnboundedSender<WindowSignal>) -> Option<JoinHandle<()>> {
        match std::thread::Builder::new()
            .name("league-window-monitor".to_owned())
            .stack_size(256 * 1024)
            .spawn(move || run(signal_tx))
        {
            Ok(handle) => Some(handle),
            Err(error) => {
                tracing::warn!(?error, "could not start League window monitor thread");
                None
            }
        }
    }

    fn run(signal_tx: UnboundedSender<WindowSignal>) {
        SIGNAL_TX.with(|slot| slot.replace(Some(signal_tx)));

        let mut hooks = Vec::with_capacity(HOOK_EVENTS.len());
        for event in HOOK_EVENTS {
            let hook = unsafe {
                SetWinEventHook(
                    event,
                    event,
                    None,
                    Some(win_event_proc),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                )
            };
            if hook.is_invalid() {
                tracing::warn!(event, "could not install League WinEvent hook");
            } else {
                hooks.push(hook);
            }
        }

        if hooks.is_empty() {
            tracing::error!("League window monitor has no active WinEvent hooks");
            SIGNAL_TX.with(|slot| slot.replace(None));
            return;
        }

        tracing::info!(hooks = hooks.len(), "League window monitor connected");
        let mut message = MSG::default();
        loop {
            let result = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
            if result == -1 {
                tracing::warn!("League window monitor message loop failed");
                break;
            }
            if result == 0 {
                break;
            }
        }

        for hook in hooks {
            unsafe {
                let _ = UnhookWinEvent(hook);
            }
        }
        SIGNAL_TX.with(|slot| slot.replace(None));
        tracing::info!("League window monitor stopped");
    }

    unsafe extern "system" fn win_event_proc(
        _hook: HWINEVENTHOOK,
        event: u32,
        hwnd: HWND,
        id_object: i32,
        id_child: i32,
        _event_thread: u32,
        _event_time: u32,
    ) {
        if id_object != OBJID_WINDOW.0 || id_child != CHILDID_SELF as i32 {
            return;
        }

        let Some(kind) = WindowSignalKind::from_event(event) else {
            return;
        };
        SIGNAL_TX.with(|slot| {
            if let Some(sender) = slot.borrow().as_ref() {
                let _ = sender.send(WindowSignal {
                    kind,
                    hwnd: hwnd.0 as usize,
                });
            }
        });
    }

    struct SnapshotContext<'a> {
        config: &'a LeagueConfig,
        observation: LeagueObservation,
        foreground: HWND,
        process_names: HashMap<u32, Option<String>>,
    }

    pub fn snapshot(config: &LeagueConfig) -> LeagueObservation {
        let mut context = SnapshotContext {
            config,
            observation: LeagueObservation::default(),
            foreground: unsafe { GetForegroundWindow() },
            process_names: HashMap::new(),
        };
        let pointer = LPARAM((&mut context as *mut SnapshotContext<'_>) as isize);
        let _ = unsafe { EnumWindows(Some(enum_window_proc), pointer) };
        context.observation
    }

    unsafe extern "system" fn enum_window_proc(hwnd: HWND, parameter: LPARAM) -> BOOL {
        let context = &mut *(parameter.0 as *mut SnapshotContext<'_>);
        if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
            return BOOL(1);
        }

        let mut process_id = 0_u32;
        if unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) } == 0 {
            return BOOL(1);
        }
        let process_name = context
            .process_names
            .entry(process_id)
            .or_insert_with(|| process_name(process_id))
            .clone()
            .unwrap_or_default();
        let class_name = window_class(hwnd);
        let title = window_title(hwnd);
        let foreground = hwnd.0 == context.foreground.0;

        let is_game = matches_signature(&process_name, &context.config.game_process_names);
        let is_client_process =
            matches_signature(&process_name, &context.config.client_process_names);
        let is_client_window = is_client_process
            && matches_optional_signature(&class_name, &context.config.client_window_classes)
            && matches_optional_signature(&title, &context.config.client_window_title_patterns);

        if is_game && (!context.config.require_foreground || foreground) {
            context.observation.game = true;
        }
        if is_client_window && (!context.config.require_foreground || foreground) {
            context.observation.client = true;
        }
        if is_game && foreground {
            context.observation.game_foreground = true;
        }
        if is_client_window && foreground {
            context.observation.client_foreground = true;
        }
        BOOL(1)
    }

    fn matches_signature(value: &str, signatures: &[String]) -> bool {
        signatures.iter().any(|signature| {
            value.eq_ignore_ascii_case(signature.trim())
                || Path::new(value)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case(signature.trim()))
        })
    }

    fn matches_optional_signature(value: &str, signatures: &[String]) -> bool {
        signatures.is_empty()
            || signatures.iter().any(|signature| {
                value
                    .to_ascii_lowercase()
                    .contains(&signature.trim().to_ascii_lowercase())
            })
    }

    fn process_name(process_id: u32) -> Option<String> {
        let handle =
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }.ok()?;
        let mut buffer = [0_u16; 512];
        let mut length = buffer.len() as u32;
        let result = unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR(buffer.as_mut_ptr()),
                &mut length,
            )
        };
        unsafe {
            let _ = CloseHandle(handle);
        }
        result.ok()?;
        String::from_utf16(&buffer[..length as usize]).ok()
    }

    fn window_class(hwnd: HWND) -> String {
        let mut buffer = [0_u16; 256];
        let length = unsafe { GetClassNameW(hwnd, &mut buffer) };
        String::from_utf16_lossy(&buffer[..length.max(0) as usize])
    }

    fn window_title(hwnd: HWND) -> String {
        let mut buffer = [0_u16; 512];
        let length = unsafe { GetWindowTextW(hwnd, &mut buffer) };
        String::from_utf16_lossy(&buffer[..length.max(0) as usize])
    }
}

#[cfg(windows)]
pub use windows_impl::{snapshot, spawn};

#[cfg(not(windows))]
pub fn spawn(_signal_tx: UnboundedSender<WindowSignal>) -> Option<std::thread::JoinHandle<()>> {
    tracing::debug!("League window monitor is disabled on this platform");
    None
}

#[cfg(not(windows))]
pub fn snapshot(_config: &LeagueConfig) -> LeagueObservation {
    LeagueObservation::default()
}
