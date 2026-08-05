/**
 * Notification chime for incoming messages, synthesized with the Web Audio
 * API — no audio file, no dependency. A soft two-tone "ding" (A5 then D6)
 * with a fast attack and exponential decay envelope. Playing requires a user
 * gesture in some browsers, but inside the Tauri webview the context is
 * usually already running; a suspended context is resumed best-effort.
 */

let audioContext: AudioContext | null = null;

/** Lazily create (and resume) the shared audio context. */
function getAudioContext(): AudioContext | null {
  try {
    audioContext ??= new AudioContext();
    if (audioContext.state === "suspended") {
      void audioContext.resume();
    }
    return audioContext;
  } catch {
    return null;
  }
}

/** Play one sine tone with an exponential-decay envelope. */
function playTone(
  context: AudioContext,
  frequency: number,
  startAt: number,
  duration: number,
  peak: number
): void {
  const oscillator = context.createOscillator();
  const gain = context.createGain();
  oscillator.type = "sine";
  oscillator.frequency.setValueAtTime(frequency, startAt);
  gain.gain.setValueAtTime(0.0001, startAt);
  gain.gain.exponentialRampToValueAtTime(peak, startAt + 0.01);
  gain.gain.exponentialRampToValueAtTime(0.0001, startAt + duration);
  oscillator.connect(gain);
  gain.connect(context.destination);
  oscillator.start(startAt);
  oscillator.stop(startAt + duration + 0.02);
}

/** Play the incoming-message notification sound. Fails silently when the Web
 *  Audio API is unavailable. */
export function playNotificationSound(): void {
  const context = getAudioContext();
  if (!context) return;
  const now = context.currentTime;
  playTone(context, 880, now, 0.15, 0.3);
  playTone(context, 1174.66, now + 0.12, 0.22, 0.25);
}
