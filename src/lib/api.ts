import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { demoHistory, demoSettings, demoSnapshots } from "./demo";
import type {
  CopilotAuthEvent,
  DeviceCode,
  HistoryPoint,
  ProviderId,
  ProviderSnapshot,
  Settings,
} from "./types";

export const EVENT_SNAPSHOT_UPDATED = "snapshot-updated";
export const EVENT_SETTINGS_CHANGED = "settings-changed";
export const EVENT_COPILOT_AUTH = "copilot-auth";

/** True for `pnpm dev` opened in a plain browser: there is no backend, so sample data is shown. */
const DEMO = import.meta.env.DEV && !("__TAURI_INTERNALS__" in globalThis);

const noop: UnlistenFn = () => {};

export function getSnapshots(): Promise<ProviderSnapshot[]> {
  if (DEMO) return Promise.resolve(demoSnapshots());
  return invoke<ProviderSnapshot[]>("get_snapshots");
}

export function refresh(provider?: ProviderId): Promise<void> {
  if (DEMO) return Promise.resolve();
  return invoke<void>("refresh", { provider: provider ?? null });
}

export function showDashboard(): Promise<void> {
  if (DEMO) return Promise.resolve();
  return invoke<void>("show_dashboard");
}

export function onSnapshotUpdated(
  handler: (snapshot: ProviderSnapshot) => void,
): Promise<UnlistenFn> {
  if (DEMO) return Promise.resolve(noop);
  return listen<ProviderSnapshot>(EVENT_SNAPSHOT_UPDATED, (event) => handler(event.payload));
}

export function getSettings(): Promise<Settings> {
  if (DEMO) return Promise.resolve(demoSettings());
  return invoke<Settings>("get_settings");
}

/** Saves and applies the settings; resolves with what the backend actually stored. */
export function saveSettings(settings: Settings): Promise<Settings> {
  if (DEMO) return Promise.resolve(settings);
  return invoke<Settings>("set_settings", { settings });
}

export function onSettingsChanged(handler: (settings: Settings) => void): Promise<UnlistenFn> {
  if (DEMO) return Promise.resolve(noop);
  return listen<Settings>(EVENT_SETTINGS_CHANGED, (event) => handler(event.payload));
}

export function getHistory(provider: ProviderId, days: number): Promise<HistoryPoint[]> {
  if (DEMO) return Promise.resolve(demoHistory(provider, days));
  return invoke<HistoryPoint[]>("get_history", { provider, days });
}

export function copilotStatus(): Promise<{ connected: boolean }> {
  if (DEMO) return Promise.resolve({ connected: false });
  return invoke<{ connected: boolean }>("copilot_status");
}

export function copilotConnect(): Promise<DeviceCode> {
  if (DEMO) {
    return Promise.resolve({
      user_code: "DEMO-CODE",
      verification_uri: "https://github.com/login/device",
      expires_in: 900,
      interval: 5,
    });
  }
  return invoke<DeviceCode>("copilot_connect");
}

export function copilotOpenVerification(): Promise<void> {
  if (DEMO) return Promise.resolve();
  return invoke<void>("copilot_open_verification");
}

export function copilotDisconnect(): Promise<void> {
  if (DEMO) return Promise.resolve();
  return invoke<void>("copilot_disconnect");
}

export function onCopilotAuth(handler: (event: CopilotAuthEvent) => void): Promise<UnlistenFn> {
  if (DEMO) return Promise.resolve(noop);
  return listen<CopilotAuthEvent>(EVENT_COPILOT_AUTH, (event) => handler(event.payload));
}
