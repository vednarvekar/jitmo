import * as bridge from "../bridge";
import type { Choice } from "../state";

/** The confirm card: shown before a send or other action that needs a yes. */
export function ConfirmCard({ id, text }: { id: number; text: string }) {
  return (
    <div className="card">
      <div className="line">{text}</div>
      <div className="buttons">
        <button className="primary" onClick={() => bridge.confirm(id, true)}>
          Confirm <kbd>Enter</kbd>
        </button>
        <button onClick={() => bridge.confirm(id, false)}>
          Cancel <kbd>Esc</kbd>
        </button>
      </div>
    </div>
  );
}

/** One short question with numbered choices (keys 1 to 9 also pick). */
export function ChoiceCard({ id, question, choices }: { id: number; question: string; choices: Choice[] }) {
  return (
    <div className="card">
      <div className="line">{question}</div>
      <div className="buttons column">
        {choices.map((c, i) => (
          <button key={c.id} onClick={() => bridge.choose(id, c.id)}>
            <kbd>{i + 1}</kbd> {c.label}
          </button>
        ))}
      </div>
    </div>
  );
}

/** One short free-text question. The answer is typed in the bar's input line above it
 *  (Enter sends, Esc cancels); the buttons do the same for the mouse. */
export function AskCard({ id, question, answer }: { id: number; question: string; answer: string }) {
  return (
    <div className="card">
      <div className="line">{question}</div>
      <div className="buttons">
        <button className="primary" onClick={() => bridge.answer(id, answer)}>
          Answer <kbd>Enter</kbd>
        </button>
        <button onClick={() => bridge.cancel(id)}>
          Cancel <kbd>Esc</kbd>
        </button>
      </div>
    </div>
  );
}
