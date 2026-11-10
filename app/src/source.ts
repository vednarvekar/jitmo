/** Where an answer came from: shown as a small round button under the answer, never as text. */
export interface Source {
  /** Short name on the button's card: "Wikipedia", "MET Norway". */
  name: string;
  /** The licence the data comes under, when it names one ("CC BY-SA 4.0"). */
  licence?: string;
  /** A page about the source or the article itself. Opened only through `open_source`. */
  url?: string;
}

/** Pages for credits that carry no link of their own. */
const HOME: Record<string, string> = {
  Wikidata: "https://www.wikidata.org/",
  "MET Norway": "https://www.met.no/en",
  "the US National Weather Service": "https://www.weather.gov/",
  "ECMWF via the Finnish Meteorological Institute": "https://en.ilmatieteenlaitos.fi/open-data",
};

/**
 * Split the credit the engine appends to a looked-up answer from the answer itself:
 *   "Canberra is ... (Wikipedia, CC BY-SA 4.0: https://en.wikipedia.org/wiki/Canberra)"
 *   "The capital of Australia is Canberra. (Wikidata, CC0)"
 *   "Rain is likely ... (Data from MET Norway)"
 * Messages without a credit come back unchanged.
 */
export function splitSource(message: string): { body: string; source?: Source } {
  const m = message.match(/\s*\(((?:Wikipedia|Wikidata|Data from)[^()]*)\)\s*$/);
  if (!m || m.index === undefined) return { body: message };
  const body = message.slice(0, m.index).trim();
  if (!body) return { body: message };
  let credit = m[1].trim();
  let url: string | undefined;
  const link = credit.match(/:\s*(https:\/\/\S+)$/);
  if (link) {
    url = link[1];
    credit = credit.slice(0, link.index).trim();
  }
  credit = credit.replace(/^Data from\s+/, "");
  const [name, ...rest] = credit.split(",").map((s) => s.trim());
  const licence = rest.join(", ") || undefined;
  return {
    body,
    source: { name: name.replace(/^the /, ""), licence, url: url ?? HOME[name] },
  };
}

/** One letter for the round button: W for Wikipedia and Wikidata, the first letter otherwise. */
export function sourceMark(s: Source): string {
  if (/^Wiki/.test(s.name)) return "W";
  return s.name.charAt(0).toUpperCase();
}
