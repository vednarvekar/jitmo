import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import * as bridge from "./bridge";
import Orb from "./Orb";
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
/** How long the pop-down animation runs before the window is hidden (see styles.css). */
const POP_DOWN_MS = 170;
/** One-tap ideas shown under an empty bar. */
const SUGGESTIONS = ["Open Spotify", "10-minute timer", "Pause music", "18% of 2,450"];

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
  // Dragging makes the window lose focus for a moment; that is not a click elsewhere.
  const dragUntil = useRef(0);

  // Pop-up and pop-down: the page plays the animation, then asks Rust to hide.
  const [anim, setAnim] = useState<"in" | "out" | "idle">("in");
  const closing = useRef(false);
  const playIn = useCallback(() => {
    closing.current = false;
    setAnim("idle");
    requestAnimationFrame(() => setAnim("in"));
  }, []);
  const closeBar = useCallback(() => {
    if (closing.current) return;
    closing.current = true;
    setAnim("out");
    setTimeout(() => bridge.hideBar(), POP_DOWN_MS);
  }, []);

  const resetBar = useCallback(() => {
    dispatch({ type: "reset" });
    setText("");
    setUndoUntil(0);
    historyAt.current = -1;
    inputRef.current?.focus();
  }, []);

  // The window is created hidden; show it once the first frame is on screen.
  useEffect(() => {
    const id = requestAnimationFrame(() => requestAnimationFrame(() => bridge.barReady()));
    return () => cancelAnimationFrame(id);
  }, []);

  // Core events and the "bar shown / lost focus" signals.
  useEffect(() => {
    const unlisten = [
      bridge.onCoreEvent((event) => {
        dispatch({ type: "core", event, now: Date.now() });
        if (event.type === "done" && event.undoable) setUndoUntil(Date.now() + UNDO_WINDOW_MS);
      }),
      bridge.onBarShown(() => {
        resetBar();
        playIn();
      }),
      bridge.onBarClose(closeBar),
      bridge.onBarBlurred(() => {
        if (Date.now() < dragUntil.current) return;
        if (hideOnBlur(stateRef.current)) closeBar();
      }),
    ];
    return () => {
      unlisten.forEach((p) => p.then((f) => f()));
    };
  }, [resetBar, playIn, closeBar]);

  // A finished result closes the bar by itself, unless the pointer is on it.
  useEffect(() => {
    const ms = autoHideMs(state);
    if (ms === null) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const arm = (delay: number) => {
      clearTimeout(timer);
      timer = setTimeout(() => {
        if (hover.current) arm(HOVER_GRACE_MS);
        else closeBar();
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
          closeBar();
        }
      } else if (
        (s.phase === "done" || s.phase === "failed") &&
        e.key.length === 1 &&
        !e.ctrlKey &&
        !e.altKey &&
        !e.metaKey
      ) {
        // The result replaced the input line; the first key typed brings it back.
        e.preventDefault();
        dispatch({ type: "reset" });
        setText(e.key);
        setTimeout(() => inputRef.current?.focus(), 0);
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

  // While a request runs or has finished, its status takes the place of the
  // input line, so the bar keeps its height. Only a confirm or choice card
  // makes it taller.
  const showStatus = state.phase === "working" || state.phase === "done" || state.phase === "failed";

  return (
    <div className="shell" ref={rootRef} data-anim={anim}>
      <div
        className="bar"
        data-phase={state.phase}
        data-tauri-drag-region
        onMouseEnter={() => (hover.current = true)}
        onMouseLeave={() => (hover.current = false)}
        onMouseDown={() => (dragUntil.current = Date.now() + 2_000)}
      >
        {/* Any empty part of the pill drags it; the input and buttons do not. */}
        <div className="row" data-tauri-drag-region>
          <Orb working={state.phase === "working"} />

          {showStatus ? (
            <div
              className={`status ${state.phase}`}
              role="status"
              aria-live="polite"
              title={state.phase === "working" ? undefined : "Type to ask something else"}
              onClick={() => state.phase !== "working" && resetBar()}
            >
              {state.phase === "working" && (
                <>
                  <span className="spinner" aria-hidden="true" />
                  <span className="message muted">Working…</span>
                </>
              )}
              {state.phase === "done" && (
                <>
                  <span className="icon" aria-hidden="true">
                    ✓
                  </span>
                  <span className="message">{state.message}</span>
                  {showUndo && (
                    <button
                      className="undo"
                      onClick={(e) => {
                        e.stopPropagation();
                        bridge.undo(state.id);
                      }}
                    >
                      Undo ({undoLeft})
                    </button>
                  )}
                </>
              )}
              {state.phase === "failed" && (
                <>
                  <span className="icon" aria-hidden="true">
                    !
                  </span>
                  <span className="message">{state.message}</span>
                </>
              )}
            </div>
          ) : (
            <input
              ref={inputRef}
              className="input"
              autoFocus
              spellCheck={false}
              aria-label="Ask jitmo"
              placeholder="Ask anything…"
              value={text}
              disabled={!typing}
              onChange={(e) => {
                setText(e.target.value);
                historyAt.current = -1;
              }}
              onKeyDown={onInputKey}
            />
          )}

          {canSend && (
          <button
            className="go"
            aria-label="Run"
            title="Run (Enter)"
            onClick={() => send(text)}
          >
            <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true">
              <path d="M4 12h14M13 6l6 6-6 6" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          </button>
          )}
          <button className="close" aria-label="Close" title="Close (Esc)" onClick={closeBar}>
            ×
          </button>
        </div>

        {(state.phase === "confirm" || state.phase === "choice") && (
          <div className="result" role="status" aria-live="polite" data-tauri-drag-region>
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
          </div>
        )}
      </div>

      {state.phase === "idle" && text === "" && (
        <div className="chips" data-tauri-drag-region>
          {SUGGESTIONS.map((s) => (
            <button key={s} className="chip" onClick={() => send(s)}>
              {s}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
