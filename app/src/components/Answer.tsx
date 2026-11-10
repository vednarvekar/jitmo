import Reveal from "./Reveal";

/**
 * A finished answer with some structure (obsidian look):
 * - an example in quotes ('open Spotify', “10-minute timer”) becomes a small button that runs it;
 * - the help reply ("I can open apps, …") becomes a heading, a set of skills you can tap to
 *   start (each opens with its own question), and the tappable examples.
 * Everything else is the plain sentence, arriving word by word.
 */

/** Quoted examples: a quote mark after a space or the start, a short phrase, a closing quote. */
const QUOTED = /(^|[\s:(])['‘“"]([^'’”"]{2,48})['’”"](?=[\s.,;:!?)]|$)/g;

type Part = { text: string } | { example: string };

export function splitExamples(text: string): Part[] {
  const parts: Part[] = [];
  let at = 0;
  for (const m of text.matchAll(QUOTED)) {
    const start = (m.index ?? 0) + m[1].length;
    if (start > at) parts.push({ text: text.slice(at, start) });
    parts.push({ example: m[2] });
    at = (m.index ?? 0) + m[0].length;
  }
  if (at < text.length) parts.push({ text: text.slice(at) });
  return parts;
}

/** The skills on the help card. `run` is a bare command that leads into the skill's question;
 *  skills with a direct, unasked effect (locking the PC) are shown but not tappable. */
const SKILLS: Array<{ label: string; run?: string }> = [
  { label: "Open apps", run: "open" },
  { label: "Music", run: "music" },
  { label: "Timers", run: "timer" },
  { label: "Reminders", run: "remind me" },
  { label: "Quick math", run: "calculate" },
  { label: "Find files", run: "find" },
  { label: "Move files", run: "move" },
  { label: "Volume", run: "volume" },
  { label: "Brightness", run: "brightness" },
  { label: "Wi-Fi", run: "wifi" },
  { label: "Weather", run: "weather" },
  { label: "Quick facts", run: "what is" },
  { label: "Screenshot" },
  { label: "Lock the PC" },
];

export const isHelp = (text: string) => text.startsWith("I can open apps");

function Examples({ text, onRun, reveal }: { text: string; onRun: (s: string) => void; reveal: boolean }) {
  return (
    <>
      {splitExamples(text).map((p, i) =>
        "example" in p ? (
          <button
            key={i}
            className="try"
            title={`Run “${p.example}”`}
            onClick={(e) => {
              e.stopPropagation();
              onRun(p.example);
            }}
          >
            {p.example}
          </button>
        ) : /^\s*,\s*$/.test(p.text) ? (
          <span key={i}> </span>
        ) : reveal ? (
          <Reveal key={i} text={p.text} />
        ) : (
          <span key={i}>{p.text}</span>
        ),
      )}
    </>
  );
}

export default function Answer({ text, onRun }: { text: string; onRun: (s: string) => void }) {
  if (!isHelp(text)) return <Examples text={text} onRun={onRun} reveal />;
  // The help reply: its first sentence is the skills list, shown as tappable skills instead.
  const rest = text.slice(text.indexOf(".") + 1).trim();
  return (
    <span className="help">
      <span className="help-lead">Here's what I can do</span>
      <span className="help-skills">
        {SKILLS.map((s, i) =>
          s.run ? (
            <button
              key={s.label}
              className="skill"
              style={{ ["--i" as string]: i }}
              title={`Start: ${s.run}`}
              onClick={(e) => {
                e.stopPropagation();
                onRun(s.run!);
              }}
            >
              {s.label}
            </button>
          ) : (
            <span key={s.label} className="skill static" style={{ ["--i" as string]: i }}>
              {s.label}
            </span>
          ),
        )}
      </span>
      <span className="help-more">
        <Examples text={rest} onRun={onRun} reveal={false} />
      </span>
    </span>
  );
}
