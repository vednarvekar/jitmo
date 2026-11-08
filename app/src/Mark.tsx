/** The jitmo mark: a lowercase "j" on a rounded tile. Drawn here from plain
 *  shapes, so it does not copy anyone else's logo or depend on a font. */
export default function Mark({ size = 24 }: { size?: number }) {
  return (
    <svg
      className="mark-svg"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      aria-hidden="true"
      data-tauri-drag-region
    >
      <rect width="24" height="24" rx="7" fill="var(--accent)" data-tauri-drag-region />
      <circle cx="14.5" cy="6.4" r="1.7" fill="#fff" data-tauri-drag-region />
      <path
        d="M14.5 10.2v6.2a3.3 3.3 0 0 1-3.3 3.3H9.4"
        fill="none"
        stroke="#fff"
        strokeWidth="2.2"
        strokeLinecap="round"
        data-tauri-drag-region
      />
    </svg>
  );
}
