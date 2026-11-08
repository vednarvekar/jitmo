/** The jitmo orb: a glossy gradient sphere. It is only CSS (see `.orb` in
 *  styles.css), not a letter or anyone's logo, and it pulses while a request runs. */
export default function Orb({ working = false }: { working?: boolean }) {
  return <span className={`orb${working ? " working" : ""}`} data-tauri-drag-region aria-hidden="true" />;
}
