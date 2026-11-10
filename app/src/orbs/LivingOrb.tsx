import { useEffect, useRef, useState } from "react";
import "./living-orb.css";

export type OrbMood = "idle" | "working" | "done" | "failed";

/** Colour moods the orb cycles through when clicked. Jitmo's own palette, three lights each. */
export const PALETTES = [
  { name: "Aurora", a: "#7b61ff", b: "#ff4fa3", c: "#2ee6d6" },
  { name: "Ember", a: "#ff7a3d", b: "#ff3d7f", c: "#ffd36b" },
  { name: "Lagoon", a: "#2fb8ff", b: "#3dffb0", c: "#6a5cff" },
  { name: "Orchid", a: "#c86bff", b: "#ff8ad8", c: "#7aa2ff" },
  { name: "Citrus", a: "#9dff3d", b: "#ffcf3d", c: "#2ee6a8" },
] as const;

const KEY = "jitmo.orbPalette";
function savedPalette(): number {
  try {
    const n = Number(localStorage.getItem(KEY));
    return Number.isInteger(n) && n >= 0 && n < PALETTES.length ? n : 0;
  } catch {
    return 0;
  }
}

/** The palette the bar is using now (the edge glow and caret take their colours from it). */
export function paletteVars(i: number): React.CSSProperties {
  const p = PALETTES[i];
  return { "--o-a": p.a, "--o-b": p.b, "--o-c": p.c } as React.CSSProperties;
}

const reduced = () =>
  typeof window !== "undefined" && window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;

/**
 * A small living orb: three coloured lights drifting inside a dark glass bead.
 * - mood: idle drifts slowly, working swirls fast and bright, done blooms, failed dims and shakes.
 * - pulse: changes on every keystroke; the orb gives a small springy beat.
 * - click: the orb squishes like jelly, sends out a ripple and moves to the next colour mood.
 */
export default function LivingOrb({
  mood,
  pulse,
  size = 34,
  onPalette,
}: {
  mood: OrbMood;
  pulse: number;
  size?: number;
  onPalette?: (index: number) => void;
}) {
  const [palette, setPalette] = useState(savedPalette);
  const [ripples, setRipples] = useState<number[]>([]);
  const bead = useRef<HTMLSpanElement>(null);

  useEffect(() => onPalette?.(palette), [palette, onPalette]);

  // A beat per keystroke: quick swell, spring back. Never stacks into a wobble.
  useEffect(() => {
    if (!pulse || !bead.current || reduced()) return;
    bead.current.getAnimations().forEach((a) => a.id === "beat" && a.cancel());
    const a = bead.current.animate(
      [{ transform: "scale(1)" }, { transform: "scale(1.09)" }, { transform: "scale(1)" }],
      { duration: 260, easing: "cubic-bezier(0.2, 0.9, 0.3, 1)" },
    );
    a.id = "beat";
  }, [pulse]);

  const play = () => {
    const next = (palette + 1) % PALETTES.length;
    setPalette(next);
    try {
      localStorage.setItem(KEY, String(next));
    } catch {
      // private window: the mood just is not remembered
    }
    const id = Date.now();
    setRipples((r) => [...r.slice(-2), id]);
    setTimeout(() => setRipples((r) => r.filter((x) => x !== id)), 700);
    if (bead.current && !reduced()) {
      bead.current.getAnimations().forEach((a) => a.cancel());
      // Jelly: squash, overshoot, settle.
      bead.current.animate(
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

  return (
    <button
      className="lorb"
      data-mood={mood}
      style={{ ...paletteVars(palette), width: size, height: size }}
      aria-label={`Jitmo (${PALETTES[palette].name}). Click to change its colour`}
      title={`${PALETTES[palette].name} · click for a new colour`}
      onClick={play}
    >
      {ripples.map((id) => (
        <span key={id} className="lorb-ripple" aria-hidden="true" />
      ))}
      <span className="lorb-bead" ref={bead} aria-hidden="true">
        <span className="lorb-light l1" />
        <span className="lorb-light l2" />
        <span className="lorb-light l3" />
        <span className="lorb-gloss" />
      </span>
    </button>
  );
}
