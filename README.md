# jitmo

A local assistant for Windows. You press a hotkey, type what you want ("open Spotify",
"remind me at 6pm", "move these PDFs to Invoices"), and it does it on your own PC. A small
model on the device understands the request; nothing leaves the machine except the search
terms of a weather or fact lookup.

Status: Phase 2 of the build (offline tasks) is mostly done. See
[docs/03-delivery/8 - Development Phases.md](docs/03-delivery/8%20-%20Development%20Phases.md)
and the [Decision Log](docs/03-delivery/8.1%20-%20Decision%20Log.md) for what works, what was
measured and what is still open.

## Layout

| Folder | What it holds |
|---|---|
| `app/` | The command bar. `src/` is the React UI (`components/`, `orbs/`, `styles/`), `src-tauri/` is the Tauri shell (window, hotkey, startup). |
| `crates/core` | The engine: routing, slot resolving, policy rules, the flow from request to result, Undo, the reminder scheduler. |
| `crates/skills` | The skill format (signed YAML), validation and loading. |
| `crates/connectors` | One connector per area: `windows_system`, `files`, `local_calc`, `reminders`, `system_controls`, `spotify`, `browser`, `web_lookup`, `dnd`, `settings`. They act on the PC; skills only call them through one trait. |
| `crates/adapter-windows` | Windows-only code: app index, Explorer selection, toasts, named pipe, Credential Manager. |
| `crates/model-worker` | The separate model process (router, extractor) and the client Core uses to run it. |
| `crates/store` | The SQLite database (encrypted in release builds) and its migrations. |
| `crates/ipc` | The messages between the UI, Core and the model worker. |
| `crates/testset-runner` | Runs the 100-test list from the spec and reports pass, time and memory. |
| `skills/` | The skills themselves, one YAML file each. |
| `testset/` | Which of the 100 tests can run automatically, and what each expects. |
| `models-src/` | The small model files that are committed (the trained router head). The encoder files are fetched, see its README. |
| `tools/` | Training and measuring scripts for the models. Nothing here ships. |
| `docs/` | Vision, spec, design and delivery plan. |
| `spikes/` | The Phase 0 experiments that decided the design (kept for their evidence). |
| `server/` | Placeholder for the update server (not needed until the beta). |

## Build and test

You need Rust (stable), Node with pnpm, and on Windows the WebView2 runtime.

```bash
cargo test --workspace            # all Rust tests
cargo run -p jitmo-testset-runner # the spec's test list (add --worker <path> for the real router)
cd app && pnpm install && pnpm test && pnpm build
```

Run the app in development (from `app/`): `pnpm tauri dev`. Press Ctrl+Alt+J to open the bar.
The real router needs the model files in `models/router/` (see `models-src/router/README.md`);
without them the app falls back to its rule-based router.

The database is encrypted only in release builds (`--features encrypted-db`, implied by
`custom-protocol`), because building SQLCipher takes tens of minutes. CI checks that
configuration compiles.

## Rules the code keeps

- Every send needs a confirm. Changes to data run first and offer Undo for 10 seconds.
- A skill may only use the capabilities it declares; anything else is denied.
- A connector's check must prove the action really happened; a failed check is reported, never hidden.
- User words never reach a script: they go to fixed scripts through environment variables.
