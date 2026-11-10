// Every input of the skills audit corpus (testset/audit_corpus.txt: clean, typos, partial,
// slang, Hinglish, ambiguous and adversarial phrasings of every skill), typed into the REAL bar
// and answered like a user would, against the real engine on a simulated PC.
//
// Two things are judged for each input:
//  1. the OUTCOME the corpus expects (ran / asked a question / declined / failed honestly);
//  2. the SCREEN: it settles (never stuck on "Working..."), nothing overflows the window, every
//     button sits inside the bar, no raw template text leaks, no script errors in the page.
// A line of the transcript goes to e2e-results/corpus/*.txt for reading.

import { test, expect } from "./harness";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import type { Page } from "@playwright/test";

const here = path.dirname(fileURLToPath(import.meta.url));
const corpusFile = path.resolve(here, "..", "..", "testset", "audit_corpus.txt");

interface Case {
  line: number;
  skill: string;
  kind: string;
  input: string;
  expect: string;
  note: string;
}

function expand(raw: string): string {
  const t = raw.trim();
  if (t === "<empty>") return "";
  if (t === "<spaces>") return "     ";
  const long = t.match(/^<long:(\d+):(.*)>$/);
  if (long) return Array(Number(long[1])).fill(long[2]).join(" ");
  return t.replace(/<tab>/g, " ").replace(/<nl>/g, " ");
}

function load(): Case[] {
  const out: Case[] = [];
  fs.readFileSync(corpusFile, "utf8")
    .split(/\r?\n/)
    .forEach((raw, i) => {
      const l = raw.trim();
      if (!l || l.startsWith("#")) return;
      const parts = l.split("|").map((p) => p.trim());
      if (parts.length < 4) return;
      // Lines that need a special simulated world (env=...) are judged by jitmo-audit,
      // which can set that world up; the UI engine runs the default world only.
      if (/(^|;)\s*env=/.test(parts[3])) return;
      out.push({
        line: i + 1,
        skill: parts[0],
        kind: parts[1],
        input: expand(parts[2]),
        expect: parts[3],
        note: parts.slice(4).join("|"),
      });
    });
  return out;
}

type Outcome = "done" | "failed" | "declined" | "asked" | "confirm" | "stuck" | "nothing";

interface Seen {
  outcome: Outcome;
  message: string;
  buttons: string[];
  undo: boolean;
}

/** Wait until the bar shows an answer or a question; say what it shows. */
async function settle(page: Page): Promise<Seen> {
  const sel = ".card, .status:not(.working)";
  try {
    await page.locator(sel).first().waitFor({ timeout: 12_000 });
  } catch {
    const working = await page.locator(".status.working").count();
    return { outcome: working ? "stuck" : "nothing", message: "", buttons: [], undo: false };
  }
  // Give a result a beat to replace a transient state.
  await page.waitForTimeout(60);
  const card = await page.locator(".card").count();
  const undo = (await page.getByRole("button", { name: /Undo/ }).count()) > 0;
  if (card) {
    const buttons = (await page.locator(".card button").allInnerTexts()).map((t) => t.replace(/\s+/g, " ").trim());
    const message = (await page.locator(".card .line").first().innerText()).trim();
    const confirm = buttons.some((b) => /^Confirm/.test(b));
    return { outcome: confirm ? "confirm" : "asked", message, buttons, undo };
  }
  const failed = (await page.locator(".status.failed").count()) > 0;
  const message = (await page.locator(".status .message").first().innerText().catch(() => "")).trim();
  const declined = failed && /can't do that yet|can't plan|can’t do that/i.test(message);
  return { outcome: declined ? "declined" : failed ? "failed" : "done", message, buttons: [], undo };
}

function modifier(expect: string, key: string): string | undefined {
  const t = expect.split(";").map((s) => s.trim()).find((s) => s.startsWith(key + "="));
  return t?.slice(key.length + 1);
}

const cases = load().filter((c) => c.input !== "" || true);
const reportDir = path.resolve(here, "..", "e2e-results", "corpus");
fs.mkdirSync(reportDir, { recursive: true });

for (const c of cases) {
  test(`${c.skill} · ${c.kind} · L${c.line} · ${c.input.slice(0, 60)}`, async ({ fastBar: bar }, info) => {
    const page = bar.page;
    const problems: string[] = [];
    page.on("pageerror", (e) => problems.push(`page error: ${e.message}`));
    page.on("console", (m) => {
      if (m.type() === "error" && !/favicon|WebSocket|\[vite\]/i.test(m.text())) problems.push(`console error: ${m.text()}`);
    });

    await bar.open();
    const parts = c.input.split(" && ");
    let seen: Seen = { outcome: "nothing", message: "", buttons: [], undo: false };
    const pick = modifier(c.expect, "pick");
    const say = modifier(c.expect, "say");
    const wantsRun = c.expect.split(";")[0].split("/").some((a) => a.startsWith("run"));

    for (const part of parts) {
      if (part.trim() === "") {
        await page.keyboard.press("Enter"); // an empty request is ignored
        continue;
      }
      if (part.length > 120) {
        await page.locator("input.input").fill(part);
      } else {
        await page.keyboard.type(part);
      }
      await page.keyboard.press("Enter");
      seen = await settle(page);
      // Answer what the corpus says a user would answer (at most four rounds).
      for (let round = 0; round < 4; round++) {
        if (seen.outcome === "confirm" && wantsRun) {
          await page.getByRole("button", { name: /^Confirm/ }).click();
        } else if (seen.outcome === "asked" && pick && seen.buttons.some((b) => b.toLowerCase().includes(pick.toLowerCase()))) {
          await page
            .locator(".card button")
            .filter({ hasText: new RegExp(pick.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "i") })
            .first()
            .click();
        } else if (seen.outcome === "asked" && say && seen.buttons.some((b) => /^Answer/.test(b))) {
          await page.keyboard.type(say);
          await page.keyboard.press("Enter");
        } else {
          break;
        }
        seen = await settle(page);
      }
    }

    // ---- the screen ------------------------------------------------------------------
    if (seen.outcome === "stuck") problems.push('the bar is stuck on "Working..."');
    const layout = await page.evaluate(() => {
      const bar = document.querySelector("[data-phase]")?.getBoundingClientRect();
      const bad: string[] = [];
      if (document.documentElement.scrollWidth > window.innerWidth + 1) bad.push("page scrolls sideways");
      if (bar) {
        document.querySelectorAll("[data-phase] button, [data-phase] input").forEach((el) => {
          const r = (el as HTMLElement).getBoundingClientRect();
          if (r.width === 0 || r.height === 0) return;
          if (r.left < bar.left - 2 || r.right > bar.right + 2 || r.top < bar.top - 2 || r.bottom > bar.bottom + 2) {
            bad.push(`"${(el as HTMLElement).innerText.trim().slice(0, 20) || el.tagName}" is outside the bar`);
          }
        });
        if (bar.bottom > window.innerHeight) bad.push("the bar runs off the bottom of the window");
      }
      return bad;
    });
    problems.push(...layout);
    const shown = seen.message;
    if (/\{[a-z_.]+\}|undefined|NaN|\[object|\bnull\b/.test(shown)) problems.push(`raw text leaks into the answer: "${shown}"`);
    if (seen.outcome !== "nothing" && seen.outcome !== "stuck" && shown.length === 0) problems.push("the answer is empty");
    if (shown.length > 600) problems.push("the answer is longer than a bar can show");

    // ---- the outcome the corpus expects -----------------------------------------------
    const primary = c.expect.split(";")[0].trim();
    const alts = primary.split("/").map((a) => a.split(":")[0]);
    const matches: Record<string, Outcome[]> = {
      run: ["done"],
      ask: ["asked"],
      decline: ["declined"],
      fail: ["failed"],
    };
    const known = alts.every((a) => a in matches);
    if (c.input.trim() !== "" && known) {
      const ok = alts.some((a) => matches[a].includes(seen.outcome) || (a === "ask" && seen.outcome === "confirm"));
      if (!ok) problems.push(`expected ${primary}, the bar ended with ${seen.outcome}: "${shown}"`);
    }

    const row = `${problems.length ? "FAIL" : "ok  "} L${c.line} ${c.skill} | ${c.kind} | ${c.input.slice(0, 70)} => ${seen.outcome}: ${shown.slice(0, 110)}${seen.buttons.length ? "  [" + seen.buttons.join(" | ") + "]" : ""}${problems.length ? "\n      " + problems.join("\n      ") : ""}\n`;
    fs.appendFileSync(path.join(reportDir, `worker-${info.workerIndex}.txt`), row);
    if (problems.length) await bar.shot(`corpus-L${c.line}`);
    expect(problems, `${c.input}\n${problems.join("\n")}`).toEqual([]);
  });
}
