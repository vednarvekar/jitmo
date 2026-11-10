import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import * as bridge from "./bridge";
import { AskCard, ChoiceCard, ConfirmCard } from "./components/Cards";
import GlassSurfaceUntyped from "./components/glass/GlassSurface";
import { Chevron, Cross } from "./components/Icons";
import { SourceCard, SourceChip } from "./components/SourceChip";
import { currentDesign } from "./design";
import { splitSource } from "./source";
import Suggestions from "./components/Suggestions";
import VoiceButtons from "./components/VoiceButtons";
import { GreyOrb, type GreyMood } from "./orbs";
import { paletteVars } from "./orbs/LivingOrb";
import FaceOrb from "./orbs/FaceOrb";
import Hints from "./components/Hints";
import WorkingWord from "./components/WorkingWord";
import Reveal from "./components/Reveal";
import Answer, { isHelp } from "./components/Answer";
import { ArrowUp, Check, Copy } from "./components/Icons";
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
/** A message that will take more than one line gets "multi": the row aligns to the top then. */
const messageClass = (m: string) =>
  isHelp(m) ? "message multi rich" : m.length > 44 ? "message multi" : "message";

/** Window width for an answer: short ones keep the compact bar; longer ones widen so they
 *  wrap into about three lines, up to 680 px. The help reply always gets the wide card. */
function answerWidth(message: string): number {
  if (!message) return 0;
  if (isHelp(message)) return 600;
  if (message.length <= 60) return 0;
  const px = message.length * 7.4; // Geist 14.5 px averages about 7.4 px a character
  return Math.round(Math.min(680, Math.max(BAR_MIN_WIDTH, px / 3 + ROW_CHROME)));
}

/** The pasted component is untyped JS; it takes CSS sizes as well as numbers. */
const GlassSurface = GlassSurfaceUntyped as unknown as React.FC<
  Record<string, unknown> & { children?: React.ReactNode }
>;

/** Which look the bar uses: "obsidian", "glass2" or "classic". See design.ts. */
const DESIGN = currentDesign();
const OBSIDIAN = DESIGN === "obsidian";
const TEXT_FONT =
  DESIGN === "classic"
    ? '500 14px "Manrope Variable", "Segoe UI", sans-serif'
    : DESIGN === "obsidian"
      ? '420 15px "Geist Variable", "Segoe UI", sans-serif'
      : '450 14px "Geist Variable", "Segoe UI", sans-serif';

/** Window width: compact by default, growing with the typed text up to a limit. */
const BAR_MIN_WIDTH = 440;
const BAR_MAX_WIDTH = 720;
/** Everything in the row that is not the text: window margin, orb, send button, gaps. */
const ROW_CHROME = 190;
let measureCtx: CanvasRenderingContext2D | null | undefined;
function wantedWidth(text: string): number {
  if (measureCtx === undefined) {
    measureCtx = document.createElement("canvas").getContext("2d");
    if (measureCtx) measureCtx.font = TEXT_FONT;
  }
  const textPx = measureCtx ? measureCtx.measureText(text).width : text.length * 8;
  return Math.round(Math.min(BAR_MAX_WIDTH, Math.max(BAR_MIN_WIDTH, textPx + ROW_CHROME)));
}

/** The round close button beside the bar. Hidden for now (Esc closes); the code stays. */
const SHOW_CLOSE_BUTTON = false;
/** One-tap ideas shown under an empty bar. */
const SUGGESTIONS = ["Open Spotify", "10-minute timer", "Pause music", "18% of 2,450"];

/**
 * The glass the bar's content sits on. "classic" is the liquid-glass pill as before;
 * "glass2" is GlassSurface. `block` is true when the content is more than one line (a long
 * answer, an open source card): the pill then becomes a rounded panel.
 */
function Surface({
  phase,
  block,
  children,
}: {
  phase: string;
  block: boolean;
  children: React.ReactNode;
}) {
  if (DESIGN === "obsidian") {
    const panel = block || ["confirm", "choice", "asking"].includes(phase);
    return (
      // The bloom sits outside the glass: inside it, the glass's own layer would put the
      // bloom over the bar's background and wash out the text.
      <div className="ob-wrap" data-phase={phase} data-panel={panel}>
        <span className="ob-glow" aria-hidden="true" />
        <div className="bar-ob" data-phase={phase} data-panel={panel} data-tauri-drag-region>
          <span className="ob-ring" aria-hidden="true" />
          {children}
        </div>
      </div>
    );
  }
  if (DESIGN === "glass2") {
    const panel = block || ["confirm", "choice", "asking"].includes(phase);
    return (
      <div className="bar-g2" data-phase={phase} data-tauri-drag-region>
        <GlassSurface
          className="g2-surface"
          width="100%"
          height="auto"
          borderRadius={panel ? 22 : 9999}
          borderWidth={0.07}
          brightness={52}
          opacity={0.93}
          blur={11}
          displace={0.4}
          backgroundOpacity={0.8}
          saturation={1.4}
          distortionScale={-150}
          redOffset={0}
          greenOffset={8}
          blueOffset={16}
          mixBlendMode="screen"
        >
          {children}
        </GlassSurface>
      </div>
    );
  }
  return (
    <div className="bar box" data-phase={phase} data-tauri-drag-region>
      <div className="circle-overlay" aria-hidden="true" />
      {children}
    </div>
  );
}

/** Copies an answer; the icon turns into a tick for a moment. */
function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      className="tool copy"
      aria-label={copied ? "Copied" : "Copy the answer"}
      title={copied ? "Copied" : "Copy"}
      onClick={(e) => {
        e.stopPropagation();
        navigator.clipboard
          ?.writeText(text)
          .then(() => {
            setCopied(true);
            setTimeout(() => setCopied(false), 1400);
          })
          .catch(() => undefined);
      }}
    >
      {copied ? <Check /> : <Copy />}
    </button>
  );
}

export default function App() {
  const [state, dispatch] = useReducer(reduce, initialState);
  // A timer or reminder that is ringing: it owns the bar until Esc, Enter or Backspace.
  const [alarm, setAlarm] = useState<string | null>(null);
  const alarmRef = useRef<string | null>(null);
  alarmRef.current = alarm;
  const [text, setText] = useState("");
  const [asked, setAsked] = useState("");
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
    // Closing with a question, confirm or choice still open cancels it, so the engine is
    // never left waiting for an answer nobody can give.
    const open = stateRef.current;
    if (["confirm", "choice", "asking"].includes(open.phase)) {
      const id = currentId(open);
      if (id !== null) bridge.cancel(id);
    }
    if (alarmRef.current !== null) {
      bridge.stopAlarm();
      setAlarm(null);
    }
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
      // An answer or a choice was sent: show that the request is running again.
      (() => {
        const onResumed = () => dispatch({ type: "resumed" });
        window.addEventListener(bridge.RESUMED, onResumed);
        return Promise.resolve(() => window.removeEventListener(bridge.RESUMED, onResumed));
      })(),
      bridge.onBarClose(closeBar),
      // A timer or reminder came due: show it as a result, unless the user is in the middle
      // of something (a running task, a question or a confirm), which must not be replaced.
      bridge.onBarNotice((message) => {
        if (!["idle", "done", "failed"].includes(stateRef.current.phase)) return;
        dispatch({
          type: "core",
          event: { type: "done", id: 0, message, undoable: false },
          now: Date.now(),
        });
      }),
      // An alarm takes the bar even from a question in progress: that question is dropped.
      bridge.onBarAlarm((message) => {
        const open = stateRef.current;
        const id = currentId(open);
        if (id !== null && ["confirm", "choice", "asking", "working"].includes(open.phase)) {
          bridge.cancel(id);
        }
        dispatch({ type: "reset" });
        setText("");
        setAlarm(message);
        // Pop up the way the bar always does (also undoes a pop-down that was under way).
        playIn();
      }),
      bridge.onBarBlurred(() => {
        if (Date.now() < dragUntil.current || alarmRef.current !== null) return;
        if (hideOnBlur(stateRef.current)) closeBar();
      }),
    ];
    // The page's own blur is a second signal for "clicked somewhere else", in case the
    // window event does not arrive (the bar is a transparent always-on-top window).
    const onWindowBlur = () => {
      if (Date.now() < dragUntil.current || alarmRef.current !== null) return;
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
  // A long answer widens the bar too, so it reads in a few comfortable lines instead of a
  // tall narrow column (obsidian look).
  const shownRef = useRef("");
  shownRef.current =
    OBSIDIAN && (state.phase === "done" || state.phase === "failed") ? state.message : "";
  const syncSize = useCallback(() => {
    const el = rootRef.current;
    if (!el) return;
    const width = Math.max(wantedWidth(textRef.current), answerWidth(shownRef.current));
    bridge.resizeBar(Math.ceil(el.getBoundingClientRect().height), width);
  }, []);
  useEffect(syncSize, [state, syncSize]);
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
    setAsked(trimmed);
    setUndoUntil(0);
    setText(""); // ready for the next request as soon as this one is sent
    bridge.submit(id, trimmed);
  }, []);

  // Keys that work from anywhere in the bar.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const s = stateRef.current;
      const id = currentId(s);
      if (alarmRef.current !== null) {
        // Ringing: Esc, Enter, Backspace (or Delete or Space) stop it and put the bar away.
        // Other keys do nothing, so a half-typed word elsewhere cannot land in the bar.
        e.preventDefault();
        if (["Escape", "Enter", "Backspace", "Delete", " "].includes(e.key)) closeBar();
      } else if (e.key === "Tab") {
        // Tab never moves focus to a button; the input keeps it.
        e.preventDefault();
        inputRef.current?.focus();
      } else if (e.key === "Escape") {
        e.preventDefault();
        if (s.phase === "working") {
          // A running action cannot be interrupted half-way, but Esc must never feel dead:
          // ask the engine to stop before its next step and put the bar away.
          if (id !== null) bridge.cancel(id);
          closeBar();
        } else if (id !== null && ["confirm", "choice", "asking"].includes(s.phase)) {
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
  }, [closeBar]);

  // A ringing alarm stops by itself after a minute and a half (the sound does too).
  useEffect(() => {
    if (alarm === null) return;
    const t = setTimeout(closeBar, 90_000);
    return () => clearTimeout(t);
  }, [alarm, closeBar]);

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
  const showStatus =
    alarm !== null ||
    state.phase === "working" ||
    state.phase === "done" ||
    state.phase === "failed";

  // A looked-up answer carries its credit at the end; it is shown as a round button instead.
  const answer = state.phase === "done" ? splitSource(state.message) : null;
  const [sourceOpen, setSourceOpen] = useState(false);
  useEffect(() => setSourceOpen(false), [state]);

  // The "+" opens a row of one-tap suggestions. Off by default, to keep the bar clean.
  const [showChips, setShowChips] = useState(false);
  // The living orb: a beat per keystroke, and its colour mood shared with the edge glow.
  const [beat, setBeat] = useState(0);
  // The edge light's colours (Aurora); LivingOrb could change them, the face orb does not.
  const palette = 0;
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
    <div
      className="shell"
      ref={rootRef}
      data-anim={anim}
      data-typing={text.length > 0 || undefined}
      style={OBSIDIAN ? paletteVars(palette) : undefined}
    >
      <div
        className="dock"
        data-tauri-drag-region
        onMouseEnter={() => (hover.current = true)}
        onMouseLeave={() => (hover.current = false)}
        onMouseDown={(e) => {
          dragUntil.current = Date.now() + 2_000;
          // Keyboard focus stays in the input: clicking anywhere else in the bar
          // (a card, a chip, empty space) must not take it.
          if (!(e.target instanceof HTMLInputElement)) e.preventDefault();
        }}
      >
        <div className="fx-layer">
          <Surface
            phase={alarm !== null ? "alarm" : state.phase}
            block={
              sourceOpen ||
              ((state.phase === "done" || state.phase === "failed") &&
                messageClass(answer ? answer.body : state.message) !== "message")
            }
          >
            {/* What was asked stays visible, so a wrong answer or a confirm can be checked. */}
            {asked && alarm === null && ["confirm", "choice", "asking", "done", "failed"].includes(state.phase) && (
              <div className="asked" title={asked} data-tauri-drag-region>
                {asked}
              </div>
            )}
            {/* Any empty part of the pill drags it; the input and buttons do not. */}
            <div className="row" data-tauri-drag-region>
              {OBSIDIAN ? (
                // The owner's face orb. (LivingOrb, the colour orb, is kept in orbs/ unused.)
                <FaceOrb
                  phase={alarm !== null ? "done" : state.phase}
                  typed={text.length}
                  pulse={beat}
                  awake={anim === "in"}
                  size={32}
                />
              ) : (
                /* Classic and glass2: the grey orb opens the suggestions. */
                <button
                  className="orb-btn"
                  aria-label="Suggestions"
                  aria-pressed={showChips}
                  title="Suggestions"
                  onClick={() => setShowChips((v) => !v)}
                >
                  <GreyOrb mood={mood} size={34} />
                </button>
              )}

              {showStatus ? (
                <div
                  className={`status ${alarm !== null ? "done alarm" : state.phase}`}
                  role={alarm !== null ? "alert" : "status"}
                  aria-live={alarm !== null ? "assertive" : "polite"}
                  title={state.phase === "working" ? undefined : "Type to ask something else"}
                  onClick={() => {
                    if (alarm !== null) closeBar();
                    else if (state.phase !== "working") resetBar();
                  }}
                >
                  {alarm !== null && (
                    <>
                      <span className="message alarm-text" title={alarm}>
                        {alarm}
                      </span>
                      <span className="alarm-hint" aria-hidden="true">
                        Esc to stop
                      </span>
                    </>
                  )}
                  {alarm === null && state.phase === "working" && (
                    <span className="message muted working-text">
                      {OBSIDIAN ? <WorkingWord /> : "Working…"}
                    </span>
                  )}
                  {state.phase === "done" && answer && (
                    <>
                      <span className={messageClass(answer.body)} title={answer.body}>
                        {OBSIDIAN ? <Answer text={answer.body} onRun={send} /> : answer.body}
                        {answer.source && (
                          <SourceChip
                            source={answer.source}
                            open={sourceOpen}
                            onToggle={() => setSourceOpen((v) => !v)}
                          />
                        )}
                      </span>
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
                  {state.phase === "failed" && <span className={messageClass(state.message)} title={state.message}>
                        {OBSIDIAN ? <Reveal text={state.message} /> : state.message}
                      </span>}
                </div>
              ) : (
                <div className="field">
                {OBSIDIAN && (
                  <Hints
                    visible={text === ""}
                    fixed={
                      state.phase === "asking"
                        ? state.hint
                        : state.phase === "choice"
                          ? "pick one below, or press its number"
                          : state.phase === "confirm"
                            ? "Enter to go ahead, Esc to cancel"
                            : undefined
                    }
                  />
                )}
                <input
                  ref={inputRef}
                  className="input"
                  autoFocus
                  spellCheck={false}
                  aria-label={answering ? "Your answer" : "Ask Jitmo"}
                  placeholder={OBSIDIAN ? "" : state.phase === "asking" ? state.hint : "Ask Jitmo"}
                  value={text}
                  disabled={!typing}
                  onChange={(e) => {
                    setText(e.target.value);
                    historyAt.current = -1;
                    setBeat((b) => b + 1);
                  }}
                  onBlur={() => {
                    // Focus moving to another part of this page goes straight back to the
                    // input. (Losing the whole window is handled elsewhere: it hides the bar.)
                    if (document.hasFocus()) setTimeout(() => inputRef.current?.focus(), 0);
                  }}
                  onKeyDown={onInputKey}
                />
                </div>
              )}

              {canSend ? (
                <button
                  className="go"
                  aria-label={answering ? "Answer" : "Run"}
                  title={answering ? "Answer (Enter)" : "Run (Enter)"}
                  onClick={() => (state.phase === "asking" ? sendAnswer(state.id, text) : send(text))}
                >
                  {OBSIDIAN ? <ArrowUp /> : <Chevron />}
                </button>
              ) : OBSIDIAN && answer ? (
                <CopyButton text={answer.body} />
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

            {sourceOpen && answer?.source && <SourceCard source={answer.source} />}
          </Surface>

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
