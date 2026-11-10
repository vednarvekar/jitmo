import { describe, expect, it } from "vitest";
import { WORKING_WORDS, pickWord } from "./WorkingWord";

describe("pickWord", () => {
  it("never repeats the word it is replacing", () => {
    for (const w of WORKING_WORDS) {
      for (let i = 0; i < 20; i++) expect(pickWord(w)).not.toBe(w);
    }
  });
  it("picks from the list", () => {
    expect(WORKING_WORDS).toContain(pickWord(undefined, () => 0));
    expect(WORKING_WORDS).toContain(pickWord(undefined, () => 0.999));
  });
});
