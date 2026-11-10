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

/// Silence a ringing timer or reminder (Esc, Enter or Backspace on the alarm).
#[tauri::command]
pub(crate) fn stop_alarm() {
    crate::sound::stop_alarm();
}

#[tauri::command]
pub(crate) fn hide_bar(app: AppHandle) {
    // A ringing alarm is put away only after the page has stopped it (Esc, Enter, Backspace,
    // a click): a stray blur or timer must not hide it.
    if crate::sound::is_alarming() {
        return;
    }
    bar::hide(&app);
}

#[tauri::command]
pub(crate) fn resize_bar(app: AppHandle, height: f64, width: f64) {
    if let Some(w) = app.get_webview_window(BAR) {
        bar::resize_keeping_anchor(
            &w,
            width.clamp(bar::BAR_WIDTH, bar::BAR_MAX_WIDTH),
            height.clamp(BAR_START_HEIGHT, 700.0),
        );
    }
}

/// Hosts an answer's source link may point to. Anything else is refused, so a page
/// (or a crafted answer) can never make the bar open an arbitrary URL or program.
const SOURCE_HOSTS: &[&str] = &[
    "en.wikipedia.org",
    "www.wikidata.org",
    "www.met.no",
    "api.met.no",
    "en.ilmatieteenlaitos.fi",
    "www.ecmwf.int",
    "www.weather.gov",
];

/// True for `https://<allowed host>/<plain path>`: no credentials, port, quotes or spaces.
pub(crate) fn is_source_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or("");
    SOURCE_HOSTS.contains(&host)
        && url.len() < 500
        && url
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/:._-~%()?=&#,".contains(c))
}

/// Open an answer's source in the default browser (the round source button).
#[tauri::command]
pub(crate) fn open_source(url: String) -> Result<(), String> {
    if !is_source_url(&url) {
        return Err("That link can't be opened.".into());
    }
    // explorer.exe hands an https URL to the default browser; no shell is involved and
    // the URL has already been limited to plain characters on known hosts.
    std::process::Command::new("explorer.exe")
        .arg(&url)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::is_source_url;

    #[test]
    fn only_known_https_sources_open() {
        assert!(is_source_url("https://en.wikipedia.org/wiki/Canberra"));
        assert!(is_source_url("https://www.met.no/en"));
        assert!(!is_source_url("http://en.wikipedia.org/wiki/Canberra"));
        assert!(!is_source_url("https://evil.example/wiki"));
        assert!(!is_source_url("https://en.wikipedia.org.evil.example/x"));
        assert!(!is_source_url("https://user@en.wikipedia.org/x"));
        assert!(!is_source_url("https://en.wikipedia.org/wiki/a\"b"));
        assert!(!is_source_url("https://en.wikipedia.org/wiki/a b"));
        assert!(!is_source_url("file:///C:/Windows/System32/calc.exe"));
    }
}

/// Play one of the bar's two small sounds (`open` or `close`). Muted by the
/// `bar_sounds` = `off` setting.
#[tauri::command]
pub(crate) fn bar_sound(kind: String) {
    if let Some(cue) = crate::sound::Cue::from_name(&kind) {
        crate::sound::play(cue);
    }
}
