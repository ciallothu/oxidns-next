import { describe, expect, it } from "vitest";
import { INSIGHTS_RANGES, queryRecorderRange } from "./query-recorder-ranges";
import { timeseriesOption } from "./analytics-options";

describe("query recorder history windows", () => {
  const now = Date.parse("2024-02-29T12:34:56Z");
  it("covers every preset with bounded, aligned buckets", () => {
    expect(
      INSIGHTS_RANGES.map((range) => queryRecorderRange(range, now).buckets),
    ).toEqual([60, 24, 7, 30, 12]);
    expect(queryRecorderRange("24h", now)).toEqual({
      sinceMs: Date.parse("2024-02-28T13:00:00Z"),
      untilMs: now,
      bucket: "hour",
      buckets: 24,
    });
    expect(queryRecorderRange("7d", now).sinceMs).toBe(
      Date.parse("2024-02-23T00:00:00Z"),
    );
    expect(queryRecorderRange("1y", now).sinceMs).toBe(
      Date.parse("2023-03-01T00:00:00Z"),
    );
  });
  it("advances the time window on refresh", () => {
    expect(queryRecorderRange("24h", now + 3_600_000).sinceMs).toBe(
      queryRecorderRange("24h", now).sinceMs + 3_600_000,
    );
  });
  it("preserves absolute timestamps, unsmoothed spikes, and missing latency", () => {
    const points = [
      {
        bucket_ms: 1_000,
        total: 10,
        error_count: 1,
        no_response_count: 0,
        avg_ms: 5,
        p95_ms: 90,
      },
      {
        bucket_ms: 86_401_000,
        total: 0,
        error_count: 0,
        no_response_count: 0,
        avg_ms: 0,
        p95_ms: 0,
      },
    ];
    const option = timeseriesOption(
      points,
      {
        total: "Queries",
        errors: "Errors",
        missing: "No response",
        average: "Average",
      },
      "en-US",
    );
    if (!Array.isArray(option.series) || !Array.isArray(option.dataZoom)) {
      throw new Error("Expected series and zoom arrays");
    }
    expect(option.series[0].data).toEqual([
      [1_000, 10],
      [86_401_000, 0],
    ]);
    expect(option.series[4].data).toEqual([
      [1_000, 90],
      [86_401_000, null],
    ]);
    expect(option.series[4].smooth).toBe(false);
    expect(option.dataZoom[0]).toMatchObject({
      filterMode: "none",
      zoomOnMouseWheel: "ctrl",
      moveOnMouseWheel: false,
    });
  });
});
