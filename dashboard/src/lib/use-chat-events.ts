import { useEffect, useRef } from "react";
import { sseUrl, tokenReady } from "./api";

type ChatEventBase = {
  thread?: string;
};

export function useChatEvents<T extends ChatEventBase>(
  thread: string | null,
  onMessage: (msg: T) => void,
  onDisconnect?: () => void,
  maxRetries = 5,
) {
  const esRef = useRef<EventSource | null>(null);
  const retriesRef = useRef(0);
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (!thread) return;
    retriesRef.current = 0;

    function connect() {
      if (esRef.current) esRef.current.close();
      tokenReady.then(() => {
        const es = new EventSource(sseUrl("/api/chat/events"));
        esRef.current = es;

        es.onopen = () => {
          retriesRef.current = 0;
        };

        es.onmessage = (e) => {
          try {
            const msg: T = JSON.parse(e.data);
            if ((msg.thread ?? "") !== thread) return;
            onMessage(msg);
          } catch {
            // ignore malformed events
          }
        };

        es.onerror = () => {
          es.close();
          esRef.current = null;
          onDisconnect?.();
          if (retriesRef.current < maxRetries) {
            const delay = Math.min(1000 * Math.pow(2, retriesRef.current), 30_000);
            retriesRef.current++;
            retryTimerRef.current = setTimeout(connect, delay);
          }
        };
      });
    }

    connect();
    return () => {
      esRef.current?.close();
      if (retryTimerRef.current) clearTimeout(retryTimerRef.current);
    };
  }, [thread, onMessage, onDisconnect, maxRetries]);
}
