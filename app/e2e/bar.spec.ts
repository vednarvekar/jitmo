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
