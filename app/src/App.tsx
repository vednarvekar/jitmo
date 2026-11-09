import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import * as bridge from "./bridge";
import { AskCard, ChoiceCard, ConfirmCard } from "./components/Cards";
import { Chevron, Cross } from "./components/Icons";
import Suggestions from "./components/Suggestions";
import VoiceButtons from "./components/VoiceButtons";
import { GreyOrb, type GreyMood } from "./orbs";
import {
  UNDO_WINDOW_MS,
  acceptsTyping,
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
const POP_DOWN_MS = 260;
/** Window width: compact by default, growing with the typed text up to a limit. */
const BAR_MIN_WIDTH = 440;
const BAR_MAX_WIDTH = 720;
/** Everything in the row that is not the text: window margin, orb, send button, gaps. */
const ROW_CHROME = 190;
let measureCtx: CanvasRenderingContext2D | null | undefined;
function wantedWidth(text: string): number {
  if (measureCtx === undefined) {
    measureCtx = document.createElement("canvas").getContext("2d");
    if (measureCtx) measureCtx.font = '500 14px "Manrope Variable", "Segoe UI", sans-serif';
  }
  const textPx = measureCtx ? measureCtx.measureText(text).width : text.length * 8;
  return Math.round(Math.min(BAR_MAX_WIDTH, Math.max(BAR_MIN_WIDTH, textPx + ROW_CHROME)));
}

/** The round close button beside the bar. Hidden for now (Esc closes); the code stays. */
const SHOW_CLOSE_BUTTON = false;
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
    // Two frames at the hidden position first, so the slide starts from a painted state.
    requestAnimationFrame(() => requestAnimationFrame(() => setAnim("in")));
  }, []);
  const closeBar = useCallback(() => {
    if (closing.current) return;
    closing.current = true;
    bridge.barSound("close");
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
    // The page's own blur is a second signal for "clicked somewhere else", in case the
    // window event does not arrive (the bar is a transparent always-on-top window).
    const onWindowBlur = () => {
      if (Date.now() < dragUntil.current) return;
      if (hideOnBlur(stateRef.current)) closeBar();
    };
    window.addEventListener("blur", onWindowBlur);
    return () => {
      unlisten.forEach((p) => p.then((f) => f()));
      window.removeEventListener("blur", onWindowBlur);
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

  // Keep the window as tall as the content, and as wide as the typed text needs
  // (compact by default, wider only when the text would not fit).
  const textRef = useRef(text);
  textRef.current = text;
  const syncSize = useCallback(() => {
    const el = rootRef.current;
    if (!el) return;
    bridge.resizeBar(Math.ceil(el.getBoundingClientRect().height), wantedWidth(textRef.current));
  }, []);
  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const ro = new ResizeObserver(syncSize);
    ro.observe(el);
    return () => ro.disconnect();
  }, [syncSize]);
  useEffect(syncSize, [text, syncSize]);

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
        const busy = ["working", "confirm", "choice", "asking"].includes(s.phase);
        if (id !== null && busy) {
          bridge.cancel(id);
        } else {
          closeBar();
        }
      } else if (
        (s.phase === "done" || s.phase === "failed") &&
        (e.key === "Backspace" || e.key === "Delete")
      ) {
        // Backspace on a result (e.g. "I can't do that yet.") clears it and the line,
        // so there is no need to click the bar first.
        e.preventDefault();
        dispatch({ type: "reset" });
        setText("");
        setTimeout(() => inputRef.current?.focus(), 0);
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

  const typing = acceptsTyping(state);
  const answering = state.phase === "asking";
  const undoLeft = Math.max(0, Math.ceil((undoUntil - now) / 1000));
  const showUndo = state.phase === "done" && state.undoable && undoLeft > 0;
  const canSend = typing && text.trim().length > 0;

  // While a question is open, what is typed answers it (even an empty answer is sent, so
  // Core can ask once more and then cancel). The input is cleared for the next question.
  const sendAnswer = useCallback((id: number, value: string) => {
    setText("");
    bridge.answer(id, value);
  }, []);

  const onInputKey = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter" && state.phase === "asking") {
      e.preventDefault();
      sendAnswer(state.id, text);
    } else if (e.key === "Enter" && typing) {
      e.preventDefault();
      send(text);
    } else if (e.key === "ArrowUp" && typing && !answering && history.current.length > 0) {
      e.preventDefault();
      historyAt.current = Math.min(historyAt.current + 1, history.current.length - 1);
      setText(history.current[historyAt.current]);
    } else if (e.key === "ArrowDown" && typing && !answering && historyAt.current >= 0) {
      e.preventDefault();
      historyAt.current -= 1;
      setText(historyAt.current >= 0 ? history.current[historyAt.current] : "");
    }
  };

  // While a request runs or has finished, its status takes the place of the
  // input line, so the bar keeps its height. Only a confirm or choice card
  // makes it taller.
  const showStatus = state.phase === "working" || state.phase === "done" || state.phase === "failed";

  // The "+" opens a row of one-tap suggestions. Off by default, to keep the bar clean.
  const [showChips, setShowChips] = useState(false);
  const mood: GreyMood =
    state.phase === "working" ||
    state.phase === "confirm" ||
    state.phase === "choice" ||
    state.phase === "asking"
      ? "working"
      : state.phase === "done"
        ? "done"
        : state.phase === "failed"
          ? "failed"
          : "idle";

  return (
    <div className="shell" ref={rootRef} data-anim={anim}>
      <div
        className="dock"
        data-tauri-drag-region
        onMouseEnter={() => (hover.current = true)}
        onMouseLeave={() => (hover.current = false)}
        onMouseDown={() => (dragUntil.current = Date.now() + 2_000)}
      >
        <div className="fx-layer">
          <div className="bar box" data-phase={state.phase} data-tauri-drag-region>
            <div className="circle-overlay" aria-hidden="true" />
            {/* Any empty part of the pill drags it; the input and buttons do not. */}
            <div className="row" data-tauri-drag-region>
              {/* The orb opens the suggestions. */}
              <button
                className="orb-btn"
                aria-label="Suggestions"
                aria-pressed={showChips}
                title="Suggestions"
                onClick={() => setShowChips((v) => !v)}
              >
                <GreyOrb mood={mood} size={34} />
              </button>

              {showStatus ? (
                <div
                  className={`status ${state.phase}`}
                  role="status"
                  aria-live="polite"
                  title={state.phase === "working" ? undefined : "Type to ask something else"}
                  onClick={() => state.phase !== "working" && resetBar()}
                >
                  {state.phase === "working" && <span className="message muted">Working…</span>}
                  {state.phase === "done" && (
                    <>
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
                  {state.phase === "failed" && <span className="message">{state.message}</span>}
                </div>
              ) : (
                <input
                  ref={inputRef}
                  className="input"
                  autoFocus
                  spellCheck={false}
                  aria-label={answering ? "Your answer" : "Ask Jitmo"}
                  placeholder={state.phase === "asking" ? state.hint : "Ask Jitmo"}
                  value={text}
                  disabled={!typing}
                  onChange={(e) => {
                    setText(e.target.value);
                    historyAt.current = -1;
                  }}
                  onKeyDown={onInputKey}
                />
              )}

              {canSend ? (
                <button
                  className="go"
                  aria-label={answering ? "Answer" : "Run"}
                  title={answering ? "Answer (Enter)" : "Run (Enter)"}
                  onClick={() => (state.phase === "asking" ? sendAnswer(state.id, text) : send(text))}
                >
                  <Chevron />
                </button>
              ) : (
                <VoiceButtons />
              )}
            </div>

            {(state.phase === "confirm" || state.phase === "choice" || state.phase === "asking") && (
              <div className="result" role="status" aria-live="polite" data-tauri-drag-region>
                {state.phase === "confirm" && <ConfirmCard id={state.id} text={state.text} />}
                {state.phase === "choice" && (
                  <ChoiceCard id={state.id} question={state.question} choices={state.choices} />
                )}
                {state.phase === "asking" && <AskCard id={state.id} question={state.question} answer={text} />}
              </div>
            )}
          </div>

          {SHOW_CLOSE_BUTTON && (
            <button className="closer box" aria-label="Close" title="Close (Esc)" onClick={closeBar}>
              <Cross />
              <div className="circle-overlay" aria-hidden="true" />
            </button>
          )}
        </div>
      </div>

      {showChips && state.phase === "idle" && text === "" && <Suggestions items={SUGGESTIONS} onPick={send} />}
    </div>
  );
}
