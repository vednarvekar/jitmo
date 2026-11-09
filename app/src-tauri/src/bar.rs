use crate::commands::AppState;
use std::sync::atomic::AtomicU64 as OwnMoveClock;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};

pub(crate) const BAR: &str = "bar";
/// Window size in logical pixels. The window is larger than the pill: the extra
/// 14 px on every side is transparent and holds the pill's shadow (see `.shell` in styles.css).
/// The starting (and smallest) width. The page asks for more when the typed text needs it.
pub(crate) const BAR_WIDTH: f64 = 440.0;
/// The widest the bar may grow.
pub(crate) const BAR_MAX_WIDTH: f64 = 720.0;
pub(crate) const BAR_START_HEIGHT: f64 = 74.0;
/// Gap between the pill's shadow margin and the taskbar, in logical pixels.
const TASKBAR_GAP: f64 = 0.0;
/// Ignore "moved" events this long after the app moved the window itself.
const OWN_MOVE_QUIET_MS: u64 = 400;
const DESTROY_AFTER_HIDDEN: Duration = Duration::from_secs(60);

/// When the app last moved the bar itself (ms since the epoch), so those moves
/// are not mistaken for the user dragging it.
static LAST_OWN_MOVE: OwnMoveClock = OwnMoveClock::new(0);
/// When the user last dragged the bar. Dragging makes the window lose focus for a
/// moment, which must not be taken for a click elsewhere.
static LAST_USER_MOVE: OwnMoveClock = OwnMoveClock::new(0);
/// A focus loss this soon after a drag is part of the drag.
const DRAG_BLUR_QUIET_MS: u64 = 1500;

/// Tell the page it is on screen so it can play the pop-up animation. Wait a moment
/// first: right after the window appears the web view is still painting its first
/// frame, and an animation that starts at that instant stutters.
fn announce_shown(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(70));
        let _ = app.emit("bar-shown", ());
        crate::sound::play_open_unless_early();
    });
}

pub(crate) fn show_when_ready(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(BAR) {
        if !w.is_visible().unwrap_or(false) {
            place_bar(app, &w);
            let _ = w.show();
            let _ = w.set_focus();
            announce_shown(app);
        }
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

/// The saved spot is the window's horizontal CENTRE and its BOTTOM edge, in physical
/// pixels: the bar grows upward from the taskbar and widens around its middle.
/// Older files held the left edge (`x`) of a window that was 580 logical pixels wide.
fn load_saved(app: &AppHandle, scale: f64) -> Option<(i32, i32)> {
    let text = std::fs::read_to_string(position_file(app)?).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let bottom = v.get("bottom")?.as_i64()? as i32;
    let cx = match v.get("cx").and_then(serde_json::Value::as_i64) {
        Some(cx) => cx as i32,
        None => v.get("x")?.as_i64()? as i32 + (580.0 * scale / 2.0) as i32,
    };
    Some((cx, bottom))
}

fn save_position(app: &AppHandle, w: &WebviewWindow) {
    let (Some(file), Ok(pos), Ok(size)) = (position_file(app), w.outer_position(), w.outer_size())
    else {
        return;
    };
    let json = serde_json::json!({
        "cx": pos.x + size.width as i32 / 2,
        "bottom": pos.y + size.height as i32,
    });
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
    if let Some((cx, bottom)) = load_saved(app, w.scale_factor().unwrap_or(1.0)) {
        // Still visible on some monitor? Check the window's middle.
        if work_area_at(app, cx, bottom - height / 2).is_some() {
            move_to(w, cx - width / 2, bottom - height);
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
pub(crate) fn resize_keeping_anchor(w: &WebviewWindow, width_logical: f64, height_logical: f64) {
    let scale = w.scale_factor().unwrap_or(1.0);
    let (Ok(pos), Ok(old)) = (w.outer_position(), w.outer_size()) else {
        let _ = w.set_size(LogicalSize::new(width_logical, height_logical));
        return;
    };
    let new_h = (height_logical * scale).round() as i32;
    let new_w = (width_logical * scale).round() as i32;
    let old_h = old.height as i32;
    if new_h == old_h && new_w == old.width as i32 {
        return;
    }
    let area = w.current_monitor().ok().flatten().map(|m| *m.work_area());
    let lower_half =
        area.is_none_or(|a| pos.y + old_h / 2 > a.position.y + a.size.height as i32 / 2);
    LAST_OWN_MOVE.store(now_ms(), Ordering::SeqCst);
    let _ = w.set_size(LogicalSize::new(width_logical, height_logical));
    // The bar widens and narrows around its middle, so it stays centred.
    let x = pos.x + (old.width as i32 - new_w) / 2;
    let y = if lower_half {
        pos.y + old_h - new_h
    } else {
        pos.y
    };
    if x != pos.x || y != pos.y {
        move_to(w, x, y);
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

pub(crate) fn create_bar(app: &AppHandle) {
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

/// Show the bar with a message nobody asked for (a timer or reminder that came due).
/// An existing window is shown without taking keyboard focus, so typing in another app is
/// not swallowed by the bar. Runs on the caller's thread and waits for the page to be up.
pub(crate) fn show_notice(app: &AppHandle, text: &str) {
    let existing = app.get_webview_window(BAR);
    let visible = existing
        .as_ref()
        .is_some_and(|w| w.is_visible().unwrap_or(false));
    if !visible {
        match &existing {
            Some(w) => {
                app.state::<AppState>()
                    .visibility
                    .fetch_add(1, Ordering::SeqCst); // cancels the destroy timer
                place_bar(app, w);
                let _ = w.show();
                announce_shown(app);
                std::thread::sleep(Duration::from_millis(700));
            }
            None => {
                // The web view was destroyed after a minute of rest: build it again and
                // wait for the page to draw.
                create_bar(app);
                std::thread::sleep(Duration::from_millis(4_000));
            }
        }
    }
    let _ = app.emit("bar-notice", text);
}

pub(crate) fn toggle(app: &AppHandle) {
    if !app
        .get_webview_window(BAR)
        .is_some_and(|w| w.is_visible().unwrap_or(false))
    {
        // Start the open sound at once, before the slower steps below: the audio device
        // may be asleep and needs a moment to wake. (A destroyed window plays it later,
        // when the new page is ready.)
        if app.get_webview_window(BAR).is_some() {
            crate::sound::play_open_early();
        }
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

pub(crate) fn hide(app: &AppHandle) {
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
