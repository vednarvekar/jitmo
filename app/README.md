# app

The command bar: a Tauri 2 shell (`src-tauri/`) and a React + TypeScript page (`src/`).

## How it works

- The shell opens the bar from a global hotkey (default `ctrl+alt+j`; change it with the `hotkey` row in the `settings` table).
- Core runs behind a per-user named pipe; the page talks to it only through the Tauri commands in `src-tauri/src/commands.rs`.
- The bar window sits on the taskbar edge. The page slides the bar up from behind the taskbar and back down before the window is hidden.
- Per SP11 the web view is destroyed 60 seconds after the bar is hidden, so idle memory stays small.
- The reminder scheduler runs in the shell and shows Windows notifications.

## Layout

```
src/
  main.tsx, App.tsx      entry and the bar itself (state wiring, keys, window animation)
  state.ts               pure state machine (no React, no Tauri), with its tests
  bridge.ts              the only file that talks to Tauri
  components/            Cards (confirm, choice), Suggestions, VoiceButtons, Icons
  orbs/                  GreyOrb (used), AiOrb and RingOrb (kept as alternatives)
  styles/                tokens, glass, controls, motion (imported in that order)
src-tauri/src/
  main.rs                entry and builder
  bar.rs                 the bar window: placement, saved position, animation timing
  commands.rs            the Tauri commands the page may call
  registry.rs            skills, router and connector registration
  storage.rs             database opening and the key
  scheduler.rs           reminder notifications
  core_client.rs         the pipe client that reaches Core
```

## Run

```bash
pnpm install
pnpm build          # the Rust build needs app/dist to exist
pnpm tauri dev      # starts Vite and the shell
pnpm test           # state-machine tests (Vitest)
```

`cargo test --workspace` from the repo root runs the Rust tests.
