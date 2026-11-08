# app

The command bar: Tauri 2 shell (`src-tauri/`) and React + TypeScript UI (`src/`).

- The shell hosts Core on its own thread (`jitmo_core::service`) and opens the bar from a global hotkey.
- Default hotkey is `ctrl+alt+j` (Ctrl+Alt+Space was taken on the dev PC). Change it with the `hotkey` row in the `settings` table.
- Per SP11 the web view is destroyed 60 s after the bar is hidden, so idle memory stays small.
- Dev build only: connectors are fakes and the database is not encrypted yet.

## Run

```bash
pnpm install
pnpm build          # the Rust build needs app/dist to exist
pnpm tauri dev      # from app/, starts Vite and the shell
pnpm test           # state-machine tests (Vitest)
```

`cargo test --workspace` from the repo root runs the Rust tests.
