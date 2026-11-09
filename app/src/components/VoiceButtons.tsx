import { Waveform } from "./Icons";

/** Voice is not built yet. Only the voice-conversation icon is shown (the plain mic is
 *  gone for now); it is dimmed, skipped by the keyboard, and says so in its tooltip.
 *  `Mic` stays in Icons.tsx for when dictation comes back. */
export default function VoiceButtons() {
  return (
    <button
      className="tool"
      aria-disabled="true"
      aria-label="Voice conversation (not available yet)"
      title="Voice is not available yet"
      tabIndex={-1}
    >
      <Waveform />
    </button>
  );
}
