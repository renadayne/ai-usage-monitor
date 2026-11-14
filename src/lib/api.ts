import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ProviderId, ProviderSnapshot } from "./types";

export const EVENT_SNAPSHOT_UPDATED = "snapshot-updated";

export function getSnapshots(): Promise<ProviderSnapshot[]> {
  return invoke<ProviderSnapshot[]>("get_snapshots");
}

export function refresh(provider?: ProviderId): Promise<void> {
  return invoke<void>("refresh", { provider: provider ?? null });
}

export function onSnapshotUpdated(
  handler: (snapshot: ProviderSnapshot) => void,
): Promise<UnlistenFn> {
  return listen<ProviderSnapshot>(EVENT_SNAPSHOT_UPDATED, (event) => handler(event.payload));
}
