"use client";

import { useEffect, useState } from "react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";
import { cn } from "@/lib/utils";
import { Cpu, HardDrive, HeartPulse, Puzzle } from "lucide-react";
import { useAppStore } from "@/lib/store";
import { WEBUI } from "@/lib/i18n";
import type { TranslationParams } from "@/lib/i18n";
import { useI18n } from "@/lib/i18n/provider";

// Ticks locally every second; re-calibrates whenever backendUptimeMs changes.
function useLocalUptime(backendUptimeMs: number): number {
  const [syncedUptime, setSyncedUptime] = useState(backendUptimeMs);
  const [elapsedMs, setElapsedMs] = useState(0);

  // Adjust state during render when the backend reports a new uptime, so the
  // displayed value snaps to it immediately (no setState-in-effect needed).
  if (syncedUptime !== backendUptimeMs) {
    setSyncedUptime(backendUptimeMs);
    setElapsedMs(0);
  }

  // Restart the wall-clock ticker on each backend sync; reading the clock
  // inside an effect/interval keeps render pure.
  useEffect(() => {
    const syncedAt = Date.now();
    const id = setInterval(() => setElapsedMs(Date.now() - syncedAt), 1000);
    return () => clearInterval(id);
  }, [backendUptimeMs]);

  return syncedUptime + elapsedMs;
}

type TFn = (key: string, params?: TranslationParams) => string;

function formatUptime(ms: number, t: TFn): string {
  const s = Math.floor(ms / 1000);
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (d > 0) return t(WEBUI.dashboard.uptimeDhm, { d, h, m });
  if (h > 0) return t(WEBUI.dashboard.uptimeHm, { h, m });
  if (m > 0) return t(WEBUI.dashboard.uptimeMs, { m, s: sec });
  return t(WEBUI.dashboard.uptimeS, { s: sec });
}

function formatMemory(mb: number): string {
  if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`;
  return `${mb} MB`;
}

function usageColor(pct: number) {
  if (pct >= 85) return "text-destructive";
  if (pct >= 60) return "text-amber-500";
  return "";
}

function usageBarColor(pct: number) {
  if (pct >= 85) return "bg-destructive";
  if (pct >= 60) return "bg-amber-500";
  return undefined;
}

function CheckDot({ status }: { status?: string }) {
  const ok = status === "ok";
  return (
    <span
      className={cn(
        "inline-block h-1.5 w-1.5 rounded-full",
        ok ? "bg-green-500" : "bg-destructive",
      )}
    />
  );
}

export function SystemMetrics() {
  const { t } = useI18n();
  const health = useAppStore((s) => s.health);
  const system = useAppStore((s) => s.system);
  const plugins = useAppStore((s) => s.plugins);
  const configPath = useAppStore((s) => s.configPath);
  const configError = useAppStore((s) => s.configError);

  const uptimeMs = useLocalUptime(system?.uptime_ms ?? health?.uptime_ms ?? 0);
  const cpuPct = system?.process_cpu_percent ?? 0;
  const memMb = system?.process_memory_mb ?? 0;
  const totalMemMb = system?.system_memory_total_mb ?? 0;
  const memPct = totalMemMb > 0 ? Math.min((memMb / totalMemMb) * 100, 100) : 0;

  const healthStatus = health?.status ?? "unknown";
  const isHealthy = healthStatus === "ok";

  const serverCount = health?.plugins.servers;
  const pluginTotal = health?.plugins.total ?? plugins.length;

  return (
    <div className="grid gap-4 grid-cols-2 lg:grid-cols-4">
      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <CardTitle className="text-sm font-medium">
            {t(WEBUI.dashboard.serviceHealth)}
          </CardTitle>
          <HeartPulse className="h-4 w-4 text-muted-foreground" />
        </CardHeader>
        <CardContent className="space-y-2">
          <div>
            <div
              className={cn(
                "text-2xl font-bold font-mono",
                isHealthy ? "text-green-500" : "text-destructive",
              )}
            >
              {healthStatus}
            </div>
            <p className="text-xs text-muted-foreground mt-0.5">
              {uptimeMs > 0
                ? t(WEBUI.dashboard.running, {
                    duration: formatUptime(uptimeMs, t),
                  })
                : t(WEBUI.dashboard.waitingData)}
            </p>
          </div>
          {health && (
            <div className="border-t border-border/50 pt-2 flex items-center gap-3 text-xs text-muted-foreground">
              <span className="flex items-center gap-1">
                <CheckDot status={health.checks.api} />
                API
              </span>
              <span className="flex items-center gap-1">
                <CheckDot status={health.checks.plugin_init} />
                {t(WEBUI.dashboard.checkPlugin)}
              </span>
              <span className="flex items-center gap-1">
                <CheckDot status={health.checks.server_startup} />
                {t(WEBUI.dashboard.checkServer)}
              </span>
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <CardTitle className="text-sm font-medium">
            {t(WEBUI.dashboard.cpuUsage)}
          </CardTitle>
          <Cpu className="h-4 w-4 text-muted-foreground" />
        </CardHeader>
        <CardContent className="space-y-2">
          <div>
            <div
              className={cn("text-2xl font-bold font-mono", usageColor(cpuPct))}
            >
              {system ? `${cpuPct.toFixed(1)}%` : "-"}
            </div>
            <p className="text-xs text-muted-foreground mt-0.5">
              {t(WEBUI.dashboard.cpuUsageDesc)}
            </p>
          </div>
          <Progress
            value={cpuPct}
            className="h-1.5"
            indicatorClassName={usageBarColor(cpuPct)}
          />
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <CardTitle className="text-sm font-medium">
            {t(WEBUI.dashboard.memUsage)}
          </CardTitle>
          <HardDrive className="h-4 w-4 text-muted-foreground" />
        </CardHeader>
        <CardContent className="space-y-2">
          <div>
            <div
              className={cn("text-2xl font-bold font-mono", usageColor(memPct))}
            >
              {system ? formatMemory(memMb) : "-"}
            </div>
            <p className="text-xs text-muted-foreground mt-0.5">
              {totalMemMb > 0
                ? t(WEBUI.dashboard.memTotal, {
                    total: formatMemory(totalMemMb),
                    pct: memPct.toFixed(1),
                  })
                : t(WEBUI.dashboard.processRss)}
            </p>
          </div>
          <Progress
            value={memPct}
            className="h-1.5"
            indicatorClassName={usageBarColor(memPct)}
          />
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <CardTitle className="text-sm font-medium">
            {t(WEBUI.dashboard.pluginTotal)}
          </CardTitle>
          <Puzzle className="h-4 w-4 text-muted-foreground" />
        </CardHeader>
        <CardContent className="space-y-2">
          <div>
            <div className="text-2xl font-bold font-mono">{pluginTotal}</div>
            <p
              className="text-xs mt-0.5 truncate text-muted-foreground font-mono"
              title={configPath}
            >
              {configPath}
            </p>
          </div>
          <div className="border-t border-border/50 pt-2 flex items-center gap-3 text-xs text-muted-foreground">
            {serverCount !== undefined && (
              <span>
                {t(WEBUI.dashboard.serverCount, { count: serverCount })}
              </span>
            )}
            <span
              className={cn(
                configError ? "text-destructive" : "text-green-500",
              )}
            >
              {configError
                ? t(WEBUI.dashboard.configError)
                : t(WEBUI.dashboard.configOk)}
            </span>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
