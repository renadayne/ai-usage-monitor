import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { HistoryChart } from "./History";
import * as api from "./lib/api";
import { buildSeries } from "./lib/chart";
import { demoSettings } from "./lib/demo";
import type { CopilotAuthEvent, HistoryPoint } from "./lib/types";
import Settings from "./Settings";

const authListeners = vi.hoisted(() => [] as ((event: unknown) => void)[]);

vi.mock("./lib/api", () => ({
  saveSettings: vi.fn((settings: unknown) => Promise.resolve(settings)),
  copilotStatus: vi.fn(() => Promise.resolve({ connected: false })),
  copilotConnect: vi.fn(() =>
    Promise.resolve({
      user_code: "ABCD-1234",
      verification_uri: "https://github.com/login/device",
      expires_in: 900,
      interval: 5,
    }),
  ),
  copilotOpenVerification: vi.fn(() => Promise.resolve()),
  copilotDisconnect: vi.fn(() => Promise.resolve()),
  onCopilotAuth: vi.fn((handler: (event: unknown) => void) => {
    authListeners.push(handler);
    return Promise.resolve(() => {});
  }),
}));

beforeEach(() => {
  vi.clearAllMocks();
  authListeners.length = 0;
});

function emitAuth(event: CopilotAuthEvent) {
  for (const listener of authListeners) listener(event);
}

describe("Settings", () => {
  it("saves edited values and reports success", async () => {
    const onSaved = vi.fn();
    render(<Settings settings={demoSettings()} onSaved={onSaved} />);
    const save = screen.getByRole("button", { name: "Save settings" });
    expect(save).toBeDisabled();

    const providers = screen.getByRole("region", { name: "Providers" });
    fireEvent.click(within(providers).getByRole("checkbox", { name: "Codex" }));
    fireEvent.change(screen.getByLabelText("Claude refresh interval in seconds"), {
      target: { value: "900" },
    });
    fireEvent.change(screen.getByLabelText("Warning threshold percent"), {
      target: { value: "70" },
    });
    fireEvent.click(screen.getByRole("checkbox", { name: /Start with Windows/ }));
    fireEvent.change(screen.getByLabelText("Claude Code folder"), {
      target: { value: "D:\\tools\\.claude" },
    });
    expect(save).toBeEnabled();
    fireEvent.click(save);

    await waitFor(() => expect(api.saveSettings).toHaveBeenCalledTimes(1));
    const sent = vi.mocked(api.saveSettings).mock.calls[0][0];
    expect(sent.providers.codex.enabled).toBe(false);
    expect(sent.providers.claude.poll_interval_secs).toBe(900);
    expect(sent.warn_threshold).toBe(70);
    expect(sent.autostart).toBe(true);
    expect(sent.claude_dir).toBe("D:\\tools\\.claude");
    expect(await screen.findByText("Saved. Changes are active now.")).toBeInTheDocument();
    expect(onSaved).toHaveBeenCalledWith(sent);
  });

  it("shows the interval floor and clears to the default", () => {
    render(<Settings settings={demoSettings()} onSaved={vi.fn()} />);
    const interval = screen.getByLabelText("Antigravity refresh interval in seconds");
    expect(interval).toHaveValue(120);
    expect(interval).toHaveAttribute("min", "30");
    fireEvent.change(interval, { target: { value: "" } });
    expect(interval).toHaveValue(null);
    expect(screen.getByLabelText("Claude refresh interval in seconds")).toHaveAttribute(
      "min",
      "300",
    );
  });

  it("reports a failed save", async () => {
    vi.mocked(api.saveSettings).mockRejectedValueOnce("disk full");
    render(<Settings settings={demoSettings()} onSaved={vi.fn()} />);
    fireEvent.click(screen.getByRole("checkbox", { name: /Notify when/ }));
    fireEvent.click(screen.getByRole("button", { name: "Save settings" }));
    expect(await screen.findByText("Could not save: disk full")).toBeInTheDocument();
  });

  it("walks through the GitHub device flow", async () => {
    render(<Settings settings={demoSettings()} onSaved={vi.fn()} />);
    const section = screen.getByRole("region", { name: "GitHub Copilot connection" });
    expect(within(section).getByText("Not connected")).toBeInTheDocument();

    fireEvent.click(within(section).getByRole("button", { name: "Connect GitHub" }));
    expect(await within(section).findByText("ABCD-1234")).toBeInTheDocument();
    fireEvent.click(within(section).getByRole("button", { name: /Open github.com/ }));
    expect(api.copilotOpenVerification).toHaveBeenCalledTimes(1);

    await waitFor(() => expect(authListeners.length).toBeGreaterThan(0));
    emitAuth({ state: "connected", message: null });
    expect(await within(section).findByText("Connected")).toBeInTheDocument();
    expect(within(section).queryByText("ABCD-1234")).not.toBeInTheDocument();

    fireEvent.click(within(section).getByRole("button", { name: "Disconnect" }));
    expect(await within(section).findByText("Not connected")).toBeInTheDocument();
    expect(api.copilotDisconnect).toHaveBeenCalledTimes(1);
  });

  it("shows why connecting failed", async () => {
    render(<Settings settings={demoSettings()} onSaved={vi.fn()} />);
    await waitFor(() => expect(authListeners.length).toBeGreaterThan(0));
    emitAuth({ state: "failed", message: "The code expired. Try again." });
    expect(await screen.findByText("The code expired. Try again.")).toBeInTheDocument();
  });
});

const NOW = Date.parse("2026-11-15T12:00:00Z");
const DAY = 86_400_000;

function point(window_id: string, used_percent: number, daysAgo: number): HistoryPoint {
  return {
    window_id,
    label: window_id === "weekly" ? "Weekly" : "5 hours",
    used_percent,
    recorded_at: new Date(NOW - daysAgo * DAY).toISOString(),
  };
}

describe("buildSeries", () => {
  const box = { width: 700, height: 100, from: NOW - 7 * DAY, to: NOW };

  it("groups by window, sorts by time and projects into the box", () => {
    const series = buildSeries(
      [point("weekly", 50, 0), point("weekly", 0, 7), point("five_hour", 150, 3.5)],
      box,
    );
    expect(series.map((s) => s.id)).toEqual(["weekly", "five_hour"]);
    expect(series[0].points).toBe("0.0,100.0 700.0,50.0");
    expect(series[0].last).toBe(50);
    expect(series[0].count).toBe(2);
    // Values are clamped to 0–100 %.
    expect(series[1].points).toBe("350.0,0.0");
  });

  it("ignores points outside the range or with bad dates", () => {
    const bad = { ...point("weekly", 10, 1), recorded_at: "garbage" };
    expect(buildSeries([point("weekly", 10, 9), bad], box)).toEqual([]);
  });
});

describe("HistoryChart", () => {
  it("draws one line per window with a legend", () => {
    render(
      <HistoryChart
        provider="claude"
        now={NOW}
        history={[point("weekly", 20, 2), point("weekly", 40, 1), point("five_hour", 90, 1)]}
      />,
    );
    expect(
      screen.getByRole("img", { name: /Claude usage over the last 7 days/ }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("series-weekly")).toBeInTheDocument();
    expect(screen.getByTestId("series-five_hour")).toBeInTheDocument();
    expect(screen.getByText("Weekly")).toBeInTheDocument();
    expect(screen.getByText("40%")).toBeInTheDocument();
  });

  it("explains an empty history", () => {
    render(<HistoryChart provider="copilot" now={NOW} history={[]} />);
    expect(screen.getByText(/No history yet/)).toBeInTheDocument();
  });
});
