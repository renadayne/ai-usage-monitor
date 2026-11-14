import { useEffect, useState } from "react";
import { getSnapshots, onSnapshotUpdated } from "./api";
import { PROVIDER_IDS, type ProviderSnapshot } from "./types";

export function mergeSnapshot(
  current: ProviderSnapshot[],
  incoming: ProviderSnapshot,
): ProviderSnapshot[] {
  const next = current.filter((snapshot) => snapshot.provider !== incoming.provider);
  next.push(incoming);
  next.sort((a, b) => PROVIDER_IDS.indexOf(a.provider) - PROVIDER_IDS.indexOf(b.provider));
  return next;
}

/** Loads the snapshots once and keeps them current from `snapshot-updated` events. */
export function useSnapshots(): { snapshots: ProviderSnapshot[]; error: string | null } {
  const [snapshots, setSnapshots] = useState<ProviderSnapshot[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    onSnapshotUpdated((snapshot) => {
      if (!disposed) setSnapshots((current) => mergeSnapshot(current, snapshot));
    })
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch((cause: unknown) => setError(String(cause)));

    getSnapshots()
      .then((initial) => {
        if (!disposed) setSnapshots((current) => initial.reduce(mergeSnapshot, current));
      })
      .catch((cause: unknown) => {
        if (!disposed) setError(String(cause));
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  return { snapshots, error };
}
