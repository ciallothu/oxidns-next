export const DASHBOARD_SYSTEM_POLL_INTERVAL_MS = 5_000;
export const DASHBOARD_HEALTH_POLL_INTERVAL_MS = 15_000;
export const ACTIVE_METRICS_POLL_INTERVAL_MS = 5_000;
export const SETTINGS_METRICS_POLL_INTERVAL_MS = 15_000;

/** Return null for routes that do not display Prometheus-backed metrics. */
export function metricsPollingInterval(
  pathname: string,
  editorMode: boolean,
): number | null {
  if (editorMode) return null;
  if (pathname === "/" || pathname.startsWith("/plugins")) {
    return ACTIVE_METRICS_POLL_INTERVAL_MS;
  }
  if (pathname.startsWith("/settings")) {
    return SETTINGS_METRICS_POLL_INTERVAL_MS;
  }
  return null;
}
