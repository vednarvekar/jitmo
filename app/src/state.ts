// Pure state for the command bar. No React, no Tauri, so it is easy to test.
//
// Mirrors the Core events in crates/ipc: every request ends in exactly one
// "done" or "failed" event.

export type Choice = { id: string; label: string };

export type CoreEvent =
  | { type: "started"; id: number }
  | { type: "confirm_needed"; id: number; text: string }
  | { type: "choice_needed"; id: number; question: string; choices: Choice[] }
  | { type: "done"; id: number; message: string; undoable: boolean }
  | { type: "failed"; id: number; message: string };

export type BarState =
  | { phase: "idle" }
  | { phase: "working"; id: number }
  | { phase: "confirm"; id: number; text: string; shownAt: number }
  | { phase: "choice"; id: number; question: string; choices: Choice[] }
  | { phase: "done"; id: number; message: string; undoable: boolean }
  | { phase: "failed"; id: number; message: string };

export type Action =
  | { type: "submitted"; id: number }
  | { type: "core"; event: CoreEvent; now: number }
  | { type: "reset" };

export const initialState: BarState = { phase: "idle" };

/** Id of the request the bar is currently showing, if any. */
export function currentId(s: BarState): number | null {
  return s.phase === "idle" ? null : s.id;
}

export function reduce(state: BarState, action: Action): BarState {
  switch (action.type) {
    case "reset":
      return initialState;
    case "submitted":
      return { phase: "working", id: action.id };
    case "core": {
      const e = action.event;
      // Ignore late events from an older request, except Undo results, which
      // arrive with the id of the finished request.
      const cur = currentId(state);
      if (cur !== null && e.id !== cur && e.type !== "done") return state;
      switch (e.type) {
        case "started":
          return { phase: "working", id: e.id };
        case "confirm_needed":
          return { phase: "confirm", id: e.id, text: e.text, shownAt: action.now };
        case "choice_needed":
          return { phase: "choice", id: e.id, question: e.question, choices: e.choices };
        case "done":
          return { phase: "done", id: e.id, message: e.message, undoable: e.undoable };
        case "failed":
          return { phase: "failed", id: e.id, message: e.message };
      }
    }
  }
}

/** Minimum time a confirm card must be visible before Enter confirms it, so a
 *  double-tapped Enter after typing cannot approve a send by accident. */
export const CONFIRM_GUARD_MS = 400;

export function canConfirmNow(state: BarState, now: number): boolean {
  return state.phase === "confirm" && now - state.shownAt >= CONFIRM_GUARD_MS;
}

/** How long the Undo button stays up after a data change (policy rule 5). */
export const UNDO_WINDOW_MS = 10_000;
