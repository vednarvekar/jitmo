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
use jitmo_core::service::serve_connection;
use jitmo_ipc::UiRequest;
use jitmo_model_worker::{WorkerClient, WorkerLauncher};
use jitmo_skills::Skill;
use jitmo_store::Store;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::AtomicU64 as OwnMoveClock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, RunEvent, State, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

mod core_client;
use core_client::CoreClient;

const BAR: &str = "bar";
/// Window size in logical pixels. The window is larger than the pill: the extra
/// 14 px on every side is transparent and holds the pill's shadow (see `.shell` in styles.css).
const BAR_WIDTH: f64 = 580.0;
const BAR_START_HEIGHT: f64 = 80.0;
/// Gap between the pill's shadow margin and the taskbar, in logical pixels.
const TASKBAR_GAP: f64 = 0.0;
/// Ignore "moved" events this long after the app moved the window itself.
const OWN_MOVE_QUIET_MS: u64 = 400;
const DEFAULT_HOTKEY: &str = "ctrl+alt+j"; // Ctrl+Alt+Space was already taken on the dev PC (SP11)
const DESTROY_AFTER_HIDDEN: Duration = Duration::from_secs(60);

/// When the app last moved the bar itself (ms since the epoch), so those moves
/// are not mistaken for the user dragging it.
static LAST_OWN_MOVE: OwnMoveClock = OwnMoveClock::new(0);
/// When the user last dragged the bar. Dragging makes the window lose focus for a
/// moment, which must not be taken for a click elsewhere.
static LAST_USER_MOVE: OwnMoveClock = OwnMoveClock::new(0);
/// A focus loss this soon after a drag is part of the drag.
const DRAG_BLUR_QUIET_MS: u64 = 1500;

struct AppState {
    core: CoreClient,
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

/// Tell the page it is on screen so it can play the pop-up animation. Wait a moment
/// first: right after the window appears the web view is still painting its first
/// frame, and an animation that starts at that instant stutters.
fn announce_shown(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(70));
        let _ = app.emit("bar-shown", ());
    });
}

/// The page has drawn its first frame: now it is safe to show the window.
#[tauri::command]
fn bar_ready(app: AppHandle) {
    show_when_ready(&app);
}

fn show_when_ready(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(BAR) {
        if !w.is_visible().unwrap_or(false) {
            place_bar(app, &w);
            let _ = w.show();
            let _ = w.set_focus();
            announce_shown(app);
        }
    }
}

#[tauri::command]
fn hide_bar(app: AppHandle) {
    hide(&app);
}

#[tauri::command]
fn resize_bar(app: AppHandle, height: f64) {
    if let Some(w) = app.get_webview_window(BAR) {
        resize_keeping_anchor(&w, height.clamp(BAR_START_HEIGHT, 700.0));
    }
}

// --- where the bar sits --------------------------------------------------------

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Move the window ourselves and remember that we did.
fn move_to(w: &WebviewWindow, x: i32, y: i32) {
    LAST_OWN_MOVE.store(now_ms(), Ordering::SeqCst);
    let _ = w.set_position(PhysicalPosition::new(x, y));
}

fn position_file(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join("bar-position.json"))
}

/// The saved spot is the window's left edge and its BOTTOM edge, in physical
/// pixels, because the bar grows upward from the taskbar.
fn load_saved(app: &AppHandle) -> Option<(i32, i32)> {
    let text = std::fs::read_to_string(position_file(app)?).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some((
        v.get("x")?.as_i64()? as i32,
        v.get("bottom")?.as_i64()? as i32,
    ))
}

fn save_position(app: &AppHandle, w: &WebviewWindow) {
    let (Some(file), Ok(pos), Ok(size)) = (position_file(app), w.outer_position(), w.outer_size())
    else {
        return;
    };
    let json = serde_json::json!({ "x": pos.x, "bottom": pos.y + size.height as i32 });
    let _ = std::fs::write(file, json.to_string());
}

/// The work area (the screen minus the taskbar) of the monitor that holds this
/// point, if any monitor does.
fn work_area_at(app: &AppHandle, x: i32, y: i32) -> Option<tauri::PhysicalRect<i32, u32>> {
    let monitor = app.monitor_from_point(x as f64, y as f64).ok().flatten()?;
    Some(*monitor.work_area())
}

/// Put the bar where it belongs: the spot the user dragged it to if that is still
/// on a screen, otherwise centered just above the taskbar on the monitor with
/// the mouse pointer.
fn place_bar(app: &AppHandle, w: &WebviewWindow) {
    let Ok(size) = w.outer_size() else { return };
    let (width, height) = (size.width as i32, size.height as i32);
    if let Some((x, bottom)) = load_saved(app) {
        // Still visible on some monitor? Check the window's middle.
        if work_area_at(app, x + width / 2, bottom - height / 2).is_some() {
            move_to(w, x, bottom - height);
            return;
        }
    }
    let cursor = app.cursor_position().ok();
    let area = cursor
        .and_then(|c| work_area_at(app, c.x as i32, c.y as i32))
        .or_else(|| app.primary_monitor().ok().flatten().map(|m| *m.work_area()));
    let Some(area) = area else { return };
    let scale = w.scale_factor().unwrap_or(1.0);
    let gap = (TASKBAR_GAP * scale) as i32;
    let x = area.position.x + (area.size.width as i32 - width) / 2;
    let bottom = area.position.y + area.size.height as i32 - gap;
    move_to(w, x, bottom - height);
}

/// Change the height without the bar jumping: it keeps its bottom edge when it
/// sits in the lower half of the screen (so it grows upward from the taskbar)
/// and its top edge when it sits in the upper half.
fn resize_keeping_anchor(w: &WebviewWindow, height_logical: f64) {
    let scale = w.scale_factor().unwrap_or(1.0);
    let (Ok(pos), Ok(old)) = (w.outer_position(), w.outer_size()) else {
        let _ = w.set_size(LogicalSize::new(BAR_WIDTH, height_logical));
        return;
    };
    let new_h = (height_logical * scale).round() as i32;
    let old_h = old.height as i32;
    if new_h == old_h {
        return;
    }
    let area = w.current_monitor().ok().flatten().map(|m| *m.work_area());
    let lower_half =
        area.is_none_or(|a| pos.y + old_h / 2 > a.position.y + a.size.height as i32 / 2);
    LAST_OWN_MOVE.store(now_ms(), Ordering::SeqCst);
    let _ = w.set_size(LogicalSize::new(BAR_WIDTH, height_logical));
    if lower_half {
        move_to(w, pos.x, pos.y + old_h - new_h);
    }
}

/// React to the window itself: tell the page when it loses focus, and remember
/// where the user drags it.
fn watch_bar(app: &AppHandle, w: &WebviewWindow) {
    let handle = app.clone();
    let win = w.clone();
    w.on_window_event(move |event| match event {
        WindowEvent::Focused(false) => {
            let dragging =
                now_ms().saturating_sub(LAST_USER_MOVE.load(Ordering::SeqCst)) < DRAG_BLUR_QUIET_MS;
            if !dragging {
                let _ = handle.emit("bar-blurred", ());
            }
        }
        WindowEvent::Moved(_) => {
            let ours =
                now_ms().saturating_sub(LAST_OWN_MOVE.load(Ordering::SeqCst)) < OWN_MOVE_QUIET_MS;
            if !ours {
                LAST_USER_MOVE.store(now_ms(), Ordering::SeqCst);
                save_position(&handle, &win);
            }
        }
        _ => {}
    });
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
        .transparent(true)
        .shadow(false)
        .visible(false)
        .build();
    match built {
        Ok(w) => {
            place_bar(app, &w);
            watch_bar(app, &w);
            // Stay hidden until the page says it has drawn, so a blank rectangle
            // never flashes. If the page never answers, show anyway after 3 s.
            let app = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(3));
                show_when_ready(&app);
            });
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
        Some(w) if w.is_visible().unwrap_or(false) => {
            // Let the page play its pop-down animation, then it asks us to hide.
            // If it does not answer, hide anyway.
            let _ = app.emit("bar-close", ());
            let app = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(500));
                if app
                    .get_webview_window(BAR)
                    .is_some_and(|w| w.is_visible().unwrap_or(false))
                {
                    hide(&app);
                }
            });
        }
        Some(w) => {
            app.state::<AppState>()
                .visibility
                .fetch_add(1, Ordering::SeqCst); // cancels the destroy timer
            place_bar(app, &w);
            let _ = w.show();
            let _ = w.set_focus();
            announce_shown(app);
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
            snapshot.set_all(jitmo_adapter_windows::explorer::explorer_selection_all(
                hwnd,
            ));
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
        include_str!("../../../skills/music.control.yaml"),
        include_str!("../../../skills/music.play.yaml"),
        include_str!("../../../skills/calc.math.yaml"),
        include_str!("../../../skills/calc.moon.yaml"),
        include_str!("../../../skills/calc.time_in.yaml"),
        include_str!("../../../skills/settings.place.yaml"),
        include_str!("../../../skills/files.find.yaml"),
        include_str!("../../../skills/files.recent.yaml"),
        include_str!("../../../skills/files.open.yaml"),
        include_str!("../../../skills/files.move.yaml"),
        include_str!("../../../skills/reminders.remind.yaml"),
        include_str!("../../../skills/reminders.timer.yaml"),
        include_str!("../../../skills/system.volume.yaml"),
        include_str!("../../../skills/system.lock.yaml"),
        include_str!("../../../skills/system.radio.yaml"),
        include_str!("../../../skills/system.screenshot.yaml"),
        include_str!("../../../skills/system.dnd.yaml"),
        include_str!("../../../skills/web.weather.yaml"),
        include_str!("../../../skills/web.fact.yaml"),
        include_str!("../../../skills/browser.youtube_latest.yaml"),
        include_str!("../../../skills/browser.search.yaml"),
        include_str!("../../../skills/browser.site.yaml"),
        include_str!("../../../skills/stub.send.yaml"),
    ]
    .iter()
    .map(|y| Skill::parse(y).expect("built-in skill is invalid"))
    .collect()
}

/// `windows_system` is real (it opens apps). The rest are still fakes that
/// report success, until their connectors are built in Phase 2.
/// Every 10 seconds, show the reminders and timers that have come due as
/// Windows notifications. Runs on its own thread with its own database connection.
fn start_reminder_scheduler(db: std::path::PathBuf, key: Option<jitmo_store::Key>) {
    std::thread::spawn(move || {
        let store = match open_store(&db, key.as_ref()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("reminders: could not open the database: {e}");
                return;
            }
        };
        loop {
            let now = chrono_utc_now();
            let _ = jitmo_core::scheduler::fire_due(&store, &now, &mut |n| {
                #[cfg(windows)]
                {
                    jitmo_adapter_windows::toast::show_toast(&n.title, &n.body).is_ok()
                }
                #[cfg(not(windows))]
                {
                    let _ = n;
                    false
                }
            });
            std::thread::sleep(std::time::Duration::from_secs(10));
        }
    });
}

/// The one place the app opens its database: encrypted with the key from
/// Credential Manager. A plain database from an earlier version is converted
/// (and backed up) the first time.
fn open_store(
    db: &std::path::Path,
    key: Option<&jitmo_store::Key>,
) -> Result<Store, jitmo_store::StoreError> {
    match key {
        Some(key) => Store::open_encrypted_migrating(db, key),
        // Development builds (no `encrypted-db` feature) keep a plain file.
        None => Store::open(db, None),
    }
}

/// Read (or create) the database key. There is deliberately no fallback to an
/// unencrypted file: if the key cannot be had, the app does not start.
fn database_key() -> Result<Option<jitmo_store::Key>, String> {
    #[cfg(not(feature = "encrypted-db"))]
    {
        Ok(None)
    }
    #[cfg(all(feature = "encrypted-db", windows))]
    {
        jitmo_adapter_windows::credentials::get_or_create_db_key("jitmo").map(Some)
    }
    #[cfg(all(feature = "encrypted-db", not(windows)))]
    {
        Err("the database key is kept in Windows Credential Manager".to_string())
    }
}

/// Now as `YYYY-MM-DDTHH:MM:SSZ`, the format reminders are stored in.
fn chrono_utc_now() -> String {
    jitmo_connectors::reminders::utc_text(chrono::Local::now())
}

/// The current user's Pictures folder.
fn pictures_dir() -> std::path::PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
        .join("Pictures")
}

/// The current user's Downloads folder.
fn downloads_dir() -> std::path::PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
        .join("Downloads")
}

fn connectors(
    db: &std::path::Path,
    key: Option<&jitmo_store::Key>,
) -> HashMap<String, Arc<dyn Connector>> {
    // The test send skill (`stub.send`) only checks the confirm card. It must say
    // so, or "Done." would look like a real message went out.
    let mut map: HashMap<String, Arc<dyn Connector>> = HashMap::from([(
        "stub_messenger".to_string(),
        Arc::new(FakeConnector::new("stub_messenger").with_answer(
            "This was a test: nothing was really sent. Real messaging is not built yet.",
        )) as Arc<dyn Connector>,
    )]);
    // The place for "will it rain tomorrow" when none is said (setting `default_place`).
    let default_place = open_store(db, key)
        .ok()
        .and_then(|s| s.get_setting("default_place").ok().flatten());
    map.insert(
        "web_lookup".into(),
        Arc::new(
            jitmo_connectors::web_lookup::WebLookup::new(default_place).with_place_source({
                // Read the saved place each time, so "set my city" works at once.
                let (place_db, place_key) = (db.to_path_buf(), key.copied());
                move || {
                    open_store(&place_db, place_key.as_ref())
                        .ok()
                        .and_then(|s| s.get_setting("default_place").ok().flatten())
                }
            }),
        ),
    );
    #[cfg(windows)]
    map.insert(
        "browser".into(),
        Arc::new(jitmo_connectors::browser::Browser::new(Arc::new(
            jitmo_connectors::system_controls::PowerShell,
        ))),
    );
    #[cfg(not(windows))]
    map.insert("browser".into(), Arc::new(FakeConnector::new("browser")));
    #[cfg(windows)]
    map.insert(
        "spotify".into(),
        Arc::new(jitmo_connectors::spotify::Spotify::new(Arc::new(
            jitmo_connectors::system_controls::PowerShell,
        ))),
    );
    #[cfg(not(windows))]
    map.insert("spotify".into(), Arc::new(FakeConnector::new("spotify")));
    #[cfg(windows)]
    map.insert(
        "dnd".into(),
        Arc::new(jitmo_connectors::dnd::Dnd::new(Arc::new(
            jitmo_connectors::system_controls::PowerShell,
        ))),
    );
    #[cfg(not(windows))]
    map.insert("dnd".into(), Arc::new(FakeConnector::new("dnd")));
    #[cfg(windows)]
    map.insert(
        "system_controls".into(),
        Arc::new(jitmo_connectors::system_controls::SystemControls::new(
            Arc::new(jitmo_connectors::system_controls::PowerShell),
            pictures_dir(),
        )),
    );
    map.insert(
        "settings".into(),
        Arc::new(match key {
            Some(k) => jitmo_connectors::settings::Settings::new(db.to_path_buf()).with_key(*k),
            None => jitmo_connectors::settings::Settings::new(db.to_path_buf()),
        }),
    );
    map.insert(
        "reminders".into(),
        Arc::new(match key {
            Some(k) => jitmo_connectors::reminders::Reminders::new(db.to_path_buf()).with_key(*k),
            None => jitmo_connectors::reminders::Reminders::new(db.to_path_buf()),
        }),
    );
    map.insert(
        "files".into(),
        Arc::new(jitmo_connectors::files::Files::new(downloads_dir())),
    );
    map.insert(
        "local_calc".into(),
        Arc::new(jitmo_connectors::local_calc::LocalCalc::new()),
    );
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
            submit, confirm, choose, cancel, undo, hide_bar, resize_bar, bar_ready
        ])
        .setup(|app| {
            // Own the per-user Core pipe first. If another jitmo already does,
            // this instance must not run a second Core (or its scheduler).
            let mut pipe_server = match jitmo_adapter_windows::pipe::PipeServer::bind(
                &jitmo_adapter_windows::pipe::default_pipe_name(),
            ) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("jitmo is already running (pipe in use: {e}); exiting");
                    std::process::exit(0);
                }
            };
            // Store: next to the user's other app data.
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let db_path = dir.join("jitmo.db");
            let key = database_key().map_err(|e| {
                let msg = format!(
                    "jitmo cannot start: the database key could not be read ({e}). \
                     Nothing was opened or changed."
                );
                eprintln!("{msg}");
                msg
            })?;
            let store = open_store(&db_path, key.as_ref()).map_err(|e| {
                let msg = format!(
                    "jitmo cannot start: the database could not be opened ({e}). \
                     The file was not changed."
                );
                eprintln!("{msg}");
                msg
            })?;
            // Build the installed-apps list now, so "open <app>" never waits for it.
            #[cfg(windows)]
            jitmo_adapter_windows::apps::warm_up();
            start_reminder_scheduler(db_path.clone(), key);
            let hotkey = store
                .get_setting("hotkey")?
                .unwrap_or_else(|| DEFAULT_HOTKEY.to_string());

            let selection = SelectionSnapshot::default();
            let flow = Flow::new(router(), store, connectors(&db_path, key.as_ref()))
                .with_context(FsContext::for_current_user(selection.clone()));
            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("jitmo-core".into())
                .spawn(move || {
                    let mut flow = flow;
                    loop {
                        match pipe_server.accept() {
                            Ok(stream) => {
                                let (r, w) = stream.split();
                                serve_connection(&mut flow, std::io::BufReader::new(r), w);
                            }
                            Err(e) => {
                                eprintln!("core pipe accept failed: {e}");
                                std::thread::sleep(Duration::from_secs(1));
                            }
                        }
                    }
                })?;
            let core = CoreClient::start(move |event| {
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
