/** The jitmo orb, recreated from the "AI orb" animation the owner picked: a soft
 *  grey sphere with light drifting across it and two white pill-shaped eyes that
 *  morph (tall, short, shifting) on a loop. Drawn with CSS only; the original
 *  animation is licensed stock and is not included. `mood` changes the pace and
 *  the eyes. The colour orb (AiOrb.tsx) and the ring (Orb.tsx) are kept. */
export type GreyMood = "idle" | "working" | "done" | "failed";

export default function GreyOrb({ mood = "idle", size = 34 }: { mood?: GreyMood; size?: number }) {
  return (
    <span className={`grey-orb ${mood}`} style={{ width: size, height: size }} aria-hidden="true">
      <span className="gl gl1" />
      <span className="gl gl2" />
      <span className="geyes">
        <span className="geye l" />
        <span className="geye r" />
      </span>
    </span>
  );
}
