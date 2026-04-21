export type Provider = "claude" | "codex" | "gemini";
export type Status = "ok" | "not_authenticated" | "expired" | "network_error" | "unknown_error";

export interface UsageWindow {
  key: string;
  name: string;
  utilization: number;
  resetsAt: string;
  timeProgress: number;
}

export interface ExtraUsage {
  isEnabled: boolean;
  monthlyLimit: number;
  usedCredits: number;
  utilization: number | null;
}

export interface UsageResponse {
  provider: Provider;
  status: Status;
  windows: UsageWindow[];
  extraUsage?: ExtraUsage;
  error?: string;
}

export interface WindowRect { x: number; y: number; width: number; height: number }

export type ViewMode = "normal" | "mini" | "super";

export interface Settings {
  window: WindowRect;
  alwaysOnTop: boolean;
  opacity: number;
  refreshIntervalSec: number;
  autostart: boolean;
  viewMode: ViewMode;
  closeToTray: boolean;
}
