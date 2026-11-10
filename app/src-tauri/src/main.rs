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

use jitmo_core::context::{FsContext, SelectionSnapshot};
use jitmo_core::flow::Flow;
use jitmo_core::service::serve_connection;
use std::str::FromStr;
use std::sync::atomic::AtomicU64;
use std::time::Duration;
use tauri::{Emitter, Manager, RunEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

mod bar;
mod commands;
mod core_client;
mod registry;
mod scheduler;
mod sound;
mod storage;

use commands::{
    answer, bar_ready, bar_sound, cancel, choose, confirm, hide_bar, open_source, resize_bar,
    submit, undo, AppState,
};
use core_client::CoreClient;

const DEFAULT_HOTKEY: &str = "ctrl+alt+j"; // Ctrl+Alt+Space was already taken on the dev PC (SP11)

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            submit,
            confirm,
            choose,
            answer,
            cancel,
            undo,
            hide_bar,
            resize_bar,
            bar_ready,
            bar_sound,
            open_source
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
            let key = storage::database_key().map_err(|e| {
                let msg = format!(
                    "jitmo cannot start: the database key could not be read ({e}). \
                     Nothing was opened or changed."
                );
                eprintln!("{msg}");
                msg
            })?;
            let store = storage::open_store(&db_path, key.as_ref()).map_err(|e| {
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
            // People who can be sent to: learned from use, plus any vCard (.vcf) the
            // user has put in their Contacts folder (Export in Windows Contacts,
            // Outlook, Google Contacts or a phone). Nothing is read from an account.
            if let Some(home) = std::env::var_os("USERPROFILE") {
                let dir = std::path::Path::new(&home).join("Contacts");
                let n = jitmo_core::known::import_vcard_folder(&store, &dir);
                if n > 0 {
                    eprintln!("imported {n} contact card(s) from {}", dir.display());
                }
            }
            sound::set_enabled(store.get_setting("bar_sounds")?.as_deref() != Some("off"));
            sound::warm_up();
            scheduler::start_reminder_scheduler(db_path.clone(), key, app.handle().clone());
            let hotkey = store
                .get_setting("hotkey")?
                .unwrap_or_else(|| DEFAULT_HOTKEY.to_string());

            let selection = SelectionSnapshot::default();
            let flow = Flow::new(
                registry::router(),
                store,
                registry::connectors(&db_path, key.as_ref()),
            )
            .with_context(FsContext::for_current_user(selection.clone()));
            // Installed apps are a closed list for understanding and resolving requests.
            #[cfg(windows)]
            let flow = flow.with_app_names(jitmo_adapter_windows::apps::installed_app_names);
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
                                bar::toggle(app);
                            }
                        })
                        .map_err(|e| e.to_string())
                });
            if let Err(e) = registered {
                eprintln!("could not register the hotkey `{hotkey}`: {e}");
                bar::create_bar(app.handle());
            } else if std::env::var_os("JITMO_OPEN_ON_START").is_some() {
                // For measuring and demos: show the bar without pressing the hotkey.
                bar::create_bar(app.handle());
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
