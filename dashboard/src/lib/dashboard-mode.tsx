import { createContext, useContext, useMemo, type ReactNode } from "react";
import { useSettings } from "./api";

export type DashboardMode = "general" | "legal";

interface DashboardModeCtx {
  mode: DashboardMode;
  isLegal: boolean;
}

const ctx = createContext<DashboardModeCtx>({ mode: "general", isLegal: false });

export function DashboardModeProvider({ children }: { children: ReactNode }) {
  const { data: settings } = useSettings();
  const value = useMemo<DashboardModeCtx>(() => {
    const mode = (settings?.dashboard_mode === "legal" ? "legal" : "general") as DashboardMode;
    return { mode, isLegal: mode === "legal" };
  }, [settings?.dashboard_mode]);
  return <ctx.Provider value={value}>{children}</ctx.Provider>;
}

export function useDashboardMode(): DashboardModeCtx {
  return useContext(ctx);
}
