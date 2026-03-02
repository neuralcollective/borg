import { createContext, useContext, type ReactNode } from "react";

export type DomainProfile = "legal" | "general";

export interface DomainConfig {
  profile: DomainProfile;
  brandName: string;
  tagline: string;
  accentColor: string;
  accentBg: string;
  defaultMode: "minimal" | "advanced";
  defaultView: "tasks" | "projects";
  hiddenNavKeys: string[];
}

const PROFILES: Record<DomainProfile, DomainConfig> = {
  legal: {
    profile: "legal",
    brandName: "BORG",
    tagline: "Legal AI",
    accentColor: "text-blue-400",
    accentBg: "bg-blue-600",
    defaultMode: "minimal",
    defaultView: "projects",
    hiddenNavKeys: ["logs", "queue", "knowledge", "creator"],
  },
  general: {
    profile: "general",
    brandName: "BORG",
    tagline: "AI Agent Orchestrator",
    accentColor: "text-orange-400",
    accentBg: "bg-orange-500",
    defaultMode: "advanced",
    defaultView: "tasks",
    hiddenNavKeys: [],
  },
};

function detectProfile(): DomainProfile {
  if (typeof window === "undefined") return "general";
  const host = window.location.hostname;
  if (host.endsWith("borg.legal") || host === "borg.legal") return "legal";
  return "general";
}

const ctx = createContext<DomainConfig>(PROFILES.general);

export function DomainProvider({ children }: { children: ReactNode }) {
  const config = PROFILES[detectProfile()];
  return <ctx.Provider value={config}>{children}</ctx.Provider>;
}

export function useDomain(): DomainConfig {
  return useContext(ctx);
}
