"use client";

import { useEffect } from "react";
import { AppHeader } from "@/components/shell/app-header";
import { WEBUI } from "@/lib/i18n";
import { useI18n } from "@/lib/i18n/provider";

/** Compatibility route for bookmarks created before Plugin Center moved home. */
export default function PluginsCompatibilityPage() {
  const { t } = useI18n();

  useEffect(() => {
    window.location.replace("/#plugins");
  }, []);

  return (
    <>
      <AppHeader title={t(WEBUI.plugins.centerTitle)} />
      <main className="flex min-h-0 flex-1 items-center justify-center p-6 text-sm text-muted-foreground">
        {t(WEBUI.common.loading)}
      </main>
    </>
  );
}
