import { useEffect, useRef } from "react";

/** The jitmo orb, recreated from the "AI orb" animation the owner picked: a soft
 *  grey sphere with light drifting across it and two white pill-shaped eyes.
 *  Drawn with CSS only; the original animation is licensed stock and is not
 *  included. The colour orb (AiOrb.tsx) and the ring (Orb.tsx) are kept.
 *
 *  Motion: ONE long, continuous sequence (see `eye-l-long` in styles.css) that
 *  starts and ends in the same pose, so nothing ever snaps: the morph from the
 *  reference, a blink, a wink, surprise, a smile, a sleepy droop, a look around
 *  and a head tilt, one flowing into the next.
 *
 *  When the pointer is near, a second pair of eyes fades in over the first and
 *  follows the pointer smoothly; moving away fades back to the sequence. A click
 *  squashes the orb. `mood` (working, done, failed) changes the pace and eyes. */
export type GreyMood = "idle" | "working" | "done" | "failed";

/** The pointer counts as "near" this close (screen pixels) to the orb. */
const NEAR_PX = 380;
/** How far the eyes can move toward the pointer, as a fraction of the orb size. */
const MAX_GAZE = 0.17;
/** Glide speed toward the target each frame (0..1); smaller is smoother and slower. */
const GLIDE = 0.16;

export default function GreyOrb({ mood = "idle", size = 34 }: { mood?: GreyMood; size?: number }) {
  const ref = useRef<HTMLSpanElement>(null);
  const moodRef = useRef(mood);
  moodRef.current = mood;

  // Eyes follow the pointer, gliding rather than jumping.
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const target = { x: 0, y: 0 };
    const now = { x: 0, y: 0 };
    let raf = 0;
    const tick = () => {
      now.x += (target.x - now.x) * GLIDE;
      now.y += (target.y - now.y) * GLIDE;
      el.style.setProperty("--gx", `${now.x.toFixed(2)}px`);
      el.style.setProperty("--gy", `${now.y.toFixed(2)}px`);
      if (Math.abs(target.x - now.x) > 0.02 || Math.abs(target.y - now.y) > 0.02) {
        raf = requestAnimationFrame(tick);
      } else {
        raf = 0;
      }
    };
    const glide = () => {
      if (!raf) raf = requestAnimationFrame(tick);
    };
    const stopLooking = () => {
      el.dataset.look = "0";
      el.dataset.over = "0";
      target.x = 0;
      target.y = 0;
      glide();
    };
    const look = (e: PointerEvent) => {
      if (moodRef.current === "working" || moodRef.current === "failed") return;
      const r = el.getBoundingClientRect();
      const dx = e.clientX - (r.left + r.width / 2);
      const dy = e.clientY - (r.top + r.height / 2);
      const dist = Math.hypot(dx, dy);
      if (dist > NEAR_PX) {
        if (el.dataset.look !== "0") stopLooking();
        return;
      }
      el.dataset.look = "1";
      // Further away moves the eyes less, close by moves them fully.
      const reach = size * MAX_GAZE * Math.min(1, dist / 80);
      target.x = dist > 0 ? (dx / dist) * reach : 0;
      target.y = dist > 0 ? (dy / dist) * reach : 0;
      el.dataset.over = dist < size * 0.9 ? "1" : "0";
      glide();
    };
    window.addEventListener("pointermove", look, { passive: true });
    document.documentElement.addEventListener("pointerleave", stopLooking);
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("pointermove", look);
      document.documentElement.removeEventListener("pointerleave", stopLooking);
    };
  }, [size]);

  // A click squashes the orb for a moment.
  useEffect(() => {
    const el = ref.current;
    const parent = el?.parentElement;
    if (!el || !parent) return;
    const poke = () => {
      el.dataset.poke = "1";
      setTimeout(() => (el.dataset.poke = "0"), 420);
    };
    parent.addEventListener("pointerdown", poke);
    return () => parent.removeEventListener("pointerdown", poke);
  }, []);

  return (
    <span
      ref={ref}
      className={`grey-orb ${mood}`}
      style={{ width: size, height: size }}
      data-look="0"
      data-over="0"
      data-poke="0"
      aria-hidden="true"
    >
      <span className="gl gl1" />
      <span className="gl gl2" />
      {/* The long sequence. */}
      <span className="geyes routine">
        <span className="geye l" />
        <span className="geye r" />
      </span>
      {/* The attentive pair that follows the pointer; fades in over the sequence. */}
      <span className="geyes attend">
        <span className="geye l" />
        <span className="geye r" />
      </span>
    </span>
  );
}
