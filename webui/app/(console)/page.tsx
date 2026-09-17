"use client";

import { AppHeader } from "@/components/shell/app-header";
import { SystemMetrics } from "@/components/dashboard/system-metrics";
import { PluginWorkspace } from "@/components/plugins/plugin-workspace";
import { useAppStore } from "@/lib/store";
import { WEBUI } from "@/lib/i18n";
import { useI18n } from "@/lib/i18n/provider";
import { useAuthStore } from "@/lib/auth-store";
import { useVisiblePolling } from "@/hooks/use-visible-polling";
import { DASHBOARD_HEALTH_POLL_INTERVAL_MS, DASHBOARD_SYSTEM_POLL_INTERVAL_MS } from "@/lib/polling-policy";

export default function DashboardPage() {
  const { t } = useI18n();
  const plugins = useAppStore((s) => s.plugins);
  const refreshHealthState = useAppStore((s) => s.refreshHealthState);
  const refreshSystemState = useAppStore((s) => s.refreshSystemState);
  const isConnected = useAuthStore((s) => s.isConnected);
  const sessionGeneration = useAuthStore((s) => s.sessionGeneration);
  useVisiblePolling(
    refreshSystemState,
    DASHBOARD_SYSTEM_POLL_INTERVAL_MS,
    isConnected,
    sessionGeneration,
  );
  useVisiblePolling(
    refreshHealthState,
    DASHBOARD_HEALTH_POLL_INTERVAL_MS,
    isConnected,
    sessionGeneration,
  );

  return (
    <>
      <AppHeader title={t(WEBUI.shell.dashboard)} />
      <main className="oxidns-next-dialog-scrollbar min-h-0 flex-1 overflow-auto p-6">
        <div className="space-y-8">
          <section>
            <h2 className="mb-4 text-lg font-semibold">
              {t(WEBUI.plugins.systemOverview)}
            </h2>
            <SystemMetrics />
          </section>

          <section id="plugins" className="scroll-mt-4 space-y-4">
            <h2 className="text-lg font-semibold">
              {t(WEBUI.plugins.centerTitle)}
              <span className="ml-2 text-sm font-normal text-muted-foreground">
                ({plugins.length})
              </span>
            </h2>
            <PluginWorkspace />
          </section>
        </div>
      </main>
    </>
  );
}
