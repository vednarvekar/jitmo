import { test, expect } from "./harness";

test("turn off bluetooth runs, reads back, and offers Undo", async ({ bar }) => {
  await bar.open();
  await bar.say("turn off bluetooth");
  expect(await bar.line()).toBe("Bluetooth is off.");
  await expect(bar.page.getByRole("button", { name: /Undo/ })).toBeVisible();
  const calls = await bar.calls();
  expect(calls.some((c) => c.includes('JITMO_KIND="Bluetooth", JITMO_STATE="Off"'))).toBe(true);
});

test("a bare 'volume' shows quick buttons, and a click finishes the job", async ({ bar }) => {
  await bar.open();
  await bar.say("volume");
  expect(await bar.line()).toBe("What should I do with the volume?");
  expect(await bar.buttons()).toEqual(["1 Louder", "2 Quieter", "3 Mute", "4 Set a level..."]);
  await bar.page.getByRole("button", { name: /Quieter/ }).click();
  await expect(bar.page.locator(".status .message")).toContainText("Volume");
});

test("'start a timer' asks how long with quick choices", async ({ bar }) => {
  await bar.open();
  await bar.say("start a timer");
  expect(await bar.line()).toBe("How long should the timer run?");
  expect((await bar.buttons()).length).toBe(6);
  await bar.page.keyboard.press("2"); // the number keys pick
  await expect(bar.page.locator(".status .message")).toContainText("10 minutes");
});

test("typing keeps focus in the input; Esc closes the bar", async ({ bar }) => {
  await bar.open();
  await bar.page.keyboard.type("hello");
  await expect(bar.page.locator("input.input")).toHaveValue("hello");
  await bar.page.keyboard.press("Escape");
  await expect.poll(() => bar.shell.hidden).toBe(true);
});

test("'turn on blue' (a half-typed radio name) still turns Bluetooth on", async ({ bar }) => {
  await bar.open();
  await bar.say("turn on blue");
  expect(await bar.line()).toBe("Bluetooth is on.");
});

test("a radio that cannot be told apart asks with buttons, never a bare question", async ({ bar }) => {
  await bar.open();
  await bar.say("turn on");
  expect(await bar.line()).toBe("Turn on what?");
  expect(await bar.buttons()).toEqual(["1 Wi-Fi", "2 Bluetooth", "3 Do Not Disturb"]);
});

test("after a typed answer the bar shows it is working, not the old question", async ({ bar }) => {
  await bar.open();
  await bar.say("weather");
  expect(await bar.line()).toBe("Which city?");
  await bar.page.keyboard.type("Pune");
  await bar.page.keyboard.press("Enter");
  // The old question must be gone at once; the result (or a working word) takes its place.
  await expect(bar.page.locator(".card")).toHaveCount(0);
});

test("a ringing alarm stays until Esc, Enter or Backspace", async ({ bar }) => {
  for (const key of ["Escape", "Enter", "Backspace"]) {
    await bar.open();
    await bar.page.evaluate(() => (window as any).__jitmo.emit("bar-alarm", "Reminder: Call mom"));
    await expect(bar.page.locator(".status.alarm")).toContainText("Call mom");
    await expect(bar.page.locator(".alarm-hint")).toBeVisible();
    // Typing does not leak into the bar while it rings.
    await bar.page.keyboard.type("x");
    await expect(bar.page.locator(".status.alarm")).toBeVisible();
    await bar.page.keyboard.press(key);
    await expect.poll(() => bar.shell.hidden).toBe(true);
  }
});
