import type { QueryRecorderTimeseriesBucket } from "./oxidns-next-api";

export const INSIGHTS_RANGES = ["1h", "24h", "7d", "30d", "1y"] as const;
export type InsightsRange = (typeof INSIGHTS_RANGES)[number];

export function queryRecorderRange(range: InsightsRange, now: number) {
  const untilMs = now;
  const buckets = { "1h": 60, "24h": 24, "7d": 7, "30d": 30, "1y": 12 }[range];
  const bucket: QueryRecorderTimeseriesBucket =
    range === "1h"
      ? "minute"
      : range === "24h"
        ? "hour"
        : range === "1y"
          ? "month"
          : "day";
  let sinceMs: number;
  if (bucket === "month") {
    const end = new Date(now);
    sinceMs = Date.UTC(
      end.getUTCFullYear(),
      end.getUTCMonth() - buckets + 1,
      1,
    );
  } else {
    const duration =
      bucket === "minute" ? 60_000 : bucket === "hour" ? 3_600_000 : 86_400_000;
    sinceMs = Math.floor(now / duration) * duration - (buckets - 1) * duration;
  }
  return { sinceMs, untilMs, bucket, buckets };
}
