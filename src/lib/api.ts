import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { demoSnapshots } from "./demo";
import type { ProviderId, ProviderSnapshot } from "./types";

export const EVENT_SNAPSHOT_UPDATED = "snapshot-updated";

/** True for `pnpm dev` opened in a plain browser: there is no backend, so sample data is shown. */
const DEMO = import.meta.env.DEV && !("__TAURI_INTERNALS__" in globalThis);

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
  if (DEMO) return Promise.resolve(() => {});
  return listen<ProviderSnapshot>(EVENT_SNAPSHOT_UPDATED, (event) => handler(event.payload));
}
