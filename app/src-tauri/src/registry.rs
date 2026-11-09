use crate::storage::open_store;
#[cfg(not(windows))]
use jitmo_connectors::fake::FakeConnector;
use jitmo_connectors::Connector;
use jitmo_core::router::Router;
use jitmo_model_worker::{WorkerClient, WorkerLauncher};
use jitmo_skills::Skill;
use std::collections::HashMap;
use std::sync::Arc;

/// Skills shipped with the app. Dev build: parsed without signature checks.
/// Rule-based routing, plus the model worker when its program sits next to the app.
pub(crate) fn router() -> Router {
    let router = Router::new(load_skills());
    let worker = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("jitmo-model-worker.exe")))
        .filter(|p| p.exists());
    match worker {
        Some(program) => {
            // Without model folders the worker exits and the app would silently fall
            // back to the weak rule-based router, so say where they are.
            let mut args = Vec::new();
            for (flag, sub) in [("--models", "router"), ("--extractor", "ner")] {
                match find_model_dir(sub) {
                    Some(dir) => {
                        args.push(flag.to_string());
                        args.push(dir.to_string_lossy().into_owned());
                    }
                    None => eprintln!("jitmo: model folder models/{sub} not found"),
                }
            }
            router.with_worker(WorkerClient::new(
                WorkerLauncher { program, args },
                std::time::Duration::from_secs(10),
            ))
        }
        None => {
            eprintln!(
                "jitmo: jitmo-model-worker.exe not found next to the app: rule-based router only"
            );
            router
        }
    }
}

/// The folder `models/<sub>`: next to the program (installed app), else in a
/// folder above it (a development build in `target/debug` finds the repo's
/// `models/`), else relative to the current folder. A folder counts only when
/// it holds an ONNX model.
fn find_model_dir(sub: &str) -> Option<std::path::PathBuf> {
    let mut candidates = Vec::new();
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.to_path_buf()))
    {
        candidates.extend(
            exe_dir
                .ancestors()
                .take(5)
                .map(|a| a.join("models").join(sub)),
        );
    }
    candidates.push(std::path::PathBuf::from("models").join(sub));
    candidates
        .into_iter()
        .find(|d| d.join("model_int8.onnx").exists())
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
    ]
    .iter()
    .map(|y| Skill::parse(y).expect("built-in skill is invalid"))
    .collect()
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

/// A reader of one setting that opens the store on each call, so a change
/// (such as "set my city to X") is seen at once without a restart.
fn setting_reader(
    db: &std::path::Path,
    key: Option<&jitmo_store::Key>,
    name: &'static str,
) -> impl Fn() -> Option<String> + Send + Sync + 'static {
    let (db, key) = (db.to_path_buf(), key.copied());
    move || {
        open_store(&db, key.as_ref())
            .ok()
            .and_then(|s| s.get_setting(name).ok().flatten())
    }
}

/// The connectors the app runs: real ones where they exist, and a clearly
/// labelled test connector for sending messages.
pub(crate) fn connectors(
    db: &std::path::Path,
    key: Option<&jitmo_store::Key>,
) -> HashMap<String, Arc<dyn Connector>> {
    // Messaging is not built (Phase 3), so there is no send skill here: the test skill
    // `stub.send` lives in the test set only, where its pattern cannot hijack a real
    // "send X to Y".
    let mut map: HashMap<String, Arc<dyn Connector>> = HashMap::new();
    // The place for "will it rain tomorrow" when none is said (setting `default_place`).
    let default_place = open_store(db, key)
        .ok()
        .and_then(|s| s.get_setting("default_place").ok().flatten());
    map.insert(
        "web_lookup".into(),
        Arc::new(
            jitmo_connectors::web_lookup::WebLookup::new(default_place)
                .with_place_source(setting_reader(db, key, "default_place"))
                // Provider order / switching one off, without an app update
                // (e.g. `weather=metno,nws;fact=wikipedia_rest`).
                .with_config_source(setting_reader(db, key, "lookup_providers"))
                // Last good answers, shown when the internet is down.
                .with_cache_store(setting_reader(db, key, "lookup_cache"), {
                    let (cache_db, cache_key) = (db.to_path_buf(), key.copied());
                    move |text: &str| {
                        if let Ok(s) = open_store(&cache_db, cache_key.as_ref()) {
                            let _ = s.set_setting("lookup_cache", text);
                        }
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
