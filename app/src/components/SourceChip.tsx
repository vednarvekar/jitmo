import * as bridge from "../bridge";
import { sourceMark, type Source } from "../source";

/** The small round source button that sits after an answer (in place of the credit text). */
export function SourceChip({
  source,
  open,
  onToggle,
}: {
  source: Source;
  open: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      className="source-chip"
      aria-label={`Source: ${source.name}`}
      aria-expanded={open}
      title={`Source: ${source.name}`}
      onClick={(e) => {
        // The status line resets the bar on click; this button must not.
        e.stopPropagation();
        onToggle();
      }}
    >
      {sourceMark(source)}
    </button>
  );
}

/** The card the source button opens: the source, its licence, and a link to it. */
export function SourceCard({ source }: { source: Source }) {
  return (
    <div className="source-card" role="note">
      <span className="source-mark" aria-hidden="true">
        {sourceMark(source)}
      </span>
      <span className="source-name">{source.name}</span>
      {source.licence && <span className="source-licence">{source.licence}</span>}
      {source.url && (
        <button
          className="source-open"
          onClick={(e) => {
            e.stopPropagation();
            bridge.openSource(source.url!);
          }}
        >
          Open ↗
        </button>
      )}
    </div>
  );
}
