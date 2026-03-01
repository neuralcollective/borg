import { useState, useRef, useCallback, useEffect } from "react";

interface SpeechRecognitionEvent {
  results: { [index: number]: { [index: number]: { transcript: string } }; length: number };
  resultIndex: number;
}

interface SpeechRecognitionInstance extends EventTarget {
  continuous: boolean;
  interimResults: boolean;
  lang: string;
  start(): void;
  stop(): void;
  abort(): void;
  onresult: ((event: SpeechRecognitionEvent) => void) | null;
  onend: (() => void) | null;
  onerror: ((event: { error: string }) => void) | null;
}

type SpeechRecognitionConstructor = new () => SpeechRecognitionInstance;

function getSpeechRecognition(): SpeechRecognitionConstructor | null {
  const w = window as unknown as Record<string, unknown>;
  return (w.SpeechRecognition ?? w.webkitSpeechRecognition ?? null) as SpeechRecognitionConstructor | null;
}

export function useDictation(onTranscript: (text: string) => void) {
  const [listening, setListening] = useState(false);
  const [supported] = useState(() => getSpeechRecognition() !== null);
  const recRef = useRef<SpeechRecognitionInstance | null>(null);
  const onTranscriptRef = useRef(onTranscript);
  onTranscriptRef.current = onTranscript;

  useEffect(() => {
    return () => recRef.current?.abort();
  }, []);

  const toggle = useCallback(() => {
    if (listening && recRef.current) {
      recRef.current.stop();
      return;
    }

    const Ctor = getSpeechRecognition();
    if (!Ctor) return;

    const rec = new Ctor();
    rec.continuous = true;
    rec.interimResults = false;
    rec.lang = "en-US";

    rec.onresult = (e: SpeechRecognitionEvent) => {
      const last = e.results[e.results.length - 1];
      if (last?.[0]?.transcript) {
        onTranscriptRef.current(last[0].transcript.trim());
      }
    };

    rec.onend = () => {
      setListening(false);
      recRef.current = null;
    };

    rec.onerror = (e: { error: string }) => {
      if (e.error !== "aborted") console.warn("Speech recognition error:", e.error);
      setListening(false);
      recRef.current = null;
    };

    recRef.current = rec;
    rec.start();
    setListening(true);
  }, [listening]);

  return { listening, supported, toggle };
}
