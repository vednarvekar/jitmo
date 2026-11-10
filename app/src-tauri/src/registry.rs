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
        include_str!("../../../skills/mail.urgent.yaml"),
        include_str!("../../../skills/mail.inbox.yaml"),
        include_str!("../../../skills/mail.summarize.yaml"),
        include_str!("../../../skills/mail.find.yaml"),
        include_str!("../../../skills/mail.unanswered.yaml"),
        include_str!("../../../skills/mail.draft.yaml"),
        include_str!("../../../skills/mail.archive_newsletters.yaml"),
        include_str!("../../../skills/mail.send.yaml"),
        include_str!("../../../skills/files.email.yaml"),
        include_str!("../../../skills/files.share.yaml"),
        include_str!("../../../skills/chat.post.yaml"),
        include_str!("../../../skills/calendar.agenda.yaml"),
        include_str!("../../../skills/calendar.free.yaml"),
        include_str!("../../../skills/calendar.context.yaml"),
        include_str!("../../../skills/calendar.add.yaml"),
        include_str!("../../../skills/calendar.block.yaml"),
        include_str!("../../../skills/calendar.invite.yaml"),
        include_str!("../../../skills/calendar.move.yaml"),
        include_str!("../../../skills/calendar.decline.yaml"),
        include_str!("../../../skills/calendar.accept.yaml"),
        include_str!("../../../skills/work.morning_brief.yaml"),
        include_str!("../../../skills/work.event_from_email.yaml"),
        include_str!("../../../skills/work.save_attachments.yaml"),
        include_str!("../../../skills/work.forward_attachment.yaml"),
        include_str!("../../../skills/work.send_report.yaml"),
        include_str!("../../../skills/account.ms365_sign_in.yaml"),
        include_str!("../../../skills/account.ms365_sign_out.yaml"),
        include_str!("../../../skills/account.ms365_client.yaml"),
        include_str!("../../../skills/account.google_sign_in.yaml"),
        include_str!("../../../skills/account.google_sign_out.yaml"),
        include_str!("../../../skills/account.google_client.yaml"),
        include_str!("../../../skills/whatsapp.send_file.yaml"),
        include_str!("../../../skills/whatsapp.send_message.yaml"),
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
        // The saved place answers "when does the sun set" with no city named.
        Arc::new(
            jitmo_connectors::local_calc::LocalCalc::new().with_place_source(setting_reader(
                db,
                key,
                "default_place",
            )),
        ),
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
    add_ms365(&mut map, db, key);
    add_google(&mut map, db, key);
    add_whatsapp(&mut map, db, key);
    map
}

/// Microsoft 365: the real connectors when an app registration (a client id) is saved,
/// otherwise stand-ins that say plainly that it is not set up (Phase 3, Track A).
fn add_ms365(
    map: &mut HashMap<String, Arc<dyn Connector>>,
    db: &std::path::Path,
    key: Option<&jitmo_store::Key>,
) {
    use jitmo_connectors::graph::{Graph, GraphCalendar, GraphMail, NoChat, NoDrive, UreqHttp};
    use jitmo_connectors::ms365::Outlook;
    use jitmo_connectors::ms365_account::{default_scopes, Ms365Account, NotSetUp};
    use jitmo_connectors::ms365_cal::OutlookCalendar;
    use jitmo_connectors::ms365_work::{fake_suite, OneDrive, Teams, WorkflowParts, Workflows};
    use jitmo_connectors::oauth::{OAuthConfig, Session, UreqTransport};
    use jitmo_connectors::Health;

    let client_id = setting_reader(db, key, "ms365_client_id")().filter(|s| !s.trim().is_empty());
    let tenant = setting_reader(db, key, "ms365_tenant")()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "common".to_string());
    let cfg = client_id.map(|id| OAuthConfig {
        client_id: id.trim().to_string(),
        tenant,
        redirect_uri: String::new(),
        scopes: default_scopes(),
        client_secret: None,
    });
    #[cfg(windows)]
    let tokens: Box<dyn jitmo_connectors::oauth::TokenStore> =
        Box::new(jitmo_connectors::oauth::CredentialTokenStore);
    #[cfg(not(windows))]
    let tokens: Box<dyn jitmo_connectors::oauth::TokenStore> =
        Box::new(jitmo_connectors::oauth::MemoryTokenStore::default());
    let session = cfg
        .clone()
        .map(|c| Arc::new(Session::new(c, Box::new(UreqTransport), tokens)));

    let (save_db, save_key) = (db.to_path_buf(), key.copied());
    let saver = move |id: &str| -> Result<(), String> {
        open_store(&save_db, save_key.as_ref())
            .map_err(|e| e.to_string())?
            .set_setting("ms365_client_id", id)
            .map_err(|e| e.to_string())
    };
    map.insert(
        "ms365_account".into(),
        Arc::new(Ms365Account::new(
            cfg,
            session.clone(),
            Box::new(open_in_browser),
            Box::new(saver),
        )),
    );

    let Some(session) = session else {
        // No app registration yet: the same connectors, each saying so.
        for c in fake_suite().connectors {
            let id = c.id().to_string();
            map.insert(id.clone(), Arc::new(NotSetUp::new(&id, c.as_ref())));
        }
        return;
    };

    // Signed in means a refresh token is stored on this PC.
    let health = {
        let session = session.clone();
        move || -> Box<dyn Fn() -> Health + Send + Sync> {
            let s = session.clone();
            Box::new(move || {
                if s.is_signed_in() {
                    Health::Ready
                } else {
                    Health::NotSignedIn
                }
            })
        }
    };
    let graph = Arc::new(Graph::new(Arc::new(UreqHttp), session));
    let mail = Arc::new(GraphMail::new(graph.clone()));
    let calendar = Arc::new(GraphCalendar::new(graph));
    let now = || -> Box<dyn Fn() -> chrono::DateTime<chrono::Local> + Send + Sync> {
        Box::new(chrono::Local::now)
    };
    let outlook: Arc<dyn Connector> = Arc::new(Outlook::new(mail.clone(), health(), now()));
    let agenda: Arc<dyn Connector> =
        Arc::new(OutlookCalendar::new(calendar.clone(), health(), now()));
    let drive = Arc::new(NoDrive);
    let chat = Arc::new(NoChat);
    let workflows: Arc<dyn Connector> = Arc::new(Workflows::new(
        WorkflowParts {
            mail,
            calendar,
            drive: drive.clone(),
            chat: chat.clone(),
            outlook: outlook.clone(),
            agenda: agenda.clone(),
        },
        health(),
        now(),
    ));
    for c in [
        outlook,
        agenda,
        Arc::new(OneDrive::new(drive, health())) as Arc<dyn Connector>,
        Arc::new(Teams::new(chat, health())),
        workflows,
    ] {
        map.insert(c.id().to_string(), c);
    }
}

/// Google Workspace: when a Google client is saved and Microsoft is not (or `mail_provider`
/// is "google"), the same mail, calendar and file connectors run on Gmail, Google Calendar
/// and Drive. The skills do not change, only what is behind the connectors.
fn add_google(
    map: &mut HashMap<String, Arc<dyn Connector>>,
    db: &std::path::Path,
    key: Option<&jitmo_store::Key>,
) {
    use jitmo_connectors::google::{scopes, GoogleCalendar, GoogleDrive, GoogleMail};
    use jitmo_connectors::google_account::GoogleAccount;
    use jitmo_connectors::graph::{Graph, NoChat, UreqHttp};
    use jitmo_connectors::ms365::Outlook;
    use jitmo_connectors::ms365_cal::OutlookCalendar;
    use jitmo_connectors::ms365_work::{OneDrive, WorkflowParts, Workflows};
    use jitmo_connectors::oauth::{OAuthConfig, Session, UreqTransport, GOOGLE};
    use jitmo_connectors::Health;

    let id = setting_reader(db, key, "google_client_id")().filter(|s| !s.trim().is_empty());
    let secret = setting_reader(db, key, "google_client_secret")().filter(|s| !s.trim().is_empty());
    let cfg = match (id, secret) {
        (Some(id), Some(secret)) => Some(OAuthConfig {
            client_id: id.trim().to_string(),
            tenant: GOOGLE.to_string(),
            redirect_uri: String::new(),
            scopes: scopes(),
            client_secret: Some(secret.trim().to_string()),
        }),
        _ => None,
    };
    #[cfg(windows)]
    let tokens: Box<dyn jitmo_connectors::oauth::TokenStore> =
        Box::new(jitmo_connectors::oauth::GoogleTokenStore);
    #[cfg(not(windows))]
    let tokens: Box<dyn jitmo_connectors::oauth::TokenStore> =
        Box::new(jitmo_connectors::oauth::MemoryTokenStore::default());
    let session = cfg
        .clone()
        .map(|c| Arc::new(Session::new(c, Box::new(UreqTransport), tokens)));

    let (save_db, save_key) = (db.to_path_buf(), key.copied());
    let saver = move |id: &str, secret: &str| -> Result<(), String> {
        let store = open_store(&save_db, save_key.as_ref()).map_err(|e| e.to_string())?;
        store
            .set_setting("google_client_id", id)
            .map_err(|e| e.to_string())?;
        store
            .set_setting("google_client_secret", secret)
            .map_err(|e| e.to_string())
    };
    map.insert(
        "google_account".into(),
        Arc::new(GoogleAccount::new(
            cfg,
            session.clone(),
            Box::new(open_in_browser),
            Box::new(saver),
        )),
    );

    let Some(session) = session else { return };
    // Microsoft stays in charge when it is set up, unless Google is chosen explicitly.
    let ms_set_up =
        setting_reader(db, key, "ms365_client_id")().is_some_and(|s| !s.trim().is_empty());
    let chosen = setting_reader(db, key, "mail_provider")()
        .is_some_and(|s| s.trim().eq_ignore_ascii_case("google"));
    if ms_set_up && !chosen {
        return;
    }

    let health = {
        let session = session.clone();
        move || -> Box<dyn Fn() -> Health + Send + Sync> {
            let s = session.clone();
            Box::new(move || {
                if s.is_signed_in() {
                    Health::Ready
                } else {
                    Health::NotSignedIn
                }
            })
        }
    };
    let g = Arc::new(Graph::new(Arc::new(UreqHttp), session).labelled("Google"));
    let mail = Arc::new(GoogleMail::new(g.clone()));
    let calendar = Arc::new(GoogleCalendar::new(g.clone()));
    let drive = Arc::new(GoogleDrive::new(g));
    let now = || -> Box<dyn Fn() -> chrono::DateTime<chrono::Local> + Send + Sync> {
        Box::new(chrono::Local::now)
    };
    let outlook: Arc<dyn Connector> = Arc::new(Outlook::new(mail.clone(), health(), now()));
    let agenda: Arc<dyn Connector> =
        Arc::new(OutlookCalendar::new(calendar.clone(), health(), now()));
    let chat = Arc::new(NoChat);
    let workflows: Arc<dyn Connector> = Arc::new(Workflows::new(
        WorkflowParts {
            mail,
            calendar,
            drive: drive.clone(),
            chat,
            outlook: outlook.clone(),
            agenda: agenda.clone(),
        },
        health(),
        now(),
    ));
    for c in [
        outlook,
        agenda,
        Arc::new(OneDrive::new(drive, health())) as Arc<dyn Connector>,
        workflows,
    ] {
        map.insert(c.id().to_string(), c);
    }
}

/// Open a web page in the default browser (no shell involved).
fn open_in_browser(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = url;
        Err("not supported here".into())
    }
}

/// WhatsApp on this PC: opens the chat with the message typed (or the file copied) and the
/// user presses Send. Numbers come from the saved contacts.
fn add_whatsapp(
    map: &mut HashMap<String, Arc<dyn Connector>>,
    db: &std::path::Path,
    key: Option<&jitmo_store::Key>,
) {
    use jitmo_connectors::whatsapp::WhatsAppDesktop;

    let (db, key) = (db.to_path_buf(), key.copied());
    let phone_of = move |name: &str| -> Option<String> {
        let store = open_store(&db, key.as_ref()).ok()?;
        store
            .contacts()
            .ok()?
            .into_iter()
            .find(|c| c.name.eq_ignore_ascii_case(name))
            .and_then(|c| c.phone)
    };
    map.insert(
        "whatsapp_desktop".into(),
        Arc::new(WhatsAppDesktop::new(
            Box::new(phone_of),
            Box::new(open_in_browser),
            Box::new(copy_file_to_clipboard),
            Box::new(whatsapp_desktop_installed),
        )),
    );
}

/// Put a file on the clipboard (as a file, so Ctrl+V pastes it into WhatsApp). The path is
/// passed as an argument, never spliced into a script.
fn copy_file_to_clipboard(path: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        let status = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Set-Clipboard -LiteralPath $args[0]",
                path,
            ])
            .status()
            .map_err(|e| e.to_string())?;
        status
            .success()
            .then_some(())
            .ok_or_else(|| "clipboard".to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("not supported here".into())
    }
}

/// Is the `whatsapp://` link registered (the desktop app is installed)?
fn whatsapp_desktop_installed() -> bool {
    #[cfg(windows)]
    {
        std::process::Command::new("reg")
            .args(["query", r"HKEY_CLASSES_ROOT\whatsapp"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// The calendar connector for the meeting heads-up (its own instance, so it never waits on
/// the engine's thread).
pub(crate) fn calendar_connector(
    db: &std::path::Path,
    key: Option<&jitmo_store::Key>,
) -> Arc<dyn Connector> {
    let mut all = connectors(db, key);
    all.remove("outlook_calendar").unwrap_or_else(|| {
        Arc::new(jitmo_connectors::fake::FakeConnector::new(
            "outlook_calendar",
        ))
    })
}

/// The local language model, when its two files are on this PC: `llama-server.exe` and a
/// `.gguf` model, by default in `<app data>/models/` (`models/llama/llama-server.exe` and the
/// first `.gguf` there), or wherever the settings `llm_server` and `llm_model` point. Off with
/// the setting `llm` = `off`. Without the files the engine runs exactly as before.
pub(crate) fn local_llm(
    store: &jitmo_store::Store,
    data_dir: &std::path::Path,
) -> Option<Box<dyn jitmo_core::llm::Llm>> {
    use jitmo_llm::{LlmConfig, LocalLlm};
    let setting = |k: &str| {
        store
            .get_setting(k)
            .ok()
            .flatten()
            .filter(|v| !v.trim().is_empty())
    };
    if setting("llm").is_some_and(|v| v.eq_ignore_ascii_case("off")) {
        return None;
    }
    let models = data_dir.join("models");
    let server = setting("llm_server")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| models.join("llama").join("llama-server.exe"));
    let model = setting("llm_model")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::fs::read_dir(&models)
                .ok()?
                .flatten()
                .map(|e| e.path())
                .find(|p| p.extension().and_then(|x| x.to_str()) == Some("gguf"))
        })?;
    if !server.is_file() || !model.is_file() {
        return None;
    }
    let mut cfg = LlmConfig::new(server, model);
    if let Some(t) = setting("llm_threads").and_then(|t| t.parse().ok()) {
        cfg.threads = t;
    }
    eprintln!("local language model: {}", cfg.model.display());
    Some(Box::new(LocalLlm::new(cfg)))
}
