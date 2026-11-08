/** One-tap ideas shown under an empty bar (opened by clicking the orb). */
export default function Suggestions({ items, onPick }: { items: string[]; onPick: (text: string) => void }) {
  return (
    <div className="chips" data-tauri-drag-region>
      {items.map((s) => (
        <button key={s} className="chip" onClick={() => onPick(s)}>
          {s}
        </button>
      ))}
    </div>
  );
}
