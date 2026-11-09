// The only file that talks to Tauri. Everything else is plain React and plain state.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { CoreEvent } from "./state";

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
