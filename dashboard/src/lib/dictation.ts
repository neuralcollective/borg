import { useState, useRef, useCallback, useEffect } from "react";

interface SpeechRecognitionResult {
  isFinal: boolean;
  [index: number]: { transcript: string };
}

interface SpeechRecognitionEvent {
  results: { [index: number]: SpeechRecognitionResult; length: number };
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

export function useDictation(currentInput: string, setInput: (value: string) => void) {
  const [listening, setListening] = useState(false);
  const [supported] = useState(() => getSpeechRecognition() !== null);
  const recRef = useRef<SpeechRecognitionInstance | null>(null);
  const baseRef = useRef("");
  const inputRef = useRef(currentInput);
  inputRef.current = currentInput;
  const setInputRef = useRef(setInput);
  setInputRef.current = setInput;

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

    baseRef.current = inputRef.current;

    const rec = new Ctor();
    rec.continuous = true;
    rec.interimResults = true;
    rec.lang = "en-US";

    rec.onresult = (e: SpeechRecognitionEvent) => {
      let finals = "";
      let interim = "";
      for (let i = 0; i < e.results.length; i++) {
        const transcript = e.results[i][0]?.transcript ?? "";
        if (e.results[i].isFinal) {
          finals += transcript;
        } else {
          interim += transcript;
        }
      }
      const dictated = (finals + interim).trim();
      const base = baseRef.current;
      setInputRef.current(base ? base + " " + dictated : dictated);
    };

    rec.onend = () => {
      // On end, finalize with only committed text (drop incomplete interim)
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
