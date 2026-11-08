import { Mic, Waveform } from "./Icons";

/** Voice is not built yet. The icons are part of the design; they are dimmed,
 *  skipped by the keyboard, and say so in their tooltip. */
export default function VoiceButtons() {
  return (
    <>
      <button
        className="tool"
        aria-disabled="true"
        aria-label="Voice input (not available yet)"
        title="Voice input is not available yet"
        tabIndex={-1}
      >
        <Mic />
      </button>
      <button
        className="tool"
        aria-disabled="true"
        aria-label="Voice conversation (not available yet)"
        title="Voice conversation is not available yet"
        tabIndex={-1}
      >
        <Waveform />
      </button>
    </>
  );
}
