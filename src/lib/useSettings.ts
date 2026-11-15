import { useEffect, useState } from "react";
import { getSettings, onSettingsChanged } from "./api";
import type { Settings as SettingsData } from "./types";

/** Loads the settings and keeps them in sync with changes made in any window. */
export function useSettings(): [SettingsData | null, (settings: SettingsData) => void] {
  const [settings, setSettings] = useState<SettingsData | null>(null);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    getSettings()
      .then((loaded) => {
        if (!disposed) setSettings(loaded);
      })
      .catch(() => {});
    onSettingsChanged((changed) => {
      if (!disposed) setSettings(changed);
    })
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  return [settings, setSettings];
}
