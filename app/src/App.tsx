import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import * as bridge from "./bridge";
import {
  UNDO_WINDOW_MS,
  canConfirmNow,
  currentId,
  initialState,
  reduce,
} from "./state";

// Ids come from the UI so the "working" state is set before any event can
// arrive, even for a request that finishes in a millisecond.
let nextId = Date.now();

export default function App() {
  const [state, dispatch] = useReducer(reduce, initialState);
  const [text, setText] = useState("");
  const [undoUntil, setUndoUntil] = useState(0);
  const [now, setNow] = useState(Date.now());
  const inputRef = useRef<HTMLInputElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const stateRef = useRef(state);
  stateRef.current = state;

  // Core events and the "bar shown" signal.
  useEffect(() => {
    const unlisten = [
      bridge.onCoreEvent((event) => {
        dispatch({ type: "core", event, now: Date.now() });
        if (event.type === "done" && event.undoable) setUndoUntil(Date.now() + UNDO_WINDOW_MS);
      }),
      bridge.onBarShown(() => {
        dispatch({ type: "reset" });
        setText("");
        setUndoUntil(0);
        inputRef.current?.focus();
      }),
    ];
    return () => {
      unlisten.forEach((p) => p.then((f) => f()));
    };
  }, []);

  // Tick for the Undo countdown.
  useEffect(() => {
    if (undoUntil <= Date.now()) return;
    const t = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(t);
  }, [undoUntil]);

  // Keep the window as tall as the content.
  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => bridge.resizeBar(Math.ceil(el.getBoundingClientRect().height)));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const send = useCallback((value: string) => {
    const trimmed = value.trim();
    if (!trimmed) return;
    const id = ++nextId;
    dispatch({ type: "submitted", id });
    setUndoUntil(0);
    bridge.submit(id, trimmed);
  }, []);

  // Keys that work from anywhere in the bar.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const s = stateRef.current;
      const id = currentId(s);
      if (e.key === "Escape") {
        e.preventDefault();
        if (id !== null && (s.phase === "working" || s.phase === "confirm" || s.phase === "choice")) {
          bridge.cancel(id);
        } else {
          bridge.hideBar();
        }
      } else if (e.key === "Enter" && s.phase === "confirm") {
        e.preventDefault();
        if (canConfirmNow(s, Date.now())) bridge.confirm(s.id, true);
      } else if (s.phase === "choice" && /^[1-9]$/.test(e.key)) {
        const choice = s.choices[Number(e.key) - 1];
        if (choice) bridge.choose(s.id, choice.id);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const typing = state.phase === "idle" || state.phase === "done" || state.phase === "failed";
  const undoLeft = Math.max(0, Math.ceil((undoUntil - now) / 1000));
  const showUndo = state.phase === "done" && state.undoable && undoLeft > 0;

  return (
    <div className="bar" ref={rootRef}>
      <input
        ref={inputRef}
        className="input"
        autoFocus
        spellCheck={false}
        placeholder="Ask jitmo… (dev build: only “open app” is real)"
        value={text}
        disabled={!typing}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && typing) {
            e.preventDefault();
            send(text);
          }
        }}
      />

      {state.phase === "working" && <div className="line muted">Working…</div>}

      {state.phase === "confirm" && (
        <div className="card">
          <div className="line">{state.text}</div>
          <div className="buttons">
            <button className="primary" onClick={() => bridge.confirm(state.id, true)}>
              Confirm <kbd>Enter</kbd>
            </button>
            <button onClick={() => bridge.confirm(state.id, false)}>
              Cancel <kbd>Esc</kbd>
            </button>
          </div>
        </div>
      )}

      {state.phase === "choice" && (
        <div className="card">
          <div className="line">{state.question}</div>
          <div className="buttons column">
            {state.choices.map((c, i) => (
              <button key={c.id} onClick={() => bridge.choose(state.id, c.id)}>
                <kbd>{i + 1}</kbd> {c.label}
              </button>
            ))}
          </div>
        </div>
      )}

      {state.phase === "done" && (
        <div className="line">
          {state.message}
          {showUndo && (
            <button className="undo" onClick={() => bridge.undo(state.id)}>
              Undo ({undoLeft})
            </button>
          )}
        </div>
      )}

      {state.phase === "failed" && <div className="line error">{state.message}</div>}
    </div>
  );
}
