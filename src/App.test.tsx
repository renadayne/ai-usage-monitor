import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import App from "./App";
import { mergeSnapshot } from "./lib/useSnapshots";
import type { ProviderSnapshot } from "./lib/types";

function snapshot(overrides: Partial<ProviderSnapshot> = {}): ProviderSnapshot {
  return {
    provider: "claude",
    activity: "active",
    health: { kind: "ok" },
    plan: "pro",
    current_model: "model-x",
    last_activity_at: null,
    windows: [],
    note: null,
    fetched_at: "2026-11-14T00:00:00Z",
    ...overrides,
  };
}

vi.mock("./lib/api", () => ({
  getSnapshots: vi.fn(() => Promise.resolve([snapshot()])),
  onSnapshotUpdated: vi.fn(() => Promise.resolve(() => {})),
  refresh: vi.fn(() => Promise.resolve()),
}));

describe("App", () => {
  it("renders the title and the loaded snapshots", async () => {
    render(<App />);
    expect(screen.getByRole("heading", { name: "AI Usage Monitor" })).toBeInTheDocument();
    expect(await screen.findByText(/"provider": "claude"/)).toBeInTheDocument();
  });
});

describe("mergeSnapshot", () => {
  it("replaces the provider's entry and keeps the canonical order", () => {
    const initial = [snapshot({ provider: "copilot" }), snapshot({ provider: "claude" })];
    const merged = mergeSnapshot(initial, snapshot({ provider: "codex", plan: "plus" }));
    expect(merged.map((s) => s.provider)).toEqual(["claude", "codex", "copilot"]);

    const replaced = mergeSnapshot(merged, snapshot({ provider: "codex", plan: "team" }));
    expect(replaced).toHaveLength(3);
    expect(replaced[1].plan).toBe("team");
  });
});
