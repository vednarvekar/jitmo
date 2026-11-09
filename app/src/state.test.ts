import { describe, expect, it } from "vitest";
import {
  BarState,
  CONFIRM_GUARD_MS,
  CoreEvent,
  acceptsTyping,
  isAnswering,
  UNDO_WINDOW_MS,
  autoHideMs,
  canConfirmNow,
  hideOnBlur,
  initialState,
  reduce,
} from "./state";

const core = (event: CoreEvent, now = 0) => ({ type: "core" as const, event, now });

describe("command bar state", () => {
  it("starts idle and goes to working on submit", () => {
    expect(initialState.phase).toBe("idle");
    expect(reduce(initialState, { type: "submitted", id: 1 })).toEqual({ phase: "working", id: 1 });
  });

  it("T01 path: started then done", () => {
    let s: BarState = reduce(initialState, { type: "submitted", id: 1 });
    s = reduce(s, core({ type: "started", id: 1 }));
    s = reduce(s, core({ type: "done", id: 1, message: "Done.", undoable: false }));
    expect(s).toEqual({ phase: "done", id: 1, message: "Done.", undoable: false });
  });

  it("shows a confirm card and then the result", () => {
    let s: BarState = reduce(initialState, { type: "submitted", id: 2 });
    s = reduce(s, core({ type: "confirm_needed", id: 2, text: 'Send "hi" to ved?' }, 1000));
    expect(s).toEqual({ phase: "confirm", id: 2, text: 'Send "hi" to ved?', shownAt: 1000 });
    s = reduce(s, core({ type: "done", id: 2, message: "Cancelled.", undoable: false }));
    expect(s.phase).toBe("done");
  });

  it("shows a choice question", () => {
    let s: BarState = reduce(initialState, { type: "submitted", id: 3 });
    const choices = [{ id: "a", label: "open x" }];
    s = reduce(s, core({ type: "choice_needed", id: 3, question: "What?", choices }));
    expect(s).toEqual({ phase: "choice", id: 3, question: "What?", choices });
  });

  it("shows failures", () => {
    let s: BarState = reduce(initialState, { type: "submitted", id: 4 });
    s = reduce(s, core({ type: "failed", id: 4, message: "I can't do that yet." }));
    expect(s).toEqual({ phase: "failed", id: 4, message: "I can't do that yet." });
  });

  it("ignores late events from an older request", () => {
    const s: BarState = { phase: "working", id: 5 };
    expect(reduce(s, core({ type: "confirm_needed", id: 4, text: "old" }))).toBe(s);
    expect(reduce(s, core({ type: "failed", id: 4, message: "old" }))).toBe(s);
  });

  it("accepts an Undo result that carries the finished request id", () => {
    const s: BarState = { phase: "done", id: 6, message: "Done.", undoable: true };
    const next = reduce(s, core({ type: "done", id: 6, message: "Undone.", undoable: false }));
    expect(next).toEqual({ phase: "done", id: 6, message: "Undone.", undoable: false });
  });

  it("reset returns to idle", () => {
    expect(reduce({ phase: "failed", id: 1, message: "x" }, { type: "reset" })).toEqual(initialState);
  });

  it("Enter cannot confirm during the guard time", () => {
    const s: BarState = { phase: "confirm", id: 7, text: "t", shownAt: 1000 };
    expect(canConfirmNow(s, 1000 + CONFIRM_GUARD_MS - 1)).toBe(false);
    expect(canConfirmNow(s, 1000 + CONFIRM_GUARD_MS)).toBe(true);
    expect(canConfirmNow({ phase: "working", id: 7 }, 99999)).toBe(false);
  });
});

describe("auto-hide and focus rules", () => {
  it("closes a plain result after 5 seconds, errors after 8", () => {
    expect(autoHideMs({ phase: "done", id: 1, message: "Done.", undoable: false })).toBe(5_000);
    expect(autoHideMs({ phase: "failed", id: 1, message: "No." })).toBe(8_000);
  });

  it("keeps the bar up until Undo has expired", () => {
    expect(autoHideMs({ phase: "done", id: 1, message: "Done.", undoable: true })).toBe(UNDO_WINDOW_MS + 1_000);
  });

  it("never closes by itself while the user is mid-task or nothing is shown", () => {
    expect(autoHideMs({ phase: "idle" })).toBeNull();
    expect(autoHideMs({ phase: "working", id: 1 })).toBeNull();
    expect(autoHideMs({ phase: "confirm", id: 1, text: "Send?", shownAt: 0 })).toBeNull();
    expect(autoHideMs({ phase: "choice", id: 1, question: "Which?", choices: [] })).toBeNull();
  });

  it("losing focus closes the bar only when nothing needs the user", () => {
    expect(hideOnBlur({ phase: "idle" })).toBe(true);
    expect(hideOnBlur({ phase: "done", id: 1, message: "ok", undoable: false })).toBe(true);
    expect(hideOnBlur({ phase: "working", id: 1 })).toBe(false);
    expect(hideOnBlur({ phase: "confirm", id: 1, text: "Send?", shownAt: 0 })).toBe(false);
    expect(hideOnBlur({ phase: "choice", id: 1, question: "Which?", choices: [] })).toBe(false);
  });
});

describe("reading time", () => {
  it("gives long messages longer on screen", () => {
    const long = "x".repeat(140); // 100 characters past 40
    expect(autoHideMs({ phase: "done", id: 1, message: long, undoable: false })).toBe(5_000 + 6_000);
    expect(autoHideMs({ phase: "failed", id: 1, message: long })).toBe(8_000 + 6_000);
  });
});

describe("free-text question (asking)", () => {
  const ask: CoreEvent = { type: "text_needed", id: 7, question: "Which song?", hint: "Type your answer" };

  it("shows the question, then the result", () => {
    let s: BarState = reduce(initialState, { type: "submitted", id: 7 });
    s = reduce(s, core(ask));
    expect(s).toEqual({ phase: "asking", id: 7, question: "Which song?", hint: "Type your answer" });
    s = reduce(s, core({ type: "done", id: 7, message: "Done.", undoable: false }));
    expect(s.phase).toBe("done");
  });

  it("stays asking when Core asks again after an empty answer, with the new hint", () => {
    let s: BarState = reduce(initialState, { type: "submitted", id: 7 });
    s = reduce(s, core(ask));
    s = reduce(s, core({ ...ask, hint: "Type something" } as CoreEvent));
    expect(s).toMatchObject({ phase: "asking", hint: "Type something" });
  });

  it("a cancelled question ends in the Cancelled. result", () => {
    let s: BarState = reduce(initialState, { type: "submitted", id: 7 });
    s = reduce(s, core(ask));
    s = reduce(s, core({ type: "done", id: 7, message: "Cancelled.", undoable: false }));
    expect(s).toEqual({ phase: "done", id: 7, message: "Cancelled.", undoable: false });
  });

  it("ignores a question from an older request", () => {
    const s = reduce(initialState, { type: "submitted", id: 8 });
    expect(reduce(s, core(ask))).toBe(s);
  });

  it("never closes the bar by itself or on losing focus while a question is open", () => {
    const s: BarState = { phase: "asking", id: 7, question: "Which song?", hint: "" };
    expect(autoHideMs(s)).toBeNull();
    expect(hideOnBlur(s)).toBe(false);
  });

  it("the input takes typing as an answer, not as a new request", () => {
    const s: BarState = { phase: "asking", id: 7, question: "Which song?", hint: "" };
    expect(acceptsTyping(s)).toBe(true);
    expect(isAnswering(s)).toBe(true);
    expect(isAnswering({ phase: "idle" })).toBe(false);
    expect(acceptsTyping({ phase: "working", id: 1 })).toBe(false);
    expect(acceptsTyping({ phase: "confirm", id: 1, text: "x", shownAt: 0 })).toBe(false);
  });
});
