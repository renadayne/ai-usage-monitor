import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { demoSnapshots } from "../lib/demo";
import type { ProviderSnapshot } from "../lib/types";
import { PopupRow } from "../Popup";
import { ProviderCard } from "./ProviderCard";

vi.mock("../lib/api", () => ({
  getSnapshots: vi.fn(() => Promise.resolve([])),
  onSnapshotUpdated: vi.fn(() => Promise.resolve(() => {})),
  refresh: vi.fn(() => Promise.resolve()),
  showDashboard: vi.fn(() => Promise.resolve()),
}));

const NOW = Date.now();
const [claude, codex, antigravity, copilot] = demoSnapshots();

function renderCard(snapshot: ProviderSnapshot, onRefresh = vi.fn()) {
  render(<ProviderCard snapshot={snapshot} now={NOW} onRefresh={onRefresh} />);
  return onRefresh;
}

describe("ProviderCard", () => {
  it("shows plan, model, activity and every window", () => {
    renderCard(claude);
    const card = screen.getByRole("region", { name: "Claude" });
    expect(within(card).getByText("max")).toBeInTheDocument();
    expect(within(card).getByText("Model: claude-fable-5-2")).toBeInTheDocument();
    expect(within(card).getByText("Active")).toBeInTheDocument();
    expect(within(card).getAllByRole("progressbar")).toHaveLength(3);
    expect(within(card).getByText(/Updated 3 min ago/)).toBeInTheDocument();
    expect(within(card).getAllByText(/resets in/)).toHaveLength(3);
  });

  it("marks severity with text, not only colour", () => {
    renderCard(claude);
    expect(screen.getByTestId("window-five_hour")).toHaveAttribute("data-severity", "normal");
    const warning = screen.getByTestId("window-weekly");
    expect(warning).toHaveAttribute("data-severity", "warning");
    expect(within(warning).getByText(/High/)).toBeInTheDocument();
    const danger = screen.getByTestId("window-weekly:opus");
    expect(danger).toHaveAttribute("data-severity", "danger");
    expect(within(danger).getByText(/Critical/)).toBeInTheDocument();
    expect(within(danger).getByRole("progressbar")).toHaveAttribute("aria-valuenow", "94");
  });

  it("explains rate limiting and flags estimated windows", () => {
    renderCard(codex);
    expect(screen.getByText("Rate limited")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent(/Retrying in [78]m/);
    expect(screen.getByText("(estimated)")).toBeInTheDocument();
    expect(screen.getByText("Idle")).toBeInTheDocument();
  });

  it("keeps old numbers visible when stale", () => {
    renderCard(antigravity);
    expect(screen.getByText("Stale")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Showing data from 5 h ago.");
    expect(screen.getAllByRole("progressbar")).toHaveLength(2);
    expect(screen.getByText("Not running")).toBeInTheDocument();
    expect(screen.getByText("Model: unknown")).toBeInTheDocument();
    expect(screen.getByText(/Antigravity is not running/)).toBeInTheDocument();
  });

  it("shows the sign-in hint when authentication is required", () => {
    renderCard(copilot);
    expect(screen.getByText("Sign-in required")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent(/No GitHub token found/);
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
    expect(screen.queryByText(/Updated/)).not.toBeInTheDocument();
  });

  it("covers not-installed, error and empty states", () => {
    const { rerender } = render(
      <ProviderCard snapshot={{ ...copilot, health: { kind: "not_installed" } }} now={NOW} />,
    );
    expect(screen.getByText("Not installed")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("This tool was not found");

    rerender(
      <ProviderCard
        snapshot={{ ...copilot, health: { kind: "error", message: "could not connect" } }}
        now={NOW}
      />,
    );
    expect(screen.getByText("Error")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("could not connect");

    rerender(<ProviderCard snapshot={{ ...copilot, health: { kind: "ok" } }} now={NOW} />);
    expect(screen.getByText("No quota data yet.")).toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it("shows absolute amounts when the provider reports them", () => {
    const snapshot: ProviderSnapshot = {
      ...copilot,
      health: { kind: "ok" },
      windows: [
        {
          id: "monthly",
          label: "AI Credits",
          used_percent: 57.5,
          remaining: 425,
          limit: 1000,
          unit: "credits",
          resets_at: null,
          estimated: false,
        },
      ],
    };
    renderCard(snapshot);
    expect(screen.getByText("425 / 1,000 credits left")).toBeInTheDocument();
    expect(screen.getByText("58%")).toBeInTheDocument();
  });

  it("refreshes only its own provider", () => {
    const onRefresh = renderCard(claude);
    fireEvent.click(screen.getByRole("button", { name: "Refresh Claude" }));
    expect(onRefresh).toHaveBeenCalledTimes(1);
  });
});

describe("PopupRow", () => {
  it("shows the tightest window", () => {
    render(
      <ul>
        <PopupRow snapshot={claude} />
      </ul>,
    );
    const row = screen.getByTestId("row-claude");
    expect(within(row).getByText("Weekly · Opus")).toBeInTheDocument();
    expect(row).toHaveTextContent("94%");
    expect(within(row).getByLabelText("Active")).toBeInTheDocument();
  });

  it("shows the problem instead of a window when unhealthy", () => {
    render(
      <ul>
        <PopupRow snapshot={copilot} />
      </ul>,
    );
    const row = screen.getByTestId("row-copilot");
    expect(within(row).getByText("Sign-in required")).toBeInTheDocument();
    expect(row).toHaveTextContent("–");
  });
});
