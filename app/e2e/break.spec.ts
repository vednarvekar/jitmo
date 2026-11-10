// Worst-case data for the bar, fed through the same boundary the real data uses (the engine's
// events). Every value is something the engine can really produce: a long file name from the
// user's folders, a contact with a long double name and a number, a fact answer plus its
// credit link, an error that quotes a path, text in other scripts, a number beyond 32 bits.
//
// For each: the screen must not overflow, every button must stay inside the bar, long words
// must wrap instead of pushing things out, and nothing may be cut off without a way to read it.

import { test, expect } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const shots = path.resolve(here, "..", "e2e-results", "break");

type Ev = Record<string, unknown>;

const LONG_FILE = "Tax return 2025 – final (copy 3) signed by accountant and reviewed by Bartholomew.pdf";
const CONTACT = "Bartholomew Fitzgerald-Montgomery III (+91 98765 43210)";
const URL_CREDIT =
  "(Wikipedia, CC BY-SA 4.0: https://en.wikipedia.org/wiki/List_of_tallest_mountains_in_the_Solar_System_by_elevation_above_datum)";
const FACT =
  "The Eiffel Tower is a wrought-iron lattice tower on the Champ de Mars in Paris, France. It is named after the engineer Gustave Eiffel, whose company designed and built the tower from 1887 to 1889 as the centerpiece of the 1889 World's Fair. " +
  URL_CREDIT;

const choices = (labels: string[]) => labels.map((label, i) => ({ id: String(i), label }));

const CASES: Array<{ name: string; events: Ev[]; mustShow?: RegExp }> = [
  { name: "done: a fact answer with its credit link (about 400 characters, one unbreakable URL)", events: [{ type: "done", id: 1, message: FACT, undoable: false }], mustShow: /Eiffel/ },
  { name: "done: a long unbreakable word", events: [{ type: "done", id: 1, message: "Saved to C:\\Users\\Aleksandra.Wisniewska-Kowalczyk\\OneDrive - Northwind Industries Holdings\\Documents\\Invoices\\2026\\Q4\\invoice_final_v12_signed.pdf", undoable: true }] },
  { name: "done: the help text", events: [{ type: "done", id: 1, message: "I can open apps, play and pause music, set timers and reminders, do quick math, find and move your files, change volume, brightness and Wi-Fi, take a screenshot, lock the PC, check the weather and answer quick facts. Say 'what timers do I have' or 'stop the timer' to see or stop them. Try: 'open Spotify', '10-minute timer', '18% of 2,450'.", undoable: false }], mustShow: /quick facts/ },
  { name: "done: Undo button next to a long message", events: [{ type: "done", id: 1, message: "Moved 12 files to Invoices: " + LONG_FILE, undoable: true }] },
  { name: "failed: an error quoting a long path", events: [{ type: "failed", id: 1, message: `I couldn't move ${LONG_FILE} to C:\\Users\\Aleksandra\\Documents\\Invoices because it is open in another program.` }] },
  { name: "failed: empty message", events: [{ type: "failed", id: 1, message: "" }] },
  { name: "confirm: a long contact and a long message", events: [{ type: "confirm_needed", id: 1, text: `Send ${LONG_FILE} to ${CONTACT} on WhatsApp?` }] },
  { name: "choice: 12 files with long names", events: [{ type: "choice_needed", id: 1, question: "Which one?", choices: choices(Array.from({ length: 12 }, (_, i) => `${LONG_FILE.slice(0, 40 + i * 3)} ${i + 1}.pdf`)) }] },
  { name: "choice: contacts that tie", events: [{ type: "choice_needed", id: 1, question: "Which Ved?", choices: choices([CONTACT, "Ved Narvekar (+91 98765 43210)", "Ved", "J"]) }] },
  { name: "choice: one option with an unbreakable label", events: [{ type: "choice_needed", id: 1, question: "Which one?", choices: choices(["C:\\Users\\Aleksandra.Wisniewska-Kowalczyk\\OneDrive-NorthwindIndustriesHoldings\\Documents\\invoice_final_v12_signed.pdf", "ok"]) }] },
  { name: "ask: a long question", events: [{ type: "text_needed", id: 1, question: "What should I remind you about, and should it repeat every week until the end of the year or only once?", hint: "For example: call mom on her birthday and bring the cake from the shop near the station" }] },
  { name: "done: Arabic (right to left)", events: [{ type: "done", id: 1, message: "تم ضبط المؤقت لمدة ١٠ دقائق. سأنبهك عند انتهاء الوقت.", undoable: true }] },
  { name: "done: Hindi with matras", events: [{ type: "done", id: 1, message: "10 मिनट का टाइमर सेट हो गया। समय पूरा होने पर मैं आपको बताऊँगा।", undoable: false }] },
  { name: "done: Thai and Japanese (tall scripts, no spaces)", events: [{ type: "done", id: 1, message: "ตั้งเวลาไว้ 10 นาทีแล้ว。タイマーを10分にセットしました。時間になったらお知らせします。", undoable: false }] },
  { name: "done: emoji", events: [{ type: "done", id: 1, message: "🎵 Playing Kesariya on Spotify 🎶 ✨ 👨‍👩‍👧‍👦", undoable: false }] },
  { name: "done: markup characters must show as text", events: [{ type: "done", id: 1, message: "<b>bold</b> &amp; **stars** {name} <script>alert(1)</script>", undoable: false }], mustShow: /<b>bold<\/b>/ },
  { name: "done: a number beyond 32 bits", events: [{ type: "done", id: 1, message: "99999999999999 * 99999999999999 = 9.9999999999998e27", undoable: false }] },
];

for (const c of CASES) {
  test(c.name, async ({ page }, info) => {
    const problems: string[] = [];
    page.on("pageerror", (e) => problems.push(`page error: ${e.message}`));
    await page.goto("/");
    await page.waitForFunction(() => Boolean((window as any).__jitmo));
    await page.evaluate(() => (window as any).__jitmo.emit("bar-shown"));
    await page.waitForTimeout(700);
    const emit = (e: Ev) => page.evaluate((x) => (window as any).__jitmo.emit("core-event", x), e);
    await emit({ type: "started", id: 1 });
    for (const ev of c.events) await emit({ ...ev, id: 1 });
    await page.waitForTimeout(400);

    const report = await page.evaluate(() => {
      const bar = document.querySelector("[data-phase]")!.getBoundingClientRect();
      const bad: string[] = [];
      if (document.documentElement.scrollWidth > window.innerWidth + 1) bad.push("page scrolls sideways");
      document.querySelectorAll<HTMLElement>("[data-phase] button, [data-phase] .message, [data-phase] .line, [data-phase] input").forEach((el) => {
        const r = el.getBoundingClientRect();
        if (!r.width || !r.height) return;
        if (r.right > bar.right + 1 || r.left < bar.left - 1) bad.push(`"${(el.innerText || el.tagName).trim().slice(0, 28)}" sticks out of the bar sideways`);
        if (el.scrollWidth > el.clientWidth + 1 && getComputedStyle(el).overflow !== "visible" && el.tagName !== "INPUT") {
          bad.push(`"${(el.innerText || "").trim().slice(0, 28)}" is wider than its box and clipped`);
        }
      });
      // Text hidden by the two-line clamp with no way to read the rest.
      document.querySelectorAll<HTMLElement>(".status .message").forEach((el) => {
        if (el.scrollHeight > el.clientHeight + 2) bad.push(`message is cut off (${el.scrollHeight}px of text in a ${el.clientHeight}px box)`);
      });
      return { bad, barHeight: Math.round(bar.height), text: document.querySelector<HTMLElement>(".status .message, .card .line")?.innerText ?? "" };
    });
    problems.push(...report.bad);
    if (c.mustShow && !c.mustShow.test(report.text)) problems.push(`expected the text to show ${c.mustShow}, got "${report.text.slice(0, 80)}"`);
    if (report.barHeight > 640) problems.push(`the bar is ${report.barHeight}px tall (the window holds 700)`);

    fs.mkdirSync(shots, { recursive: true });
    await page.screenshot({ path: path.join(shots, info.title.replace(/[^a-z0-9]+/gi, "_").slice(0, 70) + ".png") });
    expect(problems, problems.join("\n")).toEqual([]);
  });
}
