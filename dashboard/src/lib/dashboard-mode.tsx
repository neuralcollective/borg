import { createContext, useContext, useMemo, type ReactNode } from "react";
import { useSettings } from "./api";

export type DashboardMode = "general" | "legal" | "knowledge";

interface DashboardModeCtx {
  mode: DashboardMode;
  isLegal: boolean;
  isSWE: boolean;
}

const ctx = createContext<DashboardModeCtx>({ mode: "general", isLegal: false, isSWE: false });

export function DashboardModeProvider({ children }: { children: ReactNode }) {
  const { data: settings } = useSettings();
  const value = useMemo<DashboardModeCtx>(() => {
    const raw = settings?.dashboard_mode;
    const mode: DashboardMode = raw === "legal" ? "legal" : raw === "knowledge" ? "knowledge" : "general";
    return { mode, isLegal: mode === "legal", isSWE: mode === "general" };
  }, [settings?.dashboard_mode]);
  return <ctx.Provider value={value}>{children}</ctx.Provider>;
}

export function useDashboardMode(): DashboardModeCtx {
  return useContext(ctx);
}
