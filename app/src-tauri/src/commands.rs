use crate::bar::{self, BAR, BAR_START_HEIGHT};
use crate::core_client::CoreClient;
use jitmo_core::context::SelectionSnapshot;
use jitmo_ipc::UiRequest;
use std::sync::atomic::AtomicU64;
use tauri::{AppHandle, Manager, State};

pub(crate) struct AppState {
    pub(crate) core: CoreClient,
    /// What Explorer had selected when the bar was last opened.
    pub(crate) selection: SelectionSnapshot,
    /// Bumped on every show and hide, so a pending "destroy" timer can tell it is stale.
    pub(crate) visibility: AtomicU64,
}

// --- commands the web view may call ---------------------------------------

#[tauri::command]
pub(crate) fn submit(state: State<AppState>, id: u64, text: String) {
    state.core.send(UiRequest::Submit { id, text });
}

#[tauri::command]
pub(crate) fn confirm(state: State<AppState>, id: u64, approved: bool) {
    state.core.send(UiRequest::Confirm { id, approved });
}

#[tauri::command]
pub(crate) fn choose(state: State<AppState>, id: u64, choice_id: String) {
    state.core.send(UiRequest::Choose { id, choice_id });
}

#[tauri::command]
pub(crate) fn answer(state: State<AppState>, id: u64, text: String) {
    state.core.send(UiRequest::Answer { id, text });
}

#[tauri::command]
pub(crate) fn cancel(state: State<AppState>, id: u64) {
    state.core.send(UiRequest::Cancel { id });
}

#[tauri::command]
pub(crate) fn undo(state: State<AppState>, id: u64) {
    state.core.send(UiRequest::Undo { id });
}

/// The page has drawn its first frame: now it is safe to show the window.
#[tauri::command]
pub(crate) fn bar_ready(app: AppHandle) {
    bar::show_when_ready(&app);
}

#[tauri::command]
pub(crate) fn hide_bar(app: AppHandle) {
    bar::hide(&app);
}

#[tauri::command]
pub(crate) fn resize_bar(app: AppHandle, height: f64) {
    if let Some(w) = app.get_webview_window(BAR) {
        bar::resize_keeping_anchor(&w, height.clamp(BAR_START_HEIGHT, 700.0));
    }
}
