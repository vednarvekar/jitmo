import { useEffect, useRef } from "react";

/** The jitmo orb, recreated from the "AI orb" animation the owner picked: a soft
 *  grey sphere with light drifting across it and two white pill-shaped eyes.
 *  Drawn with CSS only; the original animation is licensed stock and is not
 *  included. The colour orb (AiOrb.tsx) and the ring (Orb.tsx) are kept.
 *
 *  Behaviours (idle only, picked at random every few seconds):
 *    loop      the eyes morph between a tall/short pair and a matching pair (default)
 *    blink, wink, surprised, smile, sleepy, tilt, glance, look-around
 *  When the pointer is near, the eyes stop and follow it. A click squashes the
 *  orb for a moment. `mood` (working, done, failed) takes over from idle. */
export type GreyMood = "idle" | "working" | "done" | "failed";

/** Behaviours the orb picks from, with how often (weights). */
const BEHAVIOURS: [string, number, number][] = [
  // name, weight, duration in ms
  ["blink", 4, 450],
  ["wink", 2, 700],
  ["surprised", 1, 900],
  ["smile", 2, 1500],
  ["sleepy", 1, 2600],
  ["tilt", 2, 1600],
  ["glance", 2, 1500],
  ["lookaround", 2, 2600],
  ["loop", 3, 3400],
];

/** The pointer counts as "near" this close (in screen pixels) to the orb. */
const NEAR_PX = 240;
/** How far the eyes can move toward the pointer, as a fraction of the orb size. */
const MAX_GAZE = 0.17;

function pick(): [string, number] {
  const total = BEHAVIOURS.reduce((s, b) => s + b[1], 0);
  let r = Math.random() * total;
  for (const [name, weight, ms] of BEHAVIOURS) {
    r -= weight;
    if (r <= 0) return [name, ms];
  }
  return ["loop", 3400];
}

export default function GreyOrb({ mood = "idle", size = 34 }: { mood?: GreyMood; size?: number }) {
  const ref = useRef<HTMLSpanElement>(null);
  const moodRef = useRef(mood);
  moodRef.current = mood;

  // Random behaviours while idle.
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    let timer: ReturnType<typeof setTimeout>;
    const next = () => {
      const [name, ms] = pick();
      if (moodRef.current === "idle" && el.dataset.look !== "1") el.dataset.b = name;
      timer = setTimeout(() => {
        el.dataset.b = "loop";
        timer = setTimeout(next, 1200 + Math.random() * 3200);
      }, ms);
    };
    el.dataset.b = "loop";
    timer = setTimeout(next, 1500);
    return () => clearTimeout(timer);
  }, []);

  // Eyes follow the pointer when it is near.
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const look = (e: PointerEvent) => {
      if (moodRef.current === "working" || moodRef.current === "failed") return;
      const r = el.getBoundingClientRect();
      const cx = r.left + r.width / 2;
      const cy = r.top + r.height / 2;
      const dx = e.clientX - cx;
      const dy = e.clientY - cy;
      const dist = Math.hypot(dx, dy);
      if (dist > NEAR_PX) {
        el.dataset.look = "0";
        el.style.setProperty("--gx", "0px");
        el.style.setProperty("--gy", "0px");
        return;
      }
      el.dataset.look = "1";
      // Move toward the pointer; further away moves less, close by moves fully.
      const pull = Math.min(1, dist / 70);
      const reach = size * MAX_GAZE * pull;
      const ux = dist > 0 ? dx / dist : 0;
      const uy = dist > 0 ? dy / dist : 0;
      el.style.setProperty("--gx", `${(ux * reach).toFixed(2)}px`);
      el.style.setProperty("--gy", `${(uy * reach).toFixed(2)}px`);
      // Right on top of the orb: a little smile.
      el.dataset.over = dist < size * 0.9 ? "1" : "0";
    };
    const leave = () => {
      el.dataset.look = "0";
      el.dataset.over = "0";
      el.style.setProperty("--gx", "0px");
      el.style.setProperty("--gy", "0px");
    };
    window.addEventListener("pointermove", look, { passive: true });
    document.documentElement.addEventListener("pointerleave", leave);
    return () => {
      window.removeEventListener("pointermove", look);
      document.documentElement.removeEventListener("pointerleave", leave);
    };
  }, [size]);

  // A click squashes the orb for a moment.
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const poke = () => {
      el.dataset.poke = "1";
      setTimeout(() => (el.dataset.poke = "0"), 380);
    };
    el.parentElement?.addEventListener("pointerdown", poke);
    const parent = el.parentElement;
    return () => parent?.removeEventListener("pointerdown", poke);
  }, []);

  return (
    <span
      ref={ref}
      className={`grey-orb ${mood}`}
      style={{ width: size, height: size }}
      data-b="loop"
      data-look="0"
      aria-hidden="true"
    >
      <span className="gl gl1" />
      <span className="gl gl2" />
      <span className="geyes">
        <span className="geye l" />
        <span className="geye r" />
      </span>
    </span>
  );
}
