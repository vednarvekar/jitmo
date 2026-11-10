/**
 * Which look the bar uses.
 *
 * - "obsidian": dark smoked glass, the living orb, an edge glow while working, Instrument
 *   Serif + Geist type. The current look.
 * - "glass2": the GlassSurface pill (React Bits) with Geist type.
 * - "classic": the liquid-glass pill with the grey orb and Manrope.
 * All three are kept; nothing of the older looks was removed.
 *
 * To switch for good, change DEFAULT_DESIGN. To compare without rebuilding, add
 * ?design=classic (or glass2, obsidian) to the page address in the browser preview.
 */
export type Design = "obsidian" | "glass2" | "classic";

export const DEFAULT_DESIGN: Design = "obsidian";

export function currentDesign(): Design {
  try {
    const asked = new URLSearchParams(window.location.search).get("design");
    if (asked === "classic" || asked === "glass2" || asked === "obsidian") return asked;
  } catch {
    // no window (tests): the default
  }
  return DEFAULT_DESIGN;
}
