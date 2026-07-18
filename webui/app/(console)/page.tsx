"use client";

import { useEffect } from "react";
import { SystemMetrics } from "@/components/dashboard/system-metrics";
import { PluginWorkspace } from "@/components/plugins/plugin-workspace";
import { AppHeader } from "@/components/shell/app-header";
import { WEBUI } from "@/lib/i18n";
import { useI18n } from "@/lib/i18n/provider";
import { useAppStore } from "@/lib/store";

export default function DashboardPage() {
  const { t } = useI18n();
  const plugins = useAppStore((state) => state.plugins);
  const refreshRuntimeState = useAppStore(
    (state) => state.refreshRuntimeState,
  );

  useEffect(() => {
    const id = setInterval(() => {
      void refreshRuntimeState();
    }, 3_000);
    return () => clearInterval(id);
  }, [refreshRuntimeState]);

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
