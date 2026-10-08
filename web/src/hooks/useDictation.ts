import { useEffect, useRef, useState } from "react";

/** Long enough for anything said to a house; short enough that a
    forgotten recording stops on its own. */
const LIMIT_MS = 60_000;

/** Whether this browser can record at all. Older ones cannot, and get
    no microphone button rather than one that fails. */
export function dictationSupported(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.MediaRecorder !== "undefined" &&
    typeof navigator.mediaDevices?.getUserMedia === "function"
  );
}

/** Compressed, and something both Niles's transcribers decode: Chrome
    records WebM, Safari MP4. */
function mimeType(): string | undefined {
  return ["audio/webm;codecs=opus", "audio/mp4", "audio/webm"].find((t) =>
    MediaRecorder.isTypeSupported?.(t),
  );
}

/**
 * Record from the microphone until stopped, then hand over the audio.
 */
export function useDictation(onAudio: (audio: Blob) => void) {
  const [recording, setRecording] = useState(false);
  const [error, setError] = useState<string>();
  const recorder = useRef<MediaRecorder | null>(null);
  // Leaving the page mid-sentence abandons it rather than sending it.
  const abandoned = useRef(false);

  useEffect(
    () => () => {
      abandoned.current = true;
      if (recorder.current?.state === "recording") recorder.current.stop();
    },
    [],
  );

  const start = async () => {
    setError(undefined);
    let stream: MediaStream;
    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    } catch {
      setError("Niles needs the microphone for that. Allow it in the browser's settings.");
      return;
    }
    const type = mimeType();
    const rec = new MediaRecorder(stream, type ? { mimeType: type } : undefined);
    const chunks: Blob[] = [];
    const limit = window.setTimeout(() => rec.state === "recording" && rec.stop(), LIMIT_MS);
    rec.ondataavailable = (e) => {
      if (e.data.size > 0) chunks.push(e.data);
    };
    rec.onstop = () => {
      window.clearTimeout(limit);
      // Until the tracks stop, the phone keeps showing the microphone
      // as in use.
      stream.getTracks().forEach((t) => t.stop());
      recorder.current = null;
      setRecording(false);
      const audio = new Blob(chunks, { type: rec.mimeType || type || "audio/webm" });
      if (!abandoned.current && audio.size > 0) onAudio(audio);
    };
    recorder.current = rec;
    rec.start();
    setRecording(true);
  };

  const stop = () => {
    if (recorder.current?.state === "recording") recorder.current.stop();
  };

  return { recording, start, stop, error };
}
