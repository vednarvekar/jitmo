// Hide the console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Command bar shell.
//!
//! - Hosts Core on its own thread (`jitmo_core::service`) and forwards its
//!   events to the web view.
//! - Opens the bar from a global hotkey (SP11).
//! - Keeps the web view alive for a minute after the bar is hidden, then
//!   destroys it, so idle memory drops to a few MB (SP11) while a second
//!   request right after the first still opens in tens of milliseconds.
//!
//! Dev build: only `windows_system` (open an app) is real; the other
//! connectors are fakes, and the database is not encrypted yet
//! (see docs/03-delivery/8.1 - Decision Log.md).

use jitmo_connectors::{fake::FakeConnector, Connector};
use jitmo_core::context::{FsContext, SelectionSnapshot};
use jitmo_core::flow::Flow;
use jitmo_core::router::Router;
use jitmo_core::service::CoreService;
use jitmo_ipc::UiRequest;
use jitmo_model_worker::{WorkerClient, WorkerLauncher};
use jitmo_skills::Skill;
use jitmo_store::Store;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

const BAR: &str = "bar";
const BAR_WIDTH: f64 = 640.0;
const BAR_START_HEIGHT: f64 = 72.0;
const DEFAULT_HOTKEY: &str = "ctrl+alt+j"; // Ctrl+Alt+Space was already taken on the dev PC (SP11)
const DESTROY_AFTER_HIDDEN: Duration = Duration::from_secs(60);

struct AppState {
    core: CoreService,
    /// What Explorer had selected when the bar was last opened.
    selection: SelectionSnapshot,
    /// Bumped on every show and hide, so a pending "destroy" timer can tell it is stale.
    visibility: AtomicU64,
}

// --- commands the web view may call ---------------------------------------

#[tauri::command]
fn submit(state: State<AppState>, id: u64, text: String) {
    state.core.send(UiRequest::Submit { id, text });
}

#[tauri::command]
fn confirm(state: State<AppState>, id: u64, approved: bool) {
    state.core.send(UiRequest::Confirm { id, approved });
}

#[tauri::command]
fn choose(state: State<AppState>, id: u64, choice_id: String) {
    state.core.send(UiRequest::Choose { id, choice_id });
}

#[tauri::command]
fn cancel(state: State<AppState>, id: u64) {
    state.core.send(UiRequest::Cancel { id });
}

#[tauri::command]
fn undo(state: State<AppState>, id: u64) {
    state.core.send(UiRequest::Undo { id });
}

#[tauri::command]
fn hide_bar(app: AppHandle) {
    hide(&app);
}

#[tauri::command]
fn resize_bar(app: AppHandle, height: f64) {
    if let Some(w) = app.get_webview_window(BAR) {
        let _ = w.set_size(LogicalSize::new(
            BAR_WIDTH,
            height.clamp(BAR_START_HEIGHT, 600.0),
        ));
    }
}

// --- window handling ---------------------------------------------------------

fn create_bar(app: &AppHandle) {
    let built = WebviewWindowBuilder::new(app, BAR, WebviewUrl::App("index.html".into()))
        .title("jitmo")
        .inner_size(BAR_WIDTH, BAR_START_HEIGHT)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .center()
        .build();
    match built {
        Ok(w) => {
            let _ = w.set_focus();
        }
        Err(e) => eprintln!("could not open the command bar: {e}"),
    }
}

fn toggle(app: &AppHandle) {
    if !app
        .get_webview_window(BAR)
        .is_some_and(|w| w.is_visible().unwrap_or(false))
    {
        note_selection(app); // the bar is about to open: remember what is selected now
    }
    match app.get_webview_window(BAR) {
        Some(w) if w.is_visible().unwrap_or(false) => hide(app),
        Some(w) => {
            app.state::<AppState>()
                .visibility
                .fetch_add(1, Ordering::SeqCst); // cancels the destroy timer
            let _ = w.show();
            let _ = w.set_focus();
            let _ = app.emit("bar-shown", ());
        }
        None => create_bar(app),
    }
}

/// Remember the Explorer selection from before the bar takes focus. Clears the
/// old one at once; the new one arrives a moment later.
fn note_selection(app: &AppHandle) {
    let snapshot = app.state::<AppState>().selection.clone();
    snapshot.set(None);
    #[cfg(windows)]
    {
        let hwnd = jitmo_adapter_windows::explorer::foreground_window();
        std::thread::spawn(move || {
            snapshot.set(jitmo_adapter_windows::explorer::explorer_selection(hwnd));
        });
    }
}

fn hide(app: &AppHandle) {
    let Some(w) = app.get_webview_window(BAR) else {
        return;
    };
    let _ = w.hide();
    let mine = app
        .state::<AppState>()
        .visibility
        .fetch_add(1, Ordering::SeqCst)
        + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(DESTROY_AFTER_HIDDEN);
        if app.state::<AppState>().visibility.load(Ordering::SeqCst) == mine {
            if let Some(w) = app.get_webview_window(BAR) {
                let _ = w.destroy();
            }
        }
    });
}

// --- startup --------------------------------------------------------------------

/// Skills shipped with the app. Dev build: parsed without signature checks.
/// Rule-based routing, plus the model worker when its program sits next to the app.
fn router() -> Router {
    let router = Router::new(load_skills());
    let worker = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("jitmo-model-worker.exe")))
        .filter(|p| p.exists());
    match worker {
        Some(program) => router.with_worker(WorkerClient::new(
            WorkerLauncher {
                program,
                args: Vec::new(),
            },
            std::time::Duration::from_secs(10),
        )),
        None => router,
    }
}

fn load_skills() -> Vec<Skill> {
    [
        include_str!("../../../skills/apps.open.yaml"),
        include_str!("../../../skills/music.pause.yaml"),
        include_str!("../../../skills/stub.send.yaml"),
    ]
    .iter()
    .map(|y| Skill::parse(y).expect("built-in skill is invalid"))
    .collect()
}

/// `windows_system` is real (it opens apps). The rest are still fakes that
/// report success, until their connectors are built in Phase 2.
fn connectors() -> HashMap<String, Arc<dyn Connector>> {
    let mut map: HashMap<String, Arc<dyn Connector>> = ["spotify", "stub_messenger"]
        .iter()
        .map(|id| {
            (
                id.to_string(),
                Arc::new(FakeConnector::new(id)) as Arc<dyn Connector>,
            )
        })
        .collect();
    #[cfg(windows)]
    map.insert(
        "windows_system".into(),
        Arc::new(jitmo_connectors::windows_system::WindowsSystem),
    );
    #[cfg(not(windows))]
    map.insert(
        "windows_system".into(),
        Arc::new(FakeConnector::new("windows_system")),
    );
    map
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            submit, confirm, choose, cancel, undo, hide_bar, resize_bar
        ])
        .setup(|app| {
            // Store: next to the user's other app data.
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let store = Store::open(&dir.join("jitmo.db"), None)?;
            let hotkey = store
                .get_setting("hotkey")?
                .unwrap_or_else(|| DEFAULT_HOTKEY.to_string());

            let selection = SelectionSnapshot::default();
            let flow = Flow::new(router(), store, connectors())
                .with_context(FsContext::for_current_user(selection.clone()));
            let handle = app.handle().clone();
            let core = CoreService::spawn(flow, move |event| {
                let _ = handle.emit("core-event", &event);
            });
            app.manage(AppState {
                core,
                selection,
                visibility: AtomicU64::new(0),
            });

            // Hotkey. If it cannot be registered (another app owns it), open the
            // bar right away so the user is never left with nothing on screen.
            let registered = Shortcut::from_str(&hotkey)
                .map_err(|e| e.to_string())
                .and_then(|sc| {
                    app.global_shortcut()
                        .on_shortcut(sc, |app, _sc, event| {
                            if event.state() == ShortcutState::Pressed {
                                toggle(app);
                            }
                        })
                        .map_err(|e| e.to_string())
                });
            if let Err(e) = registered {
                eprintln!("could not register the hotkey `{hotkey}`: {e}");
                create_bar(app.handle());
            } else if std::env::var_os("JITMO_OPEN_ON_START").is_some() {
                // For measuring and demos: show the bar without pressing the hotkey.
                create_bar(app.handle());
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build the app")
        .run(|_app, event| {
            // No window is normal (the web view is destroyed when idle). Keep running.
            if let RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
