import { useEffect, useState } from "react";

/** What the empty bar suggests, one at a time, in the serif placeholder voice. */
const HINTS = [
  "Ask Jitmo anything…",
  "set a 10-minute timer",
  "will it rain tomorrow?",
  "play something calm",
  "remind me at 6 to call mom",
  "what's 18% of 2,450?",
  "turn on do not disturb",
];

/**
 * The placeholder for the empty input: "Ask Jitmo anything…", then real examples that
 * cross-fade every few seconds. Sits over the input and never takes a click.
 */
export default function Hints({ visible, fixed }: { visible: boolean; fixed?: string }) {
  const [i, setI] = useState(0);
  useEffect(() => {
    if (!visible || fixed) return;
    setI(0);
    const t = setInterval(() => setI((x) => (x + 1) % HINTS.length), 3200);
    return () => clearInterval(t);
  }, [visible, fixed]);
  if (!visible) return null;
  const line = fixed ?? HINTS[i];
  return (
    <span className="hint" aria-hidden="true">
      <span key={line} className="hint-line">
        {line}
      </span>
    </span>
  );
}
