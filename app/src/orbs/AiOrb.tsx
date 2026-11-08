import "./ai-orb.css";
/** An AI orb in the style of the LottieFiles "AI Orb" pack: a soft sphere of
 *  drifting blue, pink and mint with small eyes that change with its mood.
 *  Drawn with CSS only (no Lottie file, no library), so it is cheap and
 *  original. `mood` picks the eyes and the speed:
 *    idle    two tall eyes that blink
 *    working slanted, narrowed eyes and faster colour (it is thinking)
 *    done    happy arcs
 *    failed  crossed eyes */
export type Mood = "idle" | "working" | "done" | "failed";

export default function AiOrb({ mood = "idle", size = 30 }: { mood?: Mood; size?: number }) {
  return (
    <span
      className={`ai-orb ${mood}`}
      style={{ width: size, height: size }}
      data-tauri-drag-region
      aria-hidden="true"
    >
      <span className="blob b1" />
      <span className="blob b2" />
      <span className="blob b3" />
      <span className="eyes">
        <span className="eye" />
        <span className="eye" />
      </span>
    </span>
  );
}
