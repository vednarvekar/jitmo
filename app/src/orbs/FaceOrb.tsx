import { useEffect, useRef, useState } from "react";
import GreyOrb, { type GreyMood, type OrbPose } from "./GreyOrb";

const reduced = () =>
  typeof window !== "undefined" && window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;

/** What the bar is doing, as the orb needs to know it. */
export type BarPhase = "idle" | "working" | "confirm" | "choice" | "asking" | "done" | "failed";

/**
 * The owner's "AI orb" (GreyOrb: the grey sphere with white eyes) as a character on the
 * obsidian bar. Built on how Anki's Cozmo shows feeling with eyes alone (each feeling is a
 * set of eye shapes, glided between) and on Copilot's Mico (clear listening, thinking and
 * acknowledging states, and a reaction when tapped):
 *
 *   idle       its long routine: blinks, winks, looks around; follows the pointer
 *   listening  while the user types: wide eyes that slide along the line as it grows,
 *              and a small nod per key
 *   thinking   while working: narrowed eyes scanning, a gentle wobble
 *   asking     a question card is open: big eyes looking down at it, head tilted
 *   done       happy arcs and a hop
 *   failed     droopy eyes, a shake of the head, a little dimmer
 *   click      squeezes its eyes shut and squishes like jelly, sends a ripple, then smiles
 *   bar opens  wakes up: eyes open from closed lines
 */
export default function FaceOrb({
  phase,
  typed,
  pulse,
  awake,
  size = 34,
}: {
  phase: BarPhase;
  /** How many characters are typed (the eyes follow the line). */
  typed: number;
  /** Changes on every keystroke. */
  pulse: number;
  /** False while the bar is hidden or sliding in: the orb sleeps, then wakes. */
  awake: boolean;
  size?: number;
}) {
  const body = useRef<HTMLSpanElement>(null);
  const [ripples, setRipples] = useState<number[]>([]);
  const [poke, setPoke] = useState<"" | "squeeze" | "smile">("");
  const [waking, setWaking] = useState(false);

  // Waking up: closed eyes for a beat after the bar appears, then open.
  useEffect(() => {
    if (!awake) {
      setWaking(true);
      return;
    }
    const t = setTimeout(() => setWaking(false), reduced() ? 0 : 380);
    return () => clearTimeout(t);
  }, [awake]);

  // A nod per keystroke.
  useEffect(() => {
    if (!pulse || !body.current || reduced()) return;
    body.current.getAnimations().forEach((a) => a.id === "beat" && a.cancel());
    const a = body.current.animate(
      [
        { transform: "translateY(0) scale(1)" },
        { transform: "translateY(1px) scale(1.06, 0.96)" },
        { transform: "translateY(0) scale(1)" },
      ],
      { duration: 240, easing: "cubic-bezier(0.2, 0.9, 0.3, 1)" },
    );
    a.id = "beat";
  }, [pulse]);

  const play = () => {
    const id = Date.now();
    setRipples((r) => [...r.slice(-2), id]);
    setTimeout(() => setRipples((r) => r.filter((x) => x !== id)), 700);
    setPoke("squeeze");
    setTimeout(() => setPoke("smile"), 300);
    setTimeout(() => setPoke(""), 1300);
    if (body.current && !reduced()) {
      body.current.getAnimations().forEach((a) => a.cancel());
      // Anticipation (squash), stretch, then a wobble that settles.
      body.current.animate(
        [
          { transform: "scale(1, 1)" },
          { transform: "scale(1.22, 0.8)", offset: 0.18 },
          { transform: "scale(0.88, 1.14)", offset: 0.4 },
          { transform: "scale(1.06, 0.95)", offset: 0.62 },
          { transform: "scale(0.98, 1.02)", offset: 0.8 },
          { transform: "scale(1, 1)" },
        ],
        { duration: 620, easing: "ease-out" },
      );
    }
  };

  // What the face shows. Real states win over play, except a click during idle or typing.
  const asking = phase === "confirm" || phase === "choice" || phase === "asking";
  const base: OrbPose = waking
    ? "asleep"
    : phase === "working"
      ? "thinking"
      : asking
        ? "asking"
        : phase === "done"
          ? "done"
          : phase === "failed"
            ? "failed"
            : typed > 0
              ? "listening"
              : "idle";
  const playful = base === "idle" || base === "listening";
  const pose: OrbPose =
    playful && poke === "squeeze" ? "squeeze" : playful && poke === "smile" ? "done" : base;
  const mood: GreyMood =
    pose === "thinking" ? "working" : pose === "failed" ? "failed" : pose === "done" ? "done" : "idle";
  // Eyes slide along the line: 0 at the start, up to about a fifth of the orb at 40 letters.
  const look = pose === "listening" ? Math.min(typed, 40) * 0.5 : 0;

  return (
    <button
      className="face-orb"
      data-pose={pose}
      style={{ width: size, height: size }}
      aria-label="Jitmo"
      title="Hi, I'm Jitmo"
      onClick={play}
    >
      {ripples.map((id) => (
        <span key={id} className="face-ripple" aria-hidden="true" />
      ))}
      <span className="face-body" ref={body}>
        <span className="face-act" style={{ ["--look" as string]: look }}>
          <GreyOrb mood={mood} pose={pose} size={size} />
        </span>
      </span>
    </button>
  );
}
