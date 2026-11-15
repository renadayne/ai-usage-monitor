import { useEffect, useState } from "react";
import {
  copilotConnect,
  copilotDisconnect,
  copilotOpenVerification,
  copilotStatus,
  onCopilotAuth,
  saveSettings,
} from "./lib/api";
import {
  DEFAULT_POLL_SECS,
  MIN_POLL_SECS,
  PROVIDER_IDS,
  PROVIDER_NAMES,
  type DeviceCode,
  type ProviderId,
  type Settings as SettingsData,
} from "./lib/types";

const INPUT =
  "rounded-md border border-slate-300 bg-white px-2 py-1 text-sm dark:border-slate-600 dark:bg-slate-900";
const BUTTON =
  "rounded-md border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 disabled:opacity-50 dark:border-slate-600 dark:hover:bg-slate-700";
const CARD =
  "rounded-xl border border-slate-200 bg-white p-4 shadow-sm dark:border-slate-700 dark:bg-slate-800";

function CopilotConnection() {
  const [connected, setConnected] = useState(false);
  const [code, setCode] = useState<DeviceCode | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    copilotStatus()
      .then((status) => {
        if (!disposed) setConnected(status.connected);
      })
      .catch(() => {});
    onCopilotAuth((event) => {
      if (disposed) return;
      setCode(null);
      setConnected(event.state === "connected");
      setMessage(event.state === "connected" ? "GitHub connected." : (event.message ?? "Failed."));
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

  const connect = async () => {
    setBusy(true);
    setMessage(null);
    try {
      setCode(await copilotConnect());
    } catch (cause) {
      setMessage(String(cause));
    } finally {
      setBusy(false);
    }
  };

  const disconnect = async () => {
    await copilotDisconnect();
    setConnected(false);
    setCode(null);
    setMessage("GitHub disconnected.");
  };

  return (
    <section className={CARD} aria-label="GitHub Copilot connection">
      <h2 className="text-base font-semibold">GitHub Copilot connection</h2>
      <p className="mt-1 text-sm text-slate-600 dark:text-slate-300">
        Only needed when no GitHub token is found on this computer. The token is stored in Windows
        Credential Manager and is only sent to github.com.
      </p>
      <div className="mt-3 flex flex-wrap items-center gap-3">
        <span className="text-sm">{connected ? "Connected" : "Not connected"}</span>
        {connected ? (
          <button type="button" className={BUTTON} onClick={() => void disconnect()}>
            Disconnect
          </button>
        ) : (
          <button type="button" className={BUTTON} disabled={busy} onClick={() => void connect()}>
            Connect GitHub
          </button>
        )}
      </div>
      {code && (
        <div className="mt-3 rounded-md bg-slate-100 p-3 text-sm dark:bg-slate-900">
          <p>
            Enter this code on GitHub:{" "}
            <strong className="font-mono text-base tracking-widest">{code.user_code}</strong>
          </p>
          <div className="mt-2 flex flex-wrap gap-2">
            <button type="button" className={BUTTON} onClick={() => void copilotOpenVerification()}>
              Open github.com/login/device
            </button>
            <button
              type="button"
              className={BUTTON}
              onClick={() => void navigator.clipboard?.writeText(code.user_code)}
            >
              Copy code
            </button>
          </div>
          <p className="mt-2 text-xs text-slate-500 dark:text-slate-400">
            Waiting for approval… the code expires in {Math.round(code.expires_in / 60)} minutes.
          </p>
        </div>
      )}
      {message && (
        <p role="status" className="mt-2 text-sm text-slate-600 dark:text-slate-300">
          {message}
        </p>
      )}
    </section>
  );
}

export interface SettingsProps {
  settings: SettingsData;
  onSaved: (settings: SettingsData) => void;
}

export default function Settings({ settings, onSaved }: SettingsProps) {
  const [draft, setDraft] = useState<SettingsData>(settings);
  const [status, setStatus] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const dirty = JSON.stringify(draft) !== JSON.stringify(settings);

  const update = (patch: Partial<SettingsData>) => {
    setDraft((current) => ({ ...current, ...patch }));
    setStatus(null);
  };

  const updateProvider = (id: ProviderId, patch: Partial<SettingsData["providers"][ProviderId]>) =>
    update({ providers: { ...draft.providers, [id]: { ...draft.providers[id], ...patch } } });

  const save = async () => {
    setSaving(true);
    try {
      const stored = await saveSettings(draft);
      setDraft(stored);
      onSaved(stored);
      setStatus("Saved. Changes are active now.");
    } catch (cause) {
      setStatus(`Could not save: ${String(cause)}`);
    } finally {
      setSaving(false);
    }
  };

  return (
    <form
      className="flex flex-col gap-4"
      onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <section className={CARD} aria-label="Providers">
        <h2 className="text-base font-semibold">Providers</h2>
        <p className="mt-1 text-xs text-slate-500 dark:text-slate-400">
          Leave the interval empty for the default. Each provider has a minimum so it does not get
          rate limited.
        </p>
        <div className="mt-3 flex flex-col gap-2">
          {PROVIDER_IDS.map((id) => {
            const prefs = draft.providers[id];
            return (
              <div key={id} className="flex flex-wrap items-center justify-between gap-2">
                <label className="flex items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={prefs.enabled}
                    onChange={(event) => updateProvider(id, { enabled: event.target.checked })}
                  />
                  {PROVIDER_NAMES[id]}
                </label>
                <label className="flex items-center gap-2 text-xs text-slate-500 dark:text-slate-400">
                  Refresh every
                  <input
                    type="number"
                    aria-label={`${PROVIDER_NAMES[id]} refresh interval in seconds`}
                    className={`${INPUT} w-24`}
                    min={MIN_POLL_SECS[id]}
                    max={21600}
                    step={10}
                    placeholder={String(DEFAULT_POLL_SECS[id])}
                    value={prefs.poll_interval_secs ?? ""}
                    disabled={!prefs.enabled}
                    onChange={(event) =>
                      updateProvider(id, {
                        poll_interval_secs:
                          event.target.value === "" ? null : Number(event.target.value),
                      })
                    }
                  />
                  seconds (min {MIN_POLL_SECS[id]})
                </label>
              </div>
            );
          })}
        </div>
      </section>

      <section className={CARD} aria-label="Notifications">
        <h2 className="text-base font-semibold">Notifications</h2>
        <label className="mt-3 flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={draft.notifications_enabled}
            onChange={(event) => update({ notifications_enabled: event.target.checked })}
          />
          Notify when a quota runs low or resets
        </label>
        <div className="mt-3 flex flex-wrap gap-4 text-sm">
          <label className="flex items-center gap-2">
            Warning at
            <input
              type="number"
              aria-label="Warning threshold percent"
              className={`${INPUT} w-20`}
              min={1}
              max={99}
              value={draft.warn_threshold}
              disabled={!draft.notifications_enabled}
              onChange={(event) => update({ warn_threshold: Number(event.target.value) })}
            />
            %
          </label>
          <label className="flex items-center gap-2">
            Critical at
            <input
              type="number"
              aria-label="Critical threshold percent"
              className={`${INPUT} w-20`}
              min={1}
              max={100}
              value={draft.critical_threshold}
              disabled={!draft.notifications_enabled}
              onChange={(event) => update({ critical_threshold: Number(event.target.value) })}
            />
            %
          </label>
        </div>
      </section>

      <section className={CARD} aria-label="Startup and folders">
        <h2 className="text-base font-semibold">Startup and folders</h2>
        <label className="mt-3 flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={draft.autostart}
            onChange={(event) => update({ autostart: event.target.checked })}
          />
          Start with Windows (in the tray)
        </label>
        <p className="mt-3 text-xs text-slate-500 dark:text-slate-400">
          Folders are found automatically (environment variable, your user folder, then WSL). Set
          them only if detection fails.
        </p>
        <div className="mt-2 flex flex-col gap-2 text-sm">
          <label className="flex flex-col gap-1">
            Claude Code folder
            <input
              type="text"
              className={INPUT}
              placeholder={"C:\\Users\\you\\.claude"}
              value={draft.claude_dir ?? ""}
              onChange={(event) => update({ claude_dir: event.target.value || null })}
            />
          </label>
          <label className="flex flex-col gap-1">
            Codex folder
            <input
              type="text"
              className={INPUT}
              placeholder={"C:\\Users\\you\\.codex"}
              value={draft.codex_dir ?? ""}
              onChange={(event) => update({ codex_dir: event.target.value || null })}
            />
          </label>
        </div>
      </section>

      <div className="flex items-center gap-3">
        <button type="submit" className={BUTTON} disabled={!dirty || saving}>
          Save settings
        </button>
        {status && (
          <span role="status" className="text-sm text-slate-600 dark:text-slate-300">
            {status}
          </span>
        )}
      </div>

      <CopilotConnection />
    </form>
  );
}
