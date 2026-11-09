// Pure state for the command bar. No React, no Tauri, so it is easy to test.
//
// Mirrors the Core events in crates/ipc: every request ends in exactly one
// "done" or "failed" event.

export type Choice = { id: string; label: string };

export type CoreEvent =
  | { type: "started"; id: number }
  | { type: "confirm_needed"; id: number; text: string }
  | { type: "choice_needed"; id: number; question: string; choices: Choice[] }
  | { type: "text_needed"; id: number; question: string; hint: string }
  | { type: "done"; id: number; message: string; undoable: boolean }
  | { type: "failed"; id: number; message: string };

export type BarState =
  | { phase: "idle" }
  | { phase: "working"; id: number }
  | { phase: "confirm"; id: number; text: string; shownAt: number }
  | { phase: "choice"; id: number; question: string; choices: Choice[] }
  | { phase: "asking"; id: number; question: string; hint: string }
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
        case "text_needed":
          return { phase: "asking", id: e.id, question: e.question, hint: e.hint };
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

/** How long a finished result stays up before the bar closes itself. Undo keeps
 *  it up until the Undo window has passed, and errors stay a little longer. */
export const RESULT_HIDE_MS = 5_000;
export const ERROR_HIDE_MS = 8_000;

/** Extra time for long messages: about 60 ms for each character past 40. */
function readingTime(message: string): number {
  return Math.max(0, message.length - 40) * 60;
}

/** Milliseconds until the bar should close by itself, or null if it must stay
 *  (the user is mid-task: working, confirming, choosing or answering a question, or
 *  nothing is shown). */
export function autoHideMs(s: BarState): number | null {
  switch (s.phase) {
    case "done":
      return s.undoable ? UNDO_WINDOW_MS + 1_000 : RESULT_HIDE_MS + readingTime(s.message);
    case "failed":
      return ERROR_HIDE_MS + readingTime(s.message);
    default:
      return null;
  }
}

/** Does the input line take typing? Yes when idle, after a result, and while a free-text
 *  question is open (what is typed is then the answer, not a new request). */
export function acceptsTyping(s: BarState): boolean {
  return s.phase === "idle" || s.phase === "done" || s.phase === "failed" || s.phase === "asking";
}

/** Is the typed text an answer to Core's question rather than a new request? */
export function isAnswering(s: BarState): boolean {
  return s.phase === "asking";
}

/** Should losing focus close the bar? Everywhere except while a task is running (its answer
 *  would be lost). A question or confirm that is left open is cancelled when the bar closes
 *  (see `closeBar` in App), so clicking away never leaves the engine waiting. */
export function hideOnBlur(s: BarState): boolean {
  return s.phase !== "working";
}
