/**
 * An answer that arrives word by word: each word fades and un-blurs a moment after the one
 * before it, so a result reads as spoken, not pasted. Long answers reveal in groups so the
 * whole thing still lands within about half a second. Reduced motion shows it at once (CSS).
 */
export default function Reveal({ text }: { text: string }) {
  const words = text.split(/(\s+)/);
  const count = words.filter((w) => w.trim()).length;
  const step = Math.max(6, Math.min(22, 480 / Math.max(1, count)));
  let n = 0;
  return (
    <>
      {words.map((w, i) => {
        if (!w.trim()) return w;
        const delay = Math.round(n++ * step);
        return (
          <span key={i} className="rw" style={{ animationDelay: `${delay}ms` }}>
            {w}
          </span>
        );
      })}
    </>
  );
}
