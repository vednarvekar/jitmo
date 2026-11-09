// The only file that talks to Tauri. Everything else is plain React and plain state.
//
// Browser preview: opened in a normal browser (`pnpm dev`, no Tauri around), the calls do
// nothing and events come from the console instead, e.g.
//   window.__jitmo.emit("core-event", { type: "failed", id: 1, message: "..." })
// so every state of the bar can be looked at without the desktop app.

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";
import type { CoreEvent } from "./state";

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

type Handler = (payload: unknown) => void;
const preview = new Map<string, Set<Handler>>();
if (!inTauri && typeof window !== "undefined") {
  (window as unknown as { __jitmo: unknown }).__jitmo = {
    emit: (name: string, payload?: unknown) => preview.get(name)?.forEach((h) => h(payload)),
  };
}

const invoke = <T>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
  if (inTauri) return tauriInvoke<T>(cmd, args);
  console.debug("[preview] invoke", cmd, args ?? "");
  return Promise.resolve(undefined as T);
};

const listen = <T>(name: string, cb: (e: { payload: T }) => void): Promise<UnlistenFn> => {
  if (inTauri) return tauriListen<T>(name, cb);
  const handler: Handler = (payload) => cb({ payload: payload as T });
  if (!preview.has(name)) preview.set(name, new Set());
  preview.get(name)?.add(handler);
  return Promise.resolve(() => preview.get(name)?.delete(handler));
};

export const submit = (id: number, text: string) => invoke<void>("submit", { id, text });
export const confirm = (id: number, approved: boolean) => invoke<void>("confirm", { id, approved });
export const choose = (id: number, choiceId: string) => invoke<void>("choose", { id, choiceId });
export const answer = (id: number, text: string) => invoke<void>("answer", { id, text });
export const cancel = (id: number) => invoke<void>("cancel", { id });
export const undo = (id: number) => invoke<void>("undo", { id });
export const hideBar = () => invoke<void>("hide_bar");
/** The bar's two small sounds: "open" and "close". */
export const barSound = (kind: "open" | "close") => invoke<void>("bar_sound", { kind });
export const resizeBar = (height: number, width: number) =>
  invoke<void>("resize_bar", { height, width });

export const onCoreEvent = (cb: (e: CoreEvent) => void): Promise<UnlistenFn> =>
  listen<CoreEvent>("core-event", (e) => cb(e.payload));

export const onBarShown = (cb: () => void): Promise<UnlistenFn> => listen("bar-shown", () => cb());

/** The bar window lost focus (the user clicked elsewhere). */
export const onBarBlurred = (cb: () => void): Promise<UnlistenFn> => listen("bar-blurred", () => cb());

/** The page has drawn; the window may be shown now (it is created hidden). */
export const barReady = () => invoke<void>("bar_ready");

/** Rust asks the bar to close itself (the hotkey was pressed while it was open),
 *  so the pop-down animation can play before the window is hidden. */
export const onBarClose = (cb: () => void): Promise<UnlistenFn> => listen("bar-close", () => cb());
