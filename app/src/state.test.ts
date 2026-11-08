import { describe, expect, it } from "vitest";
import {
  BarState,
  CONFIRM_GUARD_MS,
  CoreEvent,
  canConfirmNow,
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
