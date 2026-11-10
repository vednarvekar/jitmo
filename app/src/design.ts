/**
 * Which look the bar uses.
 *
 * - "glass2": the GlassSurface pill (React Bits) with Geist type. Being tried now.
 * - "classic": the liquid-glass pill with Manrope, as it was before. All of its code and
 *   styles are kept unchanged.
 *
 * To go back for good, set DEFAULT_DESIGN to "classic". To compare without rebuilding,
 * add ?design=classic (or ?design=glass2) to the page address in the browser preview.
 */
export type Design = "glass2" | "classic";

export const DEFAULT_DESIGN: Design = "glass2";

export function currentDesign(): Design {
  try {
    const asked = new URLSearchParams(window.location.search).get("design");
    if (asked === "classic" || asked === "glass2") return asked;
  } catch {
    // no window (tests): the default
  }
  return DEFAULT_DESIGN;
}
