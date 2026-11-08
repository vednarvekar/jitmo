// Variant B: destroy the web view when hidden, rebuild on hotkey.
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

fn now_ms() -> u128 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() }

fn main() {
    let sc = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyJ);
    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, shortcut, event| {
                    if shortcut == &sc && event.state() == ShortcutState::Pressed {
                        let t0 = now_ms();
                        if let Some(w) = app.get_webview_window("bar") {
                            let _ = w.destroy();
                            println!("DESTROY t={}", now_ms());
                        } else {
                            let w = WebviewWindowBuilder::new(app, "bar", WebviewUrl::App("index.html".into()))
                                .inner_size(640.0, 72.0).decorations(false).always_on_top(true)
                                .skip_taskbar(true).resizable(false).center().build().unwrap();
                            let _ = w.set_focus();
                            println!("SHOW build_ms={}", now_ms() - t0);
                        }
                    }
                })
                .build(),
        )
        .setup(move |app| { app.global_shortcut().register(sc)?; Ok(()) })
        .build(tauri::generate_context!())
        .expect("build")
        .run(|_app, e| { if let RunEvent::ExitRequested { api, code, .. } = e { if code.is_none() { api.prevent_exit(); } } });
}
