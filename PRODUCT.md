# Jitmo — product context

**What it is.** A local Windows assistant. A hotkey brings up one small command bar near the
bottom of the screen; the user types (later: speaks) a request ("set a 10-minute timer",
"play kesariya", "weather in Pune tomorrow", "move report.pdf to invoices") and Jitmo does it
on the PC, asks one short question when something is missing, or says plainly what it can't do.

**Who uses it, where.** One person at their own desk, mid-task, over whatever wallpaper and
windows are open, any time of day. The bar is up for seconds at a time: summon, type, read one
line, gone. Many requests are typed fast and messy: typos, slang, Hinglish.

**Surface mode.** Operate: speed, clarity and honest status come first. But the bar is the
whole product face, so it must feel premium and alive, not a utility text box.

**Pinned decisions (owner, 10 Nov 2026).**
- Dark obsidian glass world (reads on any wallpaper); a coloured edge glow while working, in
  Jitmo's own palette and motion (not a copy of Apple's Siri glow).
- A living orb at the left: colourful, clearly visible, reacts to typing and to working, done
  and failed. Clicking it is a playful reaction (squish, ripple, new colour mood it remembers),
  not a suggestions menu.
- Type: an elegant italic serif (Instrument Serif) for the placeholder and questions; a crisp
  sans (Geist) for what the user types and for answers.
- The previous looks ("classic" liquid glass, "glass2" GlassSurface) stay in the code and can
  be switched back (src/design.ts).

**Constraints.** Tauri 2 transparent always-on-top window (440–720 px wide, grows in height);
the page cannot see or blur the desktop. Keyboard first: Enter, Esc, Backspace, 1–9, Up/Down.
Respect reduced motion. Everything bundled locally (no web fonts at runtime).
