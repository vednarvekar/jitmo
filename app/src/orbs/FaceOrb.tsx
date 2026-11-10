import { useEffect, useRef, useState } from "react";
import GreyOrb, { type GreyMood } from "./GreyOrb";

const reduced = () =>
  typeof window !== "undefined" && window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;

/**
 * The owner's "AI orb" (GreyOrb: the grey sphere with the white eyes that blink, wink, look
 * around and follow the pointer) dressed for the obsidian bar: brighter, with a soft halo so
 * it reads on dark glass, a small beat on every keystroke, and a playful click: it squishes
 * like jelly, sends out a ripple and smiles for a moment. No suggestions menu.
 */
export default function FaceOrb({
  mood,
  pulse,
  size = 34,
}: {
  mood: GreyMood;
  pulse: number;
  size?: number;
}) {
  const body = useRef<HTMLSpanElement>(null);
  const [ripples, setRipples] = useState<number[]>([]);
  const [smiling, setSmiling] = useState(false);

  // A beat per keystroke: quick swell, spring back.
  useEffect(() => {
    if (!pulse || !body.current || reduced()) return;
    body.current.getAnimations().forEach((a) => a.id === "beat" && a.cancel());
    const a = body.current.animate(
      [{ transform: "scale(1)" }, { transform: "scale(1.08)" }, { transform: "scale(1)" }],
      { duration: 260, easing: "cubic-bezier(0.2, 0.9, 0.3, 1)" },
    );
    a.id = "beat";
  }, [pulse]);

  const play = () => {
    const id = Date.now();
    setRipples((r) => [...r.slice(-2), id]);
    setTimeout(() => setRipples((r) => r.filter((x) => x !== id)), 700);
    setSmiling(true);
    setTimeout(() => setSmiling(false), 1100);
    if (body.current && !reduced()) {
      body.current.getAnimations().forEach((a) => a.cancel());
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

  // While idle, a click makes it smile; real states (working, done, failed) always win.
  const shown: GreyMood = mood === "idle" && smiling ? "done" : mood;

  return (
    <button
      className="face-orb"
      data-mood={mood}
      style={{ width: size, height: size }}
      aria-label="Jitmo"
      title="Hi, I'm Jitmo"
      onClick={play}
    >
      {ripples.map((id) => (
        <span key={id} className="face-ripple" aria-hidden="true" />
      ))}
      <span className="face-body" ref={body}>
        <GreyOrb mood={shown} size={size} />
      </span>
    </button>
  );
}
