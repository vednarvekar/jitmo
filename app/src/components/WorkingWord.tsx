import { useEffect, useState } from "react";

// What the bar says while it works: a word that changes, so a slow answer feels alive
// instead of stuck. Playful on purpose; the answer replaces it as soon as it is ready.
export const WORKING_WORDS = [
  "Pondering",
  "Cogitating",
  "Ruminating",
  "Mulling",
  "Noodling",
  "Percolating",
  "Brewing",
  "Simmering",
  "Marinating",
  "Caramelizing",
  "Whisking",
  "Shimmying",
  "Moseying",
  "Frolicking",
  "Scampering",
  "Booping",
  "Flibbertigibbeting",
  "Honking",
  "Conjuring",
  "Untangling",
  "Tinkering",
  "Hatching",
  "Whirring",
  "Spelunking",
];

/** A word that is not `not`, picked at random. */
export function pickWord(not?: string, random: () => number = Math.random): string {
  const pool = WORKING_WORDS.filter((w) => w !== not);
  return pool[Math.floor(random() * pool.length)];
}

const EVERY_MS = 1700;

export default function WorkingWord() {
  const [word, setWord] = useState(() => pickWord());
  useEffect(() => {
    const t = setInterval(() => setWord((w) => pickWord(w)), EVERY_MS);
    return () => clearInterval(t);
  }, []);
  return (
    <>
      <span key={word} className="ww">
        {word}
      </span>
      <span className="dots" aria-hidden="true">
        <i>.</i>
        <i>.</i>
        <i>.</i>
      </span>
    </>
  );
}
