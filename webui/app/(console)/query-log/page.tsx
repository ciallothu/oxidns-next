"use client";

import { useEffect, useMemo, useState } from "react";
import { BarChart3, Database, List } from "lucide-react";
import { AppHeader } from "@/components/shell/app-header";
import {
  QueryRecorderInsightsPanel,
  QueryRecordsPanel,
} from "@/components/plugins/kinds/query-recorder";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useAppStore } from "@/lib/store";
import { WEBUI } from "@/lib/i18n";
import { useI18n } from "@/lib/i18n/provider";

export default function QueryLogPage() {
  const { t } = useI18n();
  const plugins = useAppStore((state) => state.plugins);
  const recorders = useMemo(
    () => plugins.filter((plugin) => plugin.pluginKind === "query_recorder"),
    [plugins],
  );
  const [selectedTag, setSelectedTag] = useState("");

  useEffect(() => {
    if (recorders.some((recorder) => recorder.name === selectedTag)) return;
    setSelectedTag(recorders[0]?.name ?? "");
  }, [recorders, selectedTag]);

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <AppHeader title={t(WEBUI.queryRecorder.pageTitle)} />
      <main className="oxidns-next-dialog-scrollbar min-h-0 flex-1 overflow-auto p-6">
        {recorders.length === 0 ? (
          <div className="flex min-h-[360px] flex-col items-center justify-center rounded-lg border border-dashed p-8 text-center">
            <div className="mb-4 rounded-full bg-muted p-3">
              <Database className="h-6 w-6 text-muted-foreground" />
            </div>
            <h2 className="text-base font-semibold">
              {t(WEBUI.queryRecorder.noRecorderTitle)}
            </h2>
            <p className="mt-2 max-w-xl text-sm text-muted-foreground">
              {t(WEBUI.queryRecorder.noRecorderDescription)}
            </p>
          </div>
        ) : (
          <div className="space-y-4">
            {recorders.length > 1 && (
              <div className="flex items-center justify-end gap-2">
                <span className="text-sm text-muted-foreground">
                  {t(WEBUI.queryRecorder.sourceLabel)}
                </span>
                <Select value={selectedTag} onValueChange={setSelectedTag}>
                  <SelectTrigger className="w-56 font-mono">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {recorders.map((recorder) => (
                      <SelectItem key={recorder.id} value={recorder.name}>
                        {recorder.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            )}

            {selectedTag && (
              <Tabs defaultValue="records" className="space-y-4">
                <TabsList>
                  <TabsTrigger value="records" className="gap-1.5">
                    <List className="h-3.5 w-3.5" />
                    {t(WEBUI.queryRecorder.recordsTab)}
                  </TabsTrigger>
                  <TabsTrigger value="insights" className="gap-1.5">
                    <BarChart3 className="h-3.5 w-3.5" />
                    {t(WEBUI.queryRecorder.insightsTab)}
                  </TabsTrigger>
                </TabsList>
                <TabsContent value="records" className="m-0">
                  <QueryRecordsPanel key={selectedTag} tag={selectedTag} />
                </TabsContent>
                <TabsContent value="insights" className="m-0">
                  <QueryRecorderInsightsPanel
                    key={selectedTag}
                    tag={selectedTag}
                  />
                </TabsContent>
              </Tabs>
            )}
          </div>
        )}
      </main>
    </div>
  );
}
