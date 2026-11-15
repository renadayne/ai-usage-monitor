import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import * as api from "./lib/api";
import { demoSettings, demoSnapshots } from "./lib/demo";
import { mergeSnapshot } from "./lib/useSnapshots";

const windowLabel = vi.hoisted(() => ({ value: "main" }));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: windowLabel.value }),
}));

vi.mock("./lib/api", async () => {
  const { demoSnapshots } = await import("./lib/demo");
  return {
    getSnapshots: vi.fn(() => Promise.resolve(demoSnapshots())),
    onSnapshotUpdated: vi.fn(() => Promise.resolve(() => {})),
    refresh: vi.fn(() => Promise.resolve()),
    showDashboard: vi.fn(() => Promise.resolve()),
    getSettings: vi.fn(() => import("./lib/demo").then((demo) => demo.demoSettings())),
    onSettingsChanged: vi.fn(() => Promise.resolve(() => {})),
    getHistory: vi.fn(() => Promise.resolve([])),
    saveSettings: vi.fn((settings: unknown) => Promise.resolve(settings)),
    copilotStatus: vi.fn(() => Promise.resolve({ connected: false })),
    onCopilotAuth: vi.fn(() => Promise.resolve(() => {})),
  };
});

beforeEach(() => {
  windowLabel.value = "main";
  vi.clearAllMocks();
});

describe("App", () => {
  it("renders the dashboard with one card per provider", async () => {
    render(<App />);
    expect(screen.getByRole("heading", { name: "AI Usage Monitor" })).toBeInTheDocument();
    expect(await screen.findByRole("region", { name: "Claude" })).toBeInTheDocument();
    for (const name of ["Codex", "Antigravity", "GitHub Copilot"]) {
      expect(screen.getByRole("region", { name })).toBeInTheDocument();
    }
  });

  it("refreshes everything or a single provider", async () => {
    render(<App />);
    await screen.findByRole("region", { name: "Codex" });
    fireEvent.click(screen.getByRole("button", { name: "Refresh all" }));
    expect(api.refresh).toHaveBeenLastCalledWith();
    fireEvent.click(screen.getByRole("button", { name: "Refresh Codex" }));
    expect(api.refresh).toHaveBeenLastCalledWith("codex");
  });

  it("switches between overview, history and settings", async () => {
    render(<App />);
    await screen.findByRole("region", { name: "Claude" });
    fireEvent.click(screen.getByRole("button", { name: "History" }));
    expect(await screen.findByRole("region", { name: "Claude history" })).toBeInTheDocument();
    expect(api.getHistory).toHaveBeenCalledWith("claude", 7);
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(await screen.findByRole("button", { name: "Save settings" })).toBeInTheDocument();
  });

  it("hides providers that are turned off", async () => {
    const settings = demoSettings();
    settings.providers.codex.enabled = false;
    vi.mocked(api.getSettings).mockResolvedValueOnce(settings);
    render(<App />);
    await screen.findByRole("region", { name: "Claude" });
    await waitFor(() =>
      expect(screen.queryByRole("region", { name: "Codex" })).not.toBeInTheDocument(),
    );
  });

  it("renders the compact popup in the popup window", async () => {
    windowLabel.value = "popup";
    render(<App />);
    expect(await screen.findByTestId("row-claude")).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "AI Usage Monitor" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Open dashboard" }));
    expect(api.showDashboard).toHaveBeenCalledTimes(1);
  });
});

describe("mergeSnapshot", () => {
  it("replaces the provider's entry and keeps the canonical order", () => {
    const [claude, codex, , copilot] = demoSnapshots();
    const merged = mergeSnapshot([copilot, claude], { ...codex, plan: "plus" });
    expect(merged.map((s) => s.provider)).toEqual(["claude", "codex", "copilot"]);

    const replaced = mergeSnapshot(merged, { ...codex, plan: "team" });
    expect(replaced).toHaveLength(3);
    expect(replaced[1].plan).toBe("team");
  });
});
