import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Provider, Settings, UsageResponse } from "./types";

export interface ProviderSnapshot {
  fetchedAt: string;
  response: UsageResponse;
}

export interface ProviderUpdatedPayload {
  provider: Provider;
  snapshot: ProviderSnapshot;
}

export interface UsageRefreshingPayload {
  provider: Provider;
  manual: boolean;
}

export const ipc = {
  getAllSnapshots: () =>
    invoke<Record<string, ProviderSnapshot>>("get_all_snapshots"),
  getProviderUsage: (provider: Provider, force = false) =>
    invoke<ProviderSnapshot>("get_provider_usage", { provider, force }),
  refreshAllInBackground: () => invoke<void>("refresh_all_in_background"),
  refreshViaCli: (provider: Provider) => invoke<void>("refresh_via_cli", { provider }),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
  openUrl: (url: string) => invoke<void>("open_url", { url }),

  onProviderUpdated: (cb: (p: ProviderUpdatedPayload) => void): Promise<UnlistenFn> =>
    listen<ProviderUpdatedPayload>("usage:provider_updated", (e) => cb(e.payload)),
  onUsageRefreshing: (cb: (p: UsageRefreshingPayload) => void): Promise<UnlistenFn> =>
    listen<UsageRefreshingPayload>("usage:refreshing", (e) => cb(e.payload)),
};
