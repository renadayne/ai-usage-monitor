import { describe, expect, it } from "vitest";
import { demoSnapshots } from "./demo";
import {
  formatAgo,
  formatAmount,
  formatCountdown,
  formatDuration,
  formatPercent,
  severity,
  tightestWindow,
} from "./format";

const NOW = Date.parse("2026-11-15T12:00:00Z");

describe("severity", () => {
  it("uses the 70 % and 90 % thresholds", () => {
    expect(severity(null)).toBe("normal");
    expect(severity(69.9)).toBe("normal");
    expect(severity(70)).toBe("warning");
    expect(severity(90)).toBe("warning");
    expect(severity(90.1)).toBe("danger");
  });
});

describe("formatting", () => {
  it("formats percentages and amounts", () => {
    expect(formatPercent(null)).toBe("–");
    expect(formatPercent(37.4)).toBe("37%");
    expect(formatAmount(1250)).toBe("1,250");
    expect(formatAmount(183.94)).toBe("183.9");
  });

  it("formats durations", () => {
    expect(formatDuration(20_000)).toBe("<1m");
    expect(formatDuration(12 * 60_000)).toBe("12m");
    expect(formatDuration(4 * 3_600_000 + 12 * 60_000)).toBe("4h 12m");
    expect(formatDuration(3 * 3_600_000)).toBe("3h");
    expect(formatDuration(51 * 3_600_000)).toBe("2d 3h");
    expect(formatDuration(48 * 3_600_000)).toBe("2d");
  });

  it("formats countdowns", () => {
    expect(formatCountdown(null, NOW)).toBeNull();
    expect(formatCountdown("garbage", NOW)).toBeNull();
    expect(formatCountdown("2026-11-15T14:30:00Z", NOW)).toBe("resets in 2h 30m");
    expect(formatCountdown("2026-11-15T11:59:00Z", NOW)).toBe("resetting…");
  });

  it("formats relative times", () => {
    expect(formatAgo(null, NOW)).toBeNull();
    expect(formatAgo("2026-11-15T11:59:40Z", NOW)).toBe("just now");
    expect(formatAgo("2026-11-15T11:57:00Z", NOW)).toBe("3 min ago");
    expect(formatAgo("2026-11-15T07:00:00Z", NOW)).toBe("5 h ago");
    expect(formatAgo("2026-11-12T12:00:00Z", NOW)).toBe("3 d ago");
  });
});

describe("tightestWindow", () => {
  it("picks the window with the highest usage", () => {
    const [claude, , , copilot] = demoSnapshots();
    expect(tightestWindow(claude)?.id).toBe("weekly:opus");
    expect(tightestWindow(copilot)).toBeNull();
  });
});
