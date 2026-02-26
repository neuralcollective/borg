import { createContext, useContext, useState, useEffect, useCallback, type ReactNode } from "react";

export type UIMode = "minimal" | "advanced";

interface UIModeContext {
  mode: UIMode;
  setMode: (mode: UIMode) => void;
}

const ctx = createContext<UIModeContext>({
  mode: "advanced",
  setMode: () => {},
});

const STORAGE_KEY = "borg-ui-mode";

export function UIModeProvider({ defaultMode, children }: { defaultMode: UIMode; children: ReactNode }) {
  const [mode, setModeRaw] = useState<UIMode>(() => {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === "minimal" || stored === "advanced") return stored;
    return defaultMode;
  });

  // If defaultMode changes (e.g. modes API loads) and user never set a preference, update
  useEffect(() => {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (!stored) setModeRaw(defaultMode);
  }, [defaultMode]);

  const setMode = useCallback((m: UIMode) => {
    localStorage.setItem(STORAGE_KEY, m);
    setModeRaw(m);
  }, []);

  return (
    <ctx.Provider value={{ mode, setMode }}>
      {children}
    </ctx.Provider>
  );
}

export function useUIMode() {
  return useContext(ctx);
}
