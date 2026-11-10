// The end-to-end harness: one page + one engine per test, both private to the test.
//
//   headless Edge ── the real bar page (React) ── window.__jitmoInvoke / __jitmo.emit
//                                │
//                          this file (Node)
//                                │  TCP, one JSON line per message: the desktop app's pipe protocol
//                       jitmo-e2e.exe ── the real engine on a SIMULATED PC
//
// The engine never touches the machine (no app launched, no setting changed, nothing sent),
// and the browser is headless, so running these while you work cannot disturb you.

import { test as base, expect, type Page } from "@playwright/test";
import { spawn, type ChildProcess } from "node:child_process";
import net from "node:net";
import path from "node:path";
import fs from "node:fs";
import { fileURLToPath } from "node:url";

export { expect };

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..", "..");
const exe = path.join(root, "target", "release", "jitmo-e2e.exe");
const worker = path.join(root, "target", "release", "jitmo-model-worker.exe");

export type CoreEvent = { type: string; id: number; [k: string]: unknown };

/** The engine process and its two local ports. */
class Engine {
  private proc!: ChildProcess;
  private socket!: net.Socket;
  private controlPort = 0;
  events: CoreEvent[] = [];
  private listeners: Array<(e: CoreEvent) => void> = [];

  static async start(flags: string[] = []): Promise<Engine> {
    if (!fs.existsSync(exe)) {
      throw new Error(`${exe} is missing: run  cargo build --release -p jitmo-testset-runner --bin jitmo-e2e`);
    }
    const args = ["--skills", path.join(root, "skills")];
    // The real router when its files are here; the example-phrase router otherwise.
    if (fs.existsSync(worker) && fs.existsSync(path.join(root, "models", "router", "model_int8.onnx"))) {
      args.push("--worker", worker, "--models", path.join(root, "models", "router"));
      if (fs.existsSync(path.join(root, "models", "ner"))) {
        args.push("--extractor", path.join(root, "models", "ner"));
      }
    }
    if (flags.length) args.push("--flags", flags.join(","));
    const e = new Engine();
    e.proc = spawn(exe, args, { cwd: root, stdio: ["ignore", "pipe", "inherit"] });
    const ports = await new Promise<[number, number]>((resolve, reject) => {
      let buf = "";
      const timer = setTimeout(() => reject(new Error("the engine did not start")), 20_000);
      e.proc.stdout!.on("data", (d) => {
        buf += d;
        const m = buf.match(/LISTENING (\d+) (\d+)/);
        if (m) {
          clearTimeout(timer);
          resolve([Number(m[1]), Number(m[2])]);
        }
      });
      e.proc.on("exit", () => reject(new Error("the engine exited")));
    });
    e.controlPort = ports[1];
    e.socket = net.connect(ports[0], "127.0.0.1");
    let acc = "";
    e.socket.on("data", (d) => {
      acc += d;
      let i;
      while ((i = acc.indexOf("\n")) >= 0) {
        const line = acc.slice(0, i);
        acc = acc.slice(i + 1);
        if (!line.trim()) continue;
        const ev = JSON.parse(line) as CoreEvent;
        e.events.push(ev);
        e.listeners.forEach((l) => l(ev));
      }
    });
    await new Promise<void>((r) => e.socket.once("connect", () => r()));
    return e;
  }

  clearListeners() {
    this.listeners = [];
  }
  /** Cancel every request that may still be waiting for an answer. */
  cancelPending() {
    for (const e of this.events) {
      if (e.type === "started") this.send({ type: "cancel", id: e.id });
    }
    this.events = [];
  }
  send(req: Record<string, unknown>) {
    this.socket.write(JSON.stringify(req) + "\n");
  }
  onEvent(cb: (e: CoreEvent) => void) {
    this.listeners.push(cb);
  }
  private control(cmd: string): Promise<string> {
    return new Promise((resolve) => {
      const c = net.connect(this.controlPort, "127.0.0.1");
      let out = "";
      c.on("data", (d) => (out += d));
      c.on("close", () => resolve(out));
      c.write(cmd + "\n");
      setTimeout(() => c.end(), 150);
    });
  }
  /** Every connector call and script the engine made, one per line. */
  async calls(): Promise<string[]> {
    return (await this.control("calls")).split("\n").filter(Boolean);
  }
  stop() {
    this.socket?.destroy();
    this.proc?.kill();
  }
}

/** What the desktop shell would have done (window hide/resize, sounds): recorded, not done. */
export interface Shell {
  hidden: boolean;
  sizes: Array<{ width: number; height: number }>;
  sounds: string[];
}

export interface Bar {
  page: Page;
  engine: Engine;
  shell: Shell;
  /** Open the bar the way the hotkey does, and wait until the input has focus. */
  open(): Promise<void>;
  /** Type into the focused input with real key presses and press Enter. */
  say(text: string): Promise<void>;
  /** The text of the result / question line. */
  line(): Promise<string>;
  /** The labels of the buttons in the card (Confirm, Cancel, choices...). */
  buttons(): Promise<string[]>;
  /** Everything the engine did to the simulated PC. */
  calls(): Promise<string[]>;
  /** Save a screenshot of the bar into e2e-results/shots/. */
  shot(name: string): Promise<void>;
}

/** Build the bar helpers for one page on one engine. */
async function makeBar(page: Page, engine: Engine): Promise<Bar> {
  const shell: Shell = { hidden: false, sizes: [], sounds: [] };
  engine.clearListeners();

  // Events go to the page in order.
  let chain = Promise.resolve();
  engine.onEvent((ev) => {
    chain = chain.then(() =>
      page.evaluate((e) => (window as any).__jitmo?.emit("core-event", e), ev).catch(() => undefined),
    );
  });

  await page.exposeFunction("__jitmoInvoke", async (cmd: string, args: any) => {
    switch (cmd) {
      case "submit":
        engine.send({ type: "submit", id: args.id, text: args.text });
        break;
      case "confirm":
        engine.send({ type: "confirm", id: args.id, approved: args.approved });
        break;
      case "choose":
        engine.send({ type: "choose", id: args.id, choice_id: args.choiceId });
        break;
      case "answer":
        engine.send({ type: "answer", id: args.id, text: args.text });
        break;
      case "cancel":
        engine.send({ type: "cancel", id: args.id });
        break;
      case "undo":
        engine.send({ type: "undo", id: args.id });
        break;
      case "hide_bar":
        shell.hidden = true;
        break;
      case "resize_bar":
        shell.sizes.push({ width: args.width, height: args.height });
        break;
      case "bar_sound":
        shell.sounds.push(args.kind);
        break;
      default:
        break; // bar_ready and the like
    }
  });

  const bar: Bar = {
    page,
    engine,
    shell,
    async open() {
      // Opening the bar afresh is what the desktop app does after closing it, and closing
      // cancels anything still waiting for an answer.
      engine.cancelPending();
      await page.goto("/");
      await page.waitForFunction(() => Boolean((window as any).__jitmo));
      await page.evaluate(() => (window as any).__jitmo.emit("bar-shown"));
      await expect(page.locator("input.input")).toBeFocused();
    },
    async say(text) {
      await page.keyboard.type(text);
      await page.keyboard.press("Enter");
    },
    async line() {
      const loc = page.locator(".status .message, .card .line").first();
      // "Working..." is not an answer: wait for the engine to say something.
      await expect
        .poll(async () => (await loc.innerText().catch(() => "Working…")).trim(), { timeout: 12_000 })
        .not.toBe("Working…");
      return (await loc.innerText()).trim();
    },
    async buttons() {
      return (await page.locator(".card button").allInnerTexts()).map((t) => t.replace(/\s+/g, " ").trim());
    },
    calls: () => engine.calls(),
    async shot(name) {
      const dir = path.join(here, "..", "e2e-results", "shots");
      fs.mkdirSync(dir, { recursive: true });
      await page.screenshot({ path: path.join(dir, `${name.replace(/[^a-z0-9._-]+/gi, "_")}.png`) });
    },
  };
  return bar;
}

export const test = base.extend<
  { bar: Bar; fastBar: Bar; flags: string[] },
  { workerEngine: Engine }
>({
  flags: [[], { option: true }],
  // One engine for every test in a worker: fast, for the large corpus. State (timers, volume)
  // carries over between its tests, so these tests must not depend on each other.
  workerEngine: [
    async ({}, use) => {
      const engine = await Engine.start([]);
      await use(engine);
      engine.stop();
    },
    { scope: "worker" },
  ],
  fastBar: async ({ page, workerEngine }, use) => {
    await use(await makeBar(page, workerEngine));
  },
  // A private engine per test: for specs that look at the simulated PC's exact state.
  bar: async ({ page, flags }, use) => {
    const engine = await Engine.start(flags);
    await use(await makeBar(page, engine));
    engine.stop();
  },
});
