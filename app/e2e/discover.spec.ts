import { test } from "./harness";
import fs from "node:fs";

const INPUTS = [
  "open spotify","opne spotify","open the calculator","open vs code","open notepad","open",
  "play kesariya","play janam janam song","plya despacito","play","pause the music","skip this song","music",
  "18% of 2450","2+2","five plus five","sqrt of 144","calculate",
  "time in tokyo","what time is it in london","sunrise in pune","when is the next full moon",
  "find invoice","show my recent downloads",
  "remind me to call mom at 6pm","remind me to call mom","remind me","remind me tmrw at 9 to pay rent","remind me yesterday at 5pm to call",
  "set a 10 minute timer","start a timer","what timers do i have","stop the timer",
  "volume 50","turn the volume up","mute","brightness 40","volume",
  "lock my laptop","take a screenshot",
  "turn off wifi","turn of bluetooth","wifi","wifi chalu karo","turn on bluetooth",
  "turn on do not disturb","dnd","turn off dnd",
  "will it rain in pune tomorrow","will it rain tomorrow","weather in delhi",
  "capital of australia","who wrote hamlet",
  "search the web for pasta recipes","go to amazon.in","search","latest video from fireship",
  "set my city to pune","what can you do","help","hey jitmo play kesariya","send hi to ved","what is the meaning of life",
];
const CHUNKS = 4;

for (let c = 0; c < CHUNKS; c++) {
  test(`discover ${c}`, async ({ bar }) => {
    test.setTimeout(900_000);
    fs.mkdirSync("e2e-results", { recursive: true });
    const file = `e2e-results/discovery-${c}.txt`;
    fs.writeFileSync(file, "");
    for (let i = c; i < INPUTS.length; i += CHUNKS) {
      const input = INPUTS[i];
      await bar.open();
      await bar.page.keyboard.type(input, { delay: 0 });
      await bar.page.keyboard.press("Enter");
      let line = "";
      try { line = await bar.line(); } catch { line = "<<no answer>>"; }
      const buttons = await bar.buttons().catch(() => []);
      const undo = await bar.page.getByRole("button", { name: /Undo/ }).count();
      fs.appendFileSync(file, `${String(i).padStart(2)} ${input.padEnd(34)} => ${line}${buttons.length ? "  [" + buttons.join(" | ") + "]" : ""}${undo ? "  (Undo)" : ""}\n`);
    }
  });
}
