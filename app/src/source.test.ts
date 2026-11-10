import { describe, expect, it } from "vitest";
import { splitSource } from "./source";

describe("splitSource", () => {
  it("takes the Wikipedia credit and its link off the answer", () => {
    const r = splitSource(
      "Canberra is the capital city of Australia. (Wikipedia, CC BY-SA 4.0: https://en.wikipedia.org/wiki/Canberra)",
    );
    expect(r.body).toBe("Canberra is the capital city of Australia.");
    expect(r.source).toEqual({
      name: "Wikipedia",
      licence: "CC BY-SA 4.0",
      url: "https://en.wikipedia.org/wiki/Canberra",
    });
  });

  it("handles Wikidata and the weather services", () => {
    expect(splitSource("The capital of Australia is Canberra. (Wikidata, CC0)").source).toEqual({
      name: "Wikidata",
      licence: "CC0",
      url: "https://www.wikidata.org/",
    });
    const met = splitSource("Pune today: high 30°C, low 20°C. (Data from MET Norway)");
    expect(met.body).toBe("Pune today: high 30°C, low 20°C.");
    expect(met.source?.name).toBe("MET Norway");
    expect(splitSource("Rain. (Data from the US National Weather Service)").source?.name).toBe(
      "US National Weather Service",
    );
    const fmi = splitSource(
      "Rain. (Data from ECMWF via the Finnish Meteorological Institute, CC BY 4.0)",
    ).source;
    expect(fmi?.licence).toBe("CC BY 4.0");
    expect(fmi?.url).toContain("ilmatieteenlaitos.fi");
  });

  it("leaves other messages alone", () => {
    for (const m of ["Opened Spotify.", "Done (2 files).", "(Wikipedia, CC0)", "Timer set (10 minutes)."]) {
      expect(splitSource(m)).toEqual({ body: m });
    }
  });
});
