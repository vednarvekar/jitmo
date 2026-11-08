import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import * as bridge from "./bridge";
import {
  UNDO_WINDOW_MS,
  autoHideMs,
  canConfirmNow,
  currentId,
  hideOnBlur,
  initialState,
  reduce,
} from "./state";

// Ids come from the UI so the "working" state is set before any event can
// arrive, even for a request that finishes in a millisecond.
let nextId = Date.now();

/** After the pointer leaves the bar, wait this long before closing a result. */
const HOVER_GRACE_MS = 2_500;
/** How many past requests Up/Down can step through. */
const HISTORY_SIZE = 30;

export default function App() {
  const [state, dispatch] = useReducer(reduce, initialState);
  const [text, setText] = useState("");
  const [undoUntil, setUndoUntil] = useState(0);
  const [now, setNow] = useState(Date.now());
  const inputRef = useRef<HTMLInputElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const stateRef = useRef(state);
  stateRef.current = state;
  // Past requests, newest first, and where Up/Down is in them (-1 = not browsing).
  const history = useRef<string[]>([]);
  const historyAt = useRef(-1);
  const hover = useRef(false);

  const resetBar = useCallback(() => {
    dispatch({ type: "reset" });
    setText("");
    setUndoUntil(0);
    historyAt.current = -1;
    inputRef.current?.focus();
  }, []);

  // Core events and the "bar shown / lost focus" signals.
  useEffect(() => {
    const unlisten = [
      bridge.onCoreEvent((event) => {
        dispatch({ type: "core", event, now: Date.now() });
        if (event.type === "done" && event.undoable) setUndoUntil(Date.now() + UNDO_WINDOW_MS);
      }),
      bridge.onBarShown(resetBar),
      bridge.onBarBlurred(() => {
        if (hideOnBlur(stateRef.current)) bridge.hideBar();
      }),
    ];
    return () => {
      unlisten.forEach((p) => p.then((f) => f()));
    };
  }, [resetBar]);

  // A finished result closes the bar by itself, unless the pointer is on it.
  useEffect(() => {
    const ms = autoHideMs(state);
    if (ms === null) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const arm = (delay: number) => {
      clearTimeout(timer);
      timer = setTimeout(() => {
        if (hover.current) arm(HOVER_GRACE_MS);
        else bridge.hideBar();
      }, delay);
    };
    arm(ms);
    return () => clearTimeout(timer);
  }, [state]);

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
    history.current = [trimmed, ...history.current.filter((h) => h !== trimmed)].slice(0, HISTORY_SIZE);
    historyAt.current = -1;
    dispatch({ type: "submitted", id });
    setUndoUntil(0);
    setText(""); // ready for the next request as soon as this one is sent
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
  const canSend = typing && text.trim().length > 0;

  const onInputKey = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter" && typing) {
      e.preventDefault();
      send(text);
    } else if (e.key === "ArrowUp" && typing && history.current.length > 0) {
      e.preventDefault();
      historyAt.current = Math.min(historyAt.current + 1, history.current.length - 1);
      setText(history.current[historyAt.current]);
    } else if (e.key === "ArrowDown" && typing && historyAt.current >= 0) {
      e.preventDefault();
      historyAt.current -= 1;
      setText(historyAt.current >= 0 ? history.current[historyAt.current] : "");
    }
  };

  return (
    <div className="shell" ref={rootRef}>
      <div
        className="bar"
        data-phase={state.phase}
        onMouseEnter={() => (hover.current = true)}
        onMouseLeave={() => (hover.current = false)}
      >
        {/* The strip at the top is the handle for dragging the bar. */}
        <div className="grip" data-tauri-drag-region title="Drag to move">
          <span className="grip-dot" data-tauri-drag-region />
        </div>

        <div className="row">
          <span className="mark" data-tauri-drag-region aria-hidden="true">
            ✦
          </span>
          <input
            ref={inputRef}
            className="input"
            autoFocus
            spellCheck={false}
            aria-label="Ask jitmo"
            placeholder="Ask jitmo…  try “open Spotify” or “set a 10-minute timer”"
            value={text}
            disabled={!typing}
            onChange={(e) => {
              setText(e.target.value);
              historyAt.current = -1;
              // Typing a new request clears the last result so the bar shrinks back.
              if (state.phase === "done" || state.phase === "failed") dispatch({ type: "reset" });
            }}
            onKeyDown={onInputKey}
          />
          <button
            className="go"
            aria-label="Run"
            title="Run (Enter)"
            disabled={!canSend}
            onClick={() => send(text)}
          >
            ↵
          </button>
          <button className="close" aria-label="Close" title="Close (Esc)" onClick={() => bridge.hideBar()}>
            ×
          </button>
        </div>

        <div className="result" role="status" aria-live="polite">
          {state.phase === "working" && (
            <div className="line muted">
              <span className="spinner" aria-hidden="true" /> Working…
            </div>
          )}

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
            <div className="line ok">
              <span className="icon" aria-hidden="true">
                ✓
              </span>
              <span className="message">{state.message}</span>
              {showUndo && (
                <button className="undo" onClick={() => bridge.undo(state.id)}>
                  Undo ({undoLeft})
                </button>
              )}
            </div>
          )}

          {state.phase === "failed" && (
            <div className="line error">
              <span className="icon" aria-hidden="true">
                !
              </span>
              <span className="message">{state.message}</span>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
