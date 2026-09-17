"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import {
  BarChart3,
  Filter,
  Globe,
  Info,
  Loader2,
  PieChart as PieChartIcon,
  Radio,
  RefreshCw,
  ServerCrash,
  Timer,
  Trash2,
  TrendingUp,
  Users,
  X,
} from "lucide-react";
import { AnalyticsChart } from "@/components/ui/analytics-chart";
import { distributionOption, timeseriesOption } from "@/lib/analytics-options";
import {
  INSIGHTS_RANGES,
  queryRecorderRange,
  type InsightsRange,
} from "@/lib/query-recorder-ranges";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogMedia,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { DateTimePicker } from "@/components/ui/datetime-picker";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { apiRequest, readApiResponseBody } from "@/lib/api-client";
import {
  apiHeaders,
  apiUrl,
  clearQueryRecorderHistory,
  fetchQueryRecordDetail,
  fetchQueryRecorderLatency,
  fetchQueryRecorderPluginStats,
  fetchQueryRecorderQtypeDistribution,
  fetchQueryRecorderRcodeDistribution,
  fetchQueryRecorderTimeseries,
  fetchQueryRecorderTopClients,
  fetchQueryRecorderTopQnames,
  fetchQueryRecords,
  type QueryRecordDetail,
  type QueryRecordFilters,
  type QueryRecordRow,
  type QueryRecordStatusFilter,
  type QueryRecorderDistributionResponse,
  type QueryRecorderLatencySummary,
  type QueryRecorderPluginStatsRow,
  type QueryRecorderTimeseriesResponse,
  type QueryRecorderTopResponse,
} from "@/lib/oxidns-next-api";
import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";
import type {
  PluginComponentDefinition,
  PluginDetailComponentProps,
} from "../types";
import {
  PluginDetailTemplate,
  PluginNotAppliedPlaceholder,
} from "../plugin-detail-template";
import { usePluginAppliedStatus } from "@/hooks/use-plugin-applied";
import { useI18n } from "@/lib/i18n/provider";
import { WEBUI } from "@/lib/i18n/keys";
import { DnsRecordDetailDialog } from "../dns-record-detail-dialog";
import { QueryRecordFlowCanvas } from "../query-record-flow";

function QueryRecorderDetail(props: PluginDetailComponentProps) {
  const { t } = useI18n();
  const rawDatabase = props.plugin.config.database;
  const database =
    rawDatabase &&
    typeof rawDatabase === "object" &&
    !Array.isArray(rawDatabase)
      ? (rawDatabase as Record<string, unknown>)
      : undefined;
  const databaseType = String(database?.type ?? "sqlite").toLowerCase();
  const storageSummary =
    databaseType === "postgres" || databaseType === "postgresql"
      ? { label: "PostgreSQL", value: t(WEBUI.common.configured) }
      : databaseType === "mysql"
        ? { label: "MySQL", value: t(WEBUI.common.configured) }
        : {
            label: "SQLite",
            value: String(database?.path ?? props.plugin.config.path ?? "-"),
          };
  return (
    <PluginDetailTemplate
      {...props}
      icon={<Radio className="h-5 w-5" />}
      summaryItems={[
        storageSummary,
        {
          label: t(WEBUI.queryRecorder.memoryTailLabel),
          value: String(
            props.plugin.config.memory_tail ?? t(WEBUI.common.defaultValue),
          ),
        },
        {
          label: t(WEBUI.queryRecorder.retentionLabel),
          value: t(WEBUI.queryRecorder.retentionDays, {
            days: String(
              props.plugin.config.retention_days ??
                t(WEBUI.common.defaultValue),
            ),
          }),
        },
      ]}
      metricsContent={<QueryRecordsPanel tag={props.plugin.name} />}
      extraTabs={[
        {
          value: "insights",
          icon: <BarChart3 className="mr-1 h-3.5 w-3.5" />,
          label: t(WEBUI.queryRecorder.insightsTab),
          content: <QueryRecorderInsightsPanel tag={props.plugin.name} />,
        },
      ]}
    />
  );
}

type QueryRecordFilterForm = {
  search: string;
  qtype: string;
  rcode: string;
  status: QueryRecordStatusFilter;
  sinceLocal: string;
  untilLocal: string;
};

const EMPTY_FILTER_FORM: QueryRecordFilterForm = {
  search: "",
  qtype: "all",
  rcode: "all",
  status: "all",
  sinceLocal: "",
  untilLocal: "",
};

const QTYPE_OPTIONS = [
  "A",
  "AAAA",
  "HTTPS",
  "CNAME",
  "TXT",
  "MX",
  "NS",
  "SOA",
  "SRV",
  "PTR",
  "SVCB",
  "CAA",
  "DNSKEY",
  "DS",
];

const RCODE_OPTIONS = [
  "No Error",
  "Non-Existent Domain",
  "Server Failure",
  "Query Refused",
  "Format Error",
  "Not Implemented",
];

const TOP_PAGE_SIZE = 20;
// Per-row height for the horizontal Top-N bar charts. Recharts' category axis
// drops tick labels when bars are packed too tightly, so the chart container
// grows with the row count to guarantee every Top-N entry stays labeled.
const CHART_ROW_HEIGHT = 24;
const CHART_MIN_HEIGHT = 240;
const CHART_VERTICAL_PADDING = 16;
// Cap the chart's visible footprint. When Load more pushes the row count past
// what fits here, the chart keeps its full per-row height and the container
// scrolls instead of growing without bound, so labels stay 1:1 either way.
const CHART_MAX_HEIGHT = 640;

// ---------------------------------------------------------------------------
// Statistics tab: MatcherStatsCard on top, QueryRecordsPanel
// below, sharing a single filter form (incl. matcherTag).
// ---------------------------------------------------------------------------

export function QueryRecordsPanel({ tag }: { tag: string }) {
  const appliedStatus = usePluginAppliedStatus(tag);
  if (appliedStatus === "not-applied") {
    return <PluginNotAppliedPlaceholder />;
  }
  return <QueryRecordsPanelInner tag={tag} />;
}

function QueryRecordsPanelInner({ tag }: { tag: string }) {
  const { locale, t } = useI18n();
  const [records, setRecords] = useState<QueryRecordRow[]>([]);
  const [nextCursor, setNextCursor] = useState<string | undefined>();
  const [matcherStats, setMatcherStats] = useState<
    QueryRecorderPluginStatsRow[]
  >([]);
  const [statsQueryTotal, setStatsQueryTotal] = useState(0);
  const [selected, setSelected] = useState<QueryRecordDetail | null>(null);
  const [filterForm, setFilterForm] = useState<QueryRecordFilterForm>({
    ...EMPTY_FILTER_FORM,
  });
  const [appliedFilters, setAppliedFilters] = useState<QueryRecordFilters>({});
  const [loading, setLoading] = useState(false);
  const [statsLoading, setStatsLoading] = useState(false);
  const [clearing, setClearing] = useState(false);
  const [streaming, setStreaming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [statsError, setStatsError] = useState<string | null>(null);
  const [lastClearCount, setLastClearCount] = useState<number | null>(null);
  // SSE controller (existing behavior).
  const abortRef = useRef<AbortController | null>(null);
  // Cancel the previous /records load when a new one starts (e.g. rapid
  // matcher-click). Without this each click piles a new SQL query onto the
  // blocking pool while the previous one is still running.
  const recordsAbortRef = useRef<AbortController | null>(null);
  const statsAbortRef = useRef<AbortController | null>(null);
  const filtersRef = useRef<QueryRecordFilters>({});
  const mountedRef = useRef(false);
  const refreshGenerationRef = useRef(0);

  useEffect(() => {
    filtersRef.current = appliedFilters;
  }, [appliedFilters]);

  const loadRecords = useCallback(
    async (filters: QueryRecordFilters, cursor?: string): Promise<boolean> => {
      recordsAbortRef.current?.abort();
      const controller = new AbortController();
      recordsAbortRef.current = controller;
      setLoading(true);
      setError(null);
      try {
        const response = await fetchQueryRecords(tag, {
          limit: 100,
          cursor,
          ...filters,
          signal: controller.signal,
        });
        if (controller.signal.aborted || !mountedRef.current) return false;
        setRecords((current) =>
          cursor ? [...current, ...response.records] : response.records,
        );
        setNextCursor(response.next_cursor);
        return true;
      } catch (err) {
        if (controller.signal.aborted || !mountedRef.current) return false;
        setError(
          err instanceof Error
            ? err.message
            : t(WEBUI.queryRecorder.readRecordsFailed),
        );
        return false;
      } finally {
        if (recordsAbortRef.current === controller) {
          recordsAbortRef.current = null;
          if (mountedRef.current) setLoading(false);
        }
      }
    },
    [tag, t],
  );

  const loadMatcherStats = useCallback(
    async (filters: QueryRecordFilters) => {
      statsAbortRef.current?.abort();
      const controller = new AbortController();
      statsAbortRef.current = controller;
      setStatsLoading(true);
      setStatsError(null);
      try {
        const response = await fetchQueryRecorderPluginStats(tag, {
          kind: "matcher",
          ...filters,
          matcherTag: undefined,
          signal: controller.signal,
        });
        if (controller.signal.aborted || !mountedRef.current) return;
        setMatcherStats(response.stats.filter((row) => row.kind === "matcher"));
        setStatsQueryTotal(response.query_total);
      } catch (err) {
        if (controller.signal.aborted || !mountedRef.current) return;
        setStatsError(
          err instanceof Error
            ? err.message
            : t(WEBUI.queryRecorder.readMatcherStatsFailed),
        );
      } finally {
        if (statsAbortRef.current === controller) {
          statsAbortRef.current = null;
          if (mountedRef.current) setStatsLoading(false);
        }
      }
    },
    [tag, t],
  );

  const refresh = useCallback(
    async (filters: QueryRecordFilters) => {
      const generation = ++refreshGenerationRef.current;
      statsAbortRef.current?.abort();
      const recordsLoaded = await loadRecords(filters);
      if (
        !recordsLoaded ||
        !mountedRef.current ||
        refreshGenerationRef.current !== generation
      ) {
        return;
      }
      await loadMatcherStats(filters);
    },
    [loadMatcherStats, loadRecords],
  );

  useEffect(() => {
    mountedRef.current = true;
    const initialFilters = filtersRef.current;
    const timer = window.setTimeout(() => {
      void refresh(initialFilters);
    }, 0);
    return () => {
      mountedRef.current = false;
      refreshGenerationRef.current += 1;
      window.clearTimeout(timer);
      abortRef.current?.abort();
      recordsAbortRef.current?.abort();
      statsAbortRef.current?.abort();
    };
  }, [refresh]);

  const applyFilters = () => {
    const nextFilters = filtersFromForm(filterForm);
    setAppliedFilters(nextFilters);
    void refresh(nextFilters);
  };

  const clearFilters = () => {
    setFilterForm({ ...EMPTY_FILTER_FORM });
    setAppliedFilters({});
    void refresh({});
  };

  const openDetail = async (record: QueryRecordRow) => {
    setError(null);
    try {
      const detail = await fetchQueryRecordDetail(tag, record.id);
      setSelected(detail.record);
    } catch (err) {
      setError(
        err instanceof Error
          ? err.message
          : t(WEBUI.queryRecorder.readDetailFailed),
      );
    }
  };

  const handleClearHistory = async () => {
    recordsAbortRef.current?.abort();
    statsAbortRef.current?.abort();
    setClearing(true);
    setError(null);
    setStatsError(null);
    setLastClearCount(null);
    try {
      const response = await clearQueryRecorderHistory(tag);
      setRecords([]);
      setNextCursor(undefined);
      setSelected(null);
      setMatcherStats([]);
      setStatsQueryTotal(0);
      setLastClearCount(response.cleared_records);
      await refresh(appliedFilters);
    } catch (err) {
      setError(
        err instanceof Error
          ? err.message
          : t(WEBUI.queryRecorder.clearHistoryFailed),
      );
    } finally {
      setClearing(false);
    }
  };

  const toggleStream = async () => {
    if (streaming) {
      abortRef.current?.abort();
      abortRef.current = null;
      setStreaming(false);
      return;
    }

    const controller = new AbortController();
    abortRef.current = controller;
    setStreaming(true);
    setError(null);
    try {
      const response = await apiRequest(
        apiUrl(`/plugins/${encodeURIComponent(tag)}/stream?tail=20`),
        {
          headers: { ...apiHeaders(), Accept: "text/event-stream" },
          signal: controller.signal,
        },
      );
      if (!response.ok || !response.body) {
        throw new Error(
          t(WEBUI.queryRecorder.streamHttpFailed, { status: response.status }),
        );
      }
      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";
      while (!controller.signal.aborted) {
        const { done, value } = await readApiResponseBody(response, () =>
          reader.read(),
        );
        if (done) break;
        buffer += decoder
          .decode(value, { stream: true })
          .replace(/\r\n/g, "\n")
          .replace(/\r/g, "\n");
        const chunks = buffer.split("\n\n");
        buffer = chunks.pop() ?? "";
        for (const chunk of chunks) {
          const event = parseSseEvent(chunk);
          if (event.event === "error") {
            setError(
              event.data
                ? parseSseErrorMessage(event.data, t)
                : t(WEBUI.queryRecorder.streamError),
            );
            continue;
          }
          if (!event.data) continue;
          const record = parseStreamRecord(event.data);
          if (!record) {
            setError(t(WEBUI.queryRecorder.streamInvalidEvent));
            continue;
          }
          if (!recordMatchesFilters(record, filtersRef.current)) continue;
          setRecords((current) =>
            [record, ...current.filter((item) => item.id !== record.id)].slice(
              0,
              200,
            ),
          );
        }
      }
    } catch (err) {
      if (!controller.signal.aborted) {
        setError(
          err instanceof Error
            ? err.message
            : t(WEBUI.queryRecorder.streamConnFailed),
        );
      }
    } finally {
      if (abortRef.current === controller) {
        abortRef.current = null;
        setStreaming(false);
      }
    }
  };

  const activeFilterCount = countActiveFilters(appliedFilters);

  return (
    <div className="space-y-4">
      <MatcherStatsCard
        stats={matcherStats}
        queryTotal={statsQueryTotal}
        loading={statsLoading}
        recordsLoading={loading}
        error={statsError}
        selectedMatcher={appliedFilters.matcherTag}
        onSelectMatcher={(matcherTag) => {
          const nextFilters: QueryRecordFilters = {
            ...appliedFilters,
            matcherTag: matcherTag || undefined,
          };
          setAppliedFilters(nextFilters);
          void loadRecords(nextFilters);
        }}
        onRefresh={() => void loadMatcherStats(appliedFilters)}
      />
      <Card>
        <CardHeader className="grid gap-3 p-4 pb-2 sm:grid-cols-[1fr_auto] sm:items-center">
          <div className="min-w-0">
            <CardTitle className="text-sm">
              {t(WEBUI.queryRecorder.recordsTitle)}
            </CardTitle>
            <div className="mt-2 flex flex-wrap gap-2 text-xs text-muted-foreground">
              <span className="rounded-full border bg-muted/30 px-2 py-0.5">
                {t(WEBUI.queryRecorder.loadedRecords, {
                  count: records.length,
                })}
              </span>
              {activeFilterCount > 0 && (
                <span className="rounded-full border border-primary/30 bg-primary/10 px-2 py-0.5 text-primary">
                  {t(WEBUI.queryRecorder.activeFilters, {
                    count: activeFilterCount,
                  })}
                </span>
              )}
              <span className="rounded-full border bg-muted/30 px-2 py-0.5">
                {t(WEBUI.queryRecorder.errorCount, {
                  count: records.filter((record) => record.error).length,
                })}
              </span>
              {loading && (
                <span className="inline-flex items-center gap-1 rounded-full border border-primary/30 bg-primary/10 px-2 py-0.5 text-primary">
                  <Loader2 className="h-3 w-3 animate-spin" />
                  {t(WEBUI.queryRecorder.loadingIndicator)}
                </span>
              )}
              {clearing && (
                <span className="inline-flex items-center gap-1 rounded-full border border-destructive/30 bg-destructive/10 px-2 py-0.5 text-destructive">
                  <Loader2 className="h-3 w-3 animate-spin" />
                  {t(WEBUI.queryRecorder.clearingIndicator)}
                </span>
              )}
              {lastClearCount !== null && !clearing && (
                <span className="rounded-full border border-destructive/30 bg-destructive/10 px-2 py-0.5 text-destructive">
                  {t(WEBUI.queryRecorder.clearedCount, {
                    count: lastClearCount,
                  })}
                </span>
              )}
              {streaming && (
                <span className="rounded-full border border-primary/30 bg-primary/10 px-2 py-0.5 text-primary">
                  {t(WEBUI.queryRecorder.streamingStatus)}
                </span>
              )}
            </div>
          </div>
          <div className="flex flex-wrap justify-end gap-2">
            <Button
              variant="outline"
              size="sm"
              disabled={loading || clearing}
              onClick={() => void refresh(appliedFilters)}
            >
              <RefreshCw className="h-4 w-4" />
              {t(WEBUI.common.refresh)}
            </Button>
            <Button
              variant={streaming ? "secondary" : "outline"}
              size="sm"
              disabled={clearing}
              onClick={() => void toggleStream()}
            >
              <Radio className="h-4 w-4" />
              {streaming
                ? t(WEBUI.queryRecorder.stopStreaming)
                : t(WEBUI.queryRecorder.startStreaming)}
            </Button>
            <AlertDialog>
              <AlertDialogTrigger asChild>
                <Button variant="outline" size="sm" disabled={clearing}>
                  {clearing ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    <Trash2 className="h-4 w-4" />
                  )}
                  {t(WEBUI.queryRecorder.clearHistoryButton)}
                </Button>
              </AlertDialogTrigger>
              <AlertDialogContent>
                <AlertDialogHeader>
                  <AlertDialogMedia className="bg-destructive/10 text-destructive">
                    <Trash2 className="h-5 w-5" />
                  </AlertDialogMedia>
                  <AlertDialogTitle>
                    {t(WEBUI.queryRecorder.clearHistoryTitle)}
                  </AlertDialogTitle>
                  <AlertDialogDescription>
                    {t(WEBUI.queryRecorder.clearHistoryDesc, { tag })}
                  </AlertDialogDescription>
                </AlertDialogHeader>
                <AlertDialogFooter>
                  <AlertDialogCancel disabled={clearing}>
                    {t(WEBUI.common.cancel)}
                  </AlertDialogCancel>
                  <AlertDialogAction
                    variant="destructive"
                    disabled={clearing}
                    onClick={() => void handleClearHistory()}
                  >
                    {t(WEBUI.queryRecorder.clearHistoryButton)}
                  </AlertDialogAction>
                </AlertDialogFooter>
              </AlertDialogContent>
            </AlertDialog>
          </div>
        </CardHeader>
        <CardContent className="p-4 pt-0">
          <form
            className="mb-3 rounded-md border bg-muted/20 p-3"
            onSubmit={(event) => {
              event.preventDefault();
              applyFilters();
            }}
          >
            <div className="grid gap-2 md:grid-cols-2 xl:grid-cols-4">
              <FilterField label={t(WEBUI.queryRecorder.keywordFilter)}>
                <Input
                  value={filterForm.search}
                  onChange={(event) =>
                    setFilterForm((current) => ({
                      ...current,
                      search: event.target.value,
                    }))
                  }
                  placeholder={t(WEBUI.queryRecorder.keywordPlaceholder)}
                  className="h-8 font-mono"
                />
              </FilterField>
              <FilterField label="QTYPE">
                <Select
                  value={filterForm.qtype}
                  onValueChange={(qtype) =>
                    setFilterForm((current) => ({ ...current, qtype }))
                  }
                >
                  <SelectTrigger className="h-8 font-mono">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">{t(WEBUI.common.all)}</SelectItem>
                    {QTYPE_OPTIONS.map((qtype) => (
                      <SelectItem key={qtype} value={qtype}>
                        {qtype}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </FilterField>
              <FilterField label="RCODE">
                <Select
                  value={filterForm.rcode}
                  onValueChange={(rcode) =>
                    setFilterForm((current) => ({ ...current, rcode }))
                  }
                >
                  <SelectTrigger className="h-8 font-mono">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">{t(WEBUI.common.all)}</SelectItem>
                    {RCODE_OPTIONS.map((rcode) => (
                      <SelectItem key={rcode} value={rcode}>
                        {rcode}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </FilterField>
              <FilterField label={t(WEBUI.queryRecorder.statusFilter)}>
                <Select
                  value={filterForm.status}
                  onValueChange={(status) =>
                    setFilterForm((current) => ({
                      ...current,
                      status: status as QueryRecordStatusFilter,
                    }))
                  }
                >
                  <SelectTrigger className="h-8">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">{t(WEBUI.common.all)}</SelectItem>
                    <SelectItem value="error">
                      {t(WEBUI.queryRecorder.statusError)}
                    </SelectItem>
                    <SelectItem value="has_response">
                      {t(WEBUI.queryRecorder.statusHasResponse)}
                    </SelectItem>
                    <SelectItem value="no_response">
                      {t(WEBUI.queryRecorder.statusNoResponse)}
                    </SelectItem>
                  </SelectContent>
                </Select>
              </FilterField>
              <FilterField label={t(WEBUI.queryRecorder.startTimeFilter)}>
                <DateTimePicker
                  value={filterForm.sinceLocal}
                  onChange={(sinceLocal) =>
                    setFilterForm((current) => ({
                      ...current,
                      sinceLocal,
                    }))
                  }
                  placeholder={t(WEBUI.queryRecorder.startTimePlaceholder)}
                />
              </FilterField>
              <FilterField label={t(WEBUI.queryRecorder.endTimeFilter)}>
                <DateTimePicker
                  value={filterForm.untilLocal}
                  onChange={(untilLocal) =>
                    setFilterForm((current) => ({
                      ...current,
                      untilLocal,
                    }))
                  }
                  placeholder={t(WEBUI.queryRecorder.endTimePlaceholder)}
                />
              </FilterField>
              <div className="flex items-end gap-2">
                <Button type="submit" size="sm" className="h-8">
                  <Filter className="h-4 w-4" />
                  {t(WEBUI.common.apply)}
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="h-8"
                  onClick={clearFilters}
                >
                  <X className="h-4 w-4" />
                  {t(WEBUI.common.clear)}
                </Button>
              </div>
            </div>
          </form>
          {error && (
            <div className="mb-3 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              {error}
            </div>
          )}
          <div className="relative overflow-x-auto rounded-md border">
            {loading && (
              <div
                className="pointer-events-none absolute inset-0 z-10 flex items-center justify-center bg-background/60 backdrop-blur-[1px]"
                aria-live="polite"
                role="status"
              >
                <div className="inline-flex items-center gap-2 rounded-full border border-primary/30 bg-background/95 px-3 py-1.5 text-xs text-primary shadow-sm">
                  <Loader2 className="h-4 w-4 animate-spin" />
                  {t(WEBUI.queryRecorder.loadingOverlay)}
                </div>
              </div>
            )}
            <Table className="min-w-[980px]">
              <TableHeader>
                <TableRow className="bg-muted/30 hover:bg-muted/30">
                  <TableHead>{t(WEBUI.queryRecorder.queryColumn)}</TableHead>
                  <TableHead>{t(WEBUI.queryRecorder.clientColumn)}</TableHead>
                  <TableHead>{t(WEBUI.queryRecorder.timeColumn)}</TableHead>
                  <TableHead>{t(WEBUI.queryRecorder.resultColumn)}</TableHead>
                  <TableHead>
                    {t(WEBUI.queryRecorder.answerPreviewColumn)}
                  </TableHead>
                  <TableHead>{t(WEBUI.queryRecorder.elapsedColumn)}</TableHead>
                  <TableHead>
                    <span className="inline-flex items-center gap-1">
                      {t(WEBUI.queryRecorder.recordCountColumn)}
                      <Popover>
                        <PopoverTrigger asChild>
                          <button
                            type="button"
                            className="inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none"
                            aria-label={t(WEBUI.queryRecorder.recordCountAria)}
                          >
                            <Info className="h-3.5 w-3.5" />
                          </button>
                        </PopoverTrigger>
                        <PopoverContent
                          side="top"
                          className="w-auto max-w-[16rem] p-2 text-xs"
                        >
                          {t(WEBUI.dnsRecord.responseRecordsTooltip)}
                        </PopoverContent>
                      </Popover>
                    </span>
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {records.map((record) => (
                  <TableRow
                    key={record.id}
                    className="cursor-pointer"
                    onClick={() => openDetail(record)}
                  >
                    <TableCell className="max-w-[22rem]">
                      <div className="flex min-w-0 items-center gap-2">
                        <span
                          className="truncate font-mono"
                          title={formatQuestion(record)}
                        >
                          {formatQuestionName(record)}
                        </span>
                        {formatQuestionType(record) !== "-" && (
                          <Badge variant="outline" className="font-mono">
                            {formatQuestionType(record)}
                          </Badge>
                        )}
                      </div>
                    </TableCell>
                    <TableCell className="font-mono text-xs">
                      {record.client_ip}
                    </TableCell>
                    <TableCell className="font-mono text-xs text-muted-foreground">
                      {formatTime(record.created_at_ms, locale)}
                    </TableCell>
                    <TableCell>{queryStatusBadge(record)}</TableCell>
                    <TableCell className="max-w-[20rem]">
                      <AnswerPreviewCell record={record} />
                    </TableCell>
                    <TableCell className="font-mono">
                      <span
                        className={cn(
                          "inline-flex min-w-16 justify-center rounded border px-1.5 py-0.5 text-xs font-medium tabular-nums",
                          queryElapsedClassName(record.elapsed_ms),
                        )}
                        title={queryElapsedTitle(record.elapsed_ms, t)}
                      >
                        {record.elapsed_ms}ms
                      </span>
                    </TableCell>
                    <TableCell>
                      <div className="flex items-center gap-1 font-mono text-xs">
                        <span>{record.answer_count}</span>
                        <span className="text-muted-foreground">
                          / {record.authority_count} / {record.additional_count}
                        </span>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
                {!records.length && (
                  <TableRow>
                    <TableCell
                      colSpan={7}
                      className="h-24 text-center text-muted-foreground"
                    >
                      {loading
                        ? t(WEBUI.queryRecorder.loadingRecords)
                        : t(WEBUI.queryRecorder.noRecords)}
                    </TableCell>
                  </TableRow>
                )}
              </TableBody>
            </Table>
          </div>
          {nextCursor && (
            <Button
              variant="outline"
              size="sm"
              className="mt-3"
              disabled={loading}
              onClick={() => void loadRecords(appliedFilters, nextCursor)}
            >
              {t(WEBUI.common.loadMore)}
            </Button>
          )}
        </CardContent>
        <RecordDetailDialog
          record={selected}
          onClose={() => setSelected(null)}
        />
      </Card>
    </div>
  );
}

function MatcherStatsCard({
  stats,
  queryTotal,
  loading,
  recordsLoading,
  error,
  selectedMatcher,
  onSelectMatcher,
  onRefresh,
}: {
  stats: QueryRecorderPluginStatsRow[];
  queryTotal: number;
  loading: boolean;
  /**
   * Records list is currently fetching — typically because the user just
   * clicked this matcher row. Surfaces a spinner on the selected row so the
   * click feels responsive even before /records returns.
   */
  recordsLoading: boolean;
  error: string | null;
  selectedMatcher?: string;
  onSelectMatcher: (matcherTag: string | undefined) => void;
  onRefresh: () => void;
}) {
  const { t } = useI18n();

  return (
    <Card>
      <CardHeader className="grid gap-3 p-4 pb-2 sm:grid-cols-[1fr_auto] sm:items-center">
        <div className="min-w-0">
          <CardTitle className="flex items-center gap-2 text-sm">
            <BarChart3 className="h-4 w-4 text-primary" />
            {t(WEBUI.queryRecorder.matcherStatsTitle)}
          </CardTitle>
          <div className="mt-2 flex flex-wrap gap-2 text-xs text-muted-foreground">
            <span className="rounded-full border bg-muted/30 px-2 py-0.5">
              {t(WEBUI.queryRecorder.sampleCount, { count: queryTotal })}
            </span>
            <span className="rounded-full border bg-muted/30 px-2 py-0.5">
              {t(WEBUI.queryRecorder.matcherCount, { count: stats.length })}
            </span>
            {selectedMatcher && (
              <button
                type="button"
                className="inline-flex items-center gap-1 rounded-full border border-primary/30 bg-primary/10 px-2 py-0.5 font-mono text-primary hover:bg-primary/20"
                onClick={() => onSelectMatcher(undefined)}
                title={t(WEBUI.queryRecorder.clearMatcherFilterTitle)}
              >
                {t(WEBUI.queryRecorder.selectedMatcher, {
                  matcher: selectedMatcher,
                })}
                <X className="h-3 w-3" />
              </button>
            )}
          </div>
        </div>
        <Button
          variant="outline"
          size="sm"
          disabled={loading}
          onClick={onRefresh}
        >
          <RefreshCw className="h-4 w-4" />
          {t(WEBUI.common.refresh)}
        </Button>
      </CardHeader>
      <CardContent className="p-4 pt-0">
        {error && (
          <div className="mb-3 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
            {error}
          </div>
        )}
        <div className="overflow-hidden rounded-md border">
          <Table className="min-w-[680px]">
            <TableHeader>
              <TableRow className="bg-muted/30 hover:bg-muted/30">
                <TableHead>{t(WEBUI.queryRecorder.matcherColumn)}</TableHead>
                <TableHead>{t(WEBUI.queryRecorder.checkedColumn)}</TableHead>
                <TableHead>{t(WEBUI.queryRecorder.hitColumn)}</TableHead>
                <TableHead>{t(WEBUI.queryRecorder.hitRateColumn)}</TableHead>
                <TableHead>{t(WEBUI.queryRecorder.queryShareColumn)}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {stats.map((row) => {
                const tag = row.tag;
                const selectable = Boolean(tag);
                const isSelected = selectable && tag === selectedMatcher;
                return (
                  <TableRow
                    key={tag ?? "(unknown)"}
                    className={
                      selectable
                        ? `cursor-pointer ${isSelected ? "bg-primary/10 hover:bg-primary/15" : ""}`
                        : ""
                    }
                    onClick={
                      selectable
                        ? () => onSelectMatcher(isSelected ? undefined : tag)
                        : undefined
                    }
                    title={
                      selectable
                        ? isSelected
                          ? t(WEBUI.queryRecorder.clickToCancelFilter)
                          : t(WEBUI.queryRecorder.clickToFilter)
                        : undefined
                    }
                  >
                    <TableCell className="font-mono">
                      <div className="flex items-center gap-2">
                        <span>{tag ?? "(unknown)"}</span>
                        {isSelected && (
                          <Badge
                            variant="secondary"
                            className="inline-flex items-center gap-1 font-mono"
                          >
                            {recordsLoading && (
                              <Loader2 className="h-3 w-3 animate-spin" />
                            )}
                            {recordsLoading
                              ? t(WEBUI.queryRecorder.loadingBadge)
                              : t(WEBUI.queryRecorder.selectedBadge)}
                          </Badge>
                        )}
                      </div>
                    </TableCell>
                    <TableCell className="font-mono">{row.checked}</TableCell>
                    <TableCell className="font-mono">{row.matched}</TableCell>
                    <TableCell className="font-mono">
                      {formatPercent(safeRatio(row.matched, row.checked))}
                    </TableCell>
                    <TableCell className="font-mono">
                      {formatPercent(row.query_share)}
                    </TableCell>
                  </TableRow>
                );
              })}
              {!stats.length && (
                <TableRow>
                  <TableCell
                    colSpan={5}
                    className="h-20 text-center text-muted-foreground"
                  >
                    {loading
                      ? t(WEBUI.queryRecorder.loadingMatcherStats)
                      : t(WEBUI.queryRecorder.noMatcherStats)}
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
        </div>
      </CardContent>
    </Card>
  );
}

// ---------------------------------------------------------------------------
// Aggregate tab: top-level insights independent from the Statistics tab's
// filter form. Only knob is a preset time range; sub-tabs render charts.
// ---------------------------------------------------------------------------

type TFn = (
  key: string,
  params?: Record<string, string | number | boolean | null | undefined>,
) => string;
type InsightsRangeKey = InsightsRange;
const RANGE_PRESET_KEYS = INSIGHTS_RANGES;
function rangePresetLabel(range: InsightsRangeKey, t: (key: string) => string) {
  return t(
    {
      "1h": WEBUI.queryRecorder.rangePreset1h,
      "24h": WEBUI.queryRecorder.rangePreset24h,
      "7d": WEBUI.queryRecorder.rangePreset7d,
      "30d": WEBUI.queryRecorder.rangePreset30d,
      "1y": WEBUI.queryRecorder.rangePreset1y,
    }[range],
  );
}
function rangeToFilters(
  range: InsightsRangeKey,
  now: number,
): QueryRecordFilters {
  const { sinceMs, untilMs } = queryRecorderRange(range, now);
  return { sinceMs, untilMs };
}

export function QueryRecorderInsightsPanel({ tag }: { tag: string }) {
  const appliedStatus = usePluginAppliedStatus(tag);
  if (appliedStatus === "not-applied") {
    return <PluginNotAppliedPlaceholder />;
  }
  return <QueryRecorderInsightsPanelInner tag={tag} />;
}

function QueryRecorderInsightsPanelInner({ tag }: { tag: string }) {
  const { t } = useI18n();
  const [range, setRange] = useState<InsightsRangeKey>("24h");
  // `nonce` lets the user force a refresh of every sub-tab without changing
  // the range; we bump it on the toolbar refresh button.
  const [nonce, setNonce] = useState(() => Date.now());
  const refresh = () => setNonce(Date.now());
  const filters = useMemo(() => rangeToFilters(range, nonce), [range, nonce]);
  const rangeLabel = useMemo(() => rangePresetLabel(range, t), [range, t]);

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader className="grid gap-3 p-4 pb-2 sm:grid-cols-[1fr_auto] sm:items-center">
          <div className="min-w-0">
            <CardTitle className="flex items-center gap-2 text-sm">
              <BarChart3 className="h-4 w-4 text-primary" />
              {t(WEBUI.queryRecorder.insightsViewTitle)}
            </CardTitle>
            <div className="mt-2 flex flex-wrap gap-2 text-xs text-muted-foreground">
              <span className="rounded-full border bg-muted/30 px-2 py-0.5">
                {rangeLabel}
              </span>
              <span className="rounded-full border bg-muted/30 px-2 py-0.5">
                {t(WEBUI.queryRecorder.insightsIndependent)}
              </span>
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <PresetRangePicker
              value={range}
              onChange={(next) => {
                setRange(next);
                refresh();
              }}
            />
            <Button variant="outline" size="sm" onClick={refresh}>
              <RefreshCw className="h-4 w-4" />
              {t(WEBUI.common.refresh)}
            </Button>
          </div>
        </CardHeader>
      </Card>

      <Tabs defaultValue="timeseries">
        <TabsList className="grid w-full auto-rows-8 grid-cols-3 gap-1 group-data-horizontal/tabs:h-auto sm:grid-cols-6">
          <TabsTrigger value="clients" className="shrink-0">
            <Users className="h-3.5 w-3.5" />
            {t(WEBUI.queryRecorder.clientsTab)}
          </TabsTrigger>
          <TabsTrigger value="qnames" className="shrink-0">
            <Globe className="h-3.5 w-3.5" />
            {t(WEBUI.queryRecorder.domainsTab)}
          </TabsTrigger>
          <TabsTrigger value="qtype" className="shrink-0">
            <BarChart3 className="h-3.5 w-3.5" />
            QTYPE
          </TabsTrigger>
          <TabsTrigger value="rcode" className="shrink-0">
            <PieChartIcon className="h-3.5 w-3.5" />
            RCODE
          </TabsTrigger>
          <TabsTrigger value="latency" className="shrink-0">
            <Timer className="h-3.5 w-3.5" />
            {t(WEBUI.queryRecorder.latencyTab)}
          </TabsTrigger>
          <TabsTrigger value="timeseries" className="shrink-0">
            <TrendingUp className="h-3.5 w-3.5" />
            {t(WEBUI.queryRecorder.trendTab)}
          </TabsTrigger>
        </TabsList>
        <TabsContent value="clients" className="min-h-[40rem]">
          <TopBucketsCard
            key={`clients-${tag}-${range}-${nonce}`}
            title={t(WEBUI.queryRecorder.topClientsTitle)}
            icon={<Users className="h-4 w-4 text-primary" />}
            tag={tag}
            filters={filters}
            fetcher={fetchQueryRecorderTopClients}
            keyLabel={t(WEBUI.queryRecorder.clientIpKeyLabel)}
          />
        </TabsContent>
        <TabsContent value="qnames" className="min-h-[40rem]">
          <TopBucketsCard
            key={`qnames-${tag}-${range}-${nonce}`}
            title={t(WEBUI.queryRecorder.topDomainsTitle)}
            icon={<Globe className="h-4 w-4 text-primary" />}
            tag={tag}
            filters={filters}
            fetcher={fetchQueryRecorderTopQnames}
            keyLabel="QNAME"
          />
        </TabsContent>
        <TabsContent value="qtype" className="min-h-[40rem]">
          <DistributionCard
            key={`qtype-${tag}-${range}-${nonce}`}
            title={t(WEBUI.queryRecorder.qtypeDistTitle)}
            icon={<BarChart3 className="h-4 w-4 text-primary" />}
            tag={tag}
            filters={filters}
            fetcher={fetchQueryRecorderQtypeDistribution}
            keyLabel="QTYPE"
            preferBarChart
          />
        </TabsContent>
        <TabsContent value="rcode" className="min-h-[40rem]">
          <DistributionCard
            key={`rcode-${tag}-${range}-${nonce}`}
            title={t(WEBUI.queryRecorder.rcodeDistTitle)}
            icon={<PieChartIcon className="h-4 w-4 text-primary" />}
            tag={tag}
            filters={filters}
            fetcher={fetchQueryRecorderRcodeDistribution}
            keyLabel="RCODE"
            preferBarChart={false}
          />
        </TabsContent>
        <TabsContent value="latency" className="min-h-[40rem]">
          <LatencyCard
            key={`latency-${tag}-${range}-${nonce}`}
            tag={tag}
            filters={filters}
          />
        </TabsContent>
        <TabsContent value="timeseries" className="min-h-[40rem]">
          <TimeseriesCard
            key={`timeseries-${tag}-${range}`}
            tag={tag}
            filters={filters}
            range={range}
            onRefresh={refresh}
          />
        </TabsContent>
      </Tabs>
    </div>
  );
}

function PresetRangePicker({
  value,
  onChange,
}: {
  value: InsightsRangeKey;
  onChange: (next: InsightsRangeKey) => void;
}) {
  const { t } = useI18n();
  return (
    <div className="inline-flex min-h-8 flex-wrap items-center rounded-md border bg-muted/30 p-0.5">
      {RANGE_PRESET_KEYS.map((presetKey) => {
        const active = presetKey === value;
        return (
          <button
            key={presetKey}
            type="button"
            onClick={() => onChange(presetKey)}
            aria-pressed={active}
            className={cn(
              "rounded px-2 py-1 text-xs transition-colors",
              active
                ? "bg-background text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            {rangePresetLabel(presetKey, t)}
          </button>
        );
      })}
    </div>
  );
}

function TopBucketsCard({
  title,
  icon,
  tag,
  filters,
  fetcher,
  keyLabel,
}: {
  title: string;
  icon: ReactNode;
  tag: string;
  filters: QueryRecordFilters;
  fetcher: (
    tag: string,
    options: QueryRecordFilters & { limit?: number; signal?: AbortSignal },
  ) => Promise<QueryRecorderTopResponse>;
  keyLabel: string;
}) {
  const { t } = useI18n();
  const [data, setData] = useState<QueryRecorderTopResponse | null>(null);
  const [limit, setLimit] = useState(TOP_PAGE_SIZE);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const abortRef = useRef<AbortController | null>(null);

  const load = useCallback(
    async (requestedLimit: number) => {
      abortRef.current?.abort();
      const controller = new AbortController();
      abortRef.current = controller;
      setLoading(true);
      setError(null);
      try {
        const response = await fetcher(tag, {
          ...filters,
          limit: requestedLimit,
          signal: controller.signal,
        });
        if (controller.signal.aborted || abortRef.current !== controller) {
          return;
        }
        setData(response);
        setLimit(requestedLimit);
      } catch (err) {
        if (controller.signal.aborted || abortRef.current !== controller) {
          return;
        }
        setError(
          err instanceof Error
            ? err.message
            : t(WEBUI.queryRecorder.readStatsFailed),
        );
      } finally {
        if (abortRef.current === controller) {
          abortRef.current = null;
          setLoading(false);
        }
      }
    },
    [tag, filters, fetcher, t],
  );

  useEffect(() => {
    const timer = window.setTimeout(() => {
      void load(TOP_PAGE_SIZE);
    }, 0);
    return () => {
      window.clearTimeout(timer);
      abortRef.current?.abort();
    };
  }, [load]);

  const chartData = useMemo(
    () =>
      (data?.rows ?? []).slice().map((row) => ({
        key: row.key,
        count: row.count,
        share: row.share,
      })),
    [data],
  );
  const hasMoreRows = chartData.length >= limit;
  // Keep one readable slot per category; the surrounding panel scrolls.
  const chartHeight = Math.max(
    CHART_MIN_HEIGHT,
    chartData.length * CHART_ROW_HEIGHT + CHART_VERTICAL_PADDING,
  );

  return (
    <Card className="mt-3">
      <CardHeader className="grid gap-3 p-4 pb-2 sm:grid-cols-[1fr_auto] sm:items-center">
        <div className="min-w-0">
          <CardTitle className="flex items-center gap-2 text-sm">
            {icon}
            {title}
          </CardTitle>
          <div className="mt-2 flex flex-wrap gap-2 text-xs text-muted-foreground">
            <span className="rounded-full border bg-muted/30 px-2 py-0.5">
              {t(WEBUI.queryRecorder.sampleCount, {
                count: data?.sample_size ?? 0,
              })}
            </span>
            <span className="rounded-full border bg-muted/30 px-2 py-0.5">
              {t(WEBUI.queryRecorder.topCount, { count: chartData.length })}
            </span>
          </div>
        </div>
        <Button
          variant="outline"
          size="sm"
          disabled={loading}
          onClick={() => void load(limit)}
        >
          <RefreshCw className="h-4 w-4" />
          {t(WEBUI.common.refresh)}
        </Button>
      </CardHeader>
      <CardContent className="space-y-4 p-4 pt-0">
        {error && (
          <div className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
            {error}
          </div>
        )}
        {chartData.length > 0 && (
          <div
            className="w-full overflow-y-auto"
            style={{ maxHeight: CHART_MAX_HEIGHT }}
          >
            <div className="w-full" style={{ height: chartHeight }}>
              <AnalyticsChart
                label={title}
                option={distributionOption(chartData, keyLabel, "horizontal")}
                height={chartHeight}
              />
            </div>
          </div>
        )}
        <div className="overflow-hidden rounded-md border">
          <Table className="min-w-[560px]">
            <TableHeader>
              <TableRow className="bg-muted/30 hover:bg-muted/30">
                <TableHead className="w-12">#</TableHead>
                <TableHead>{keyLabel}</TableHead>
                <TableHead>{t(WEBUI.queryRecorder.countColumn)}</TableHead>
                <TableHead>{t(WEBUI.queryRecorder.shareColumn)}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {chartData.map((row, index) => (
                <TableRow key={row.key}>
                  <TableCell className="font-mono text-xs text-muted-foreground">
                    {index + 1}
                  </TableCell>
                  <TableCell className="font-mono">{row.key}</TableCell>
                  <TableCell className="font-mono">{row.count}</TableCell>
                  <TableCell className="font-mono">
                    {formatPercent(row.share)}
                  </TableCell>
                </TableRow>
              ))}
              {!chartData.length && (
                <TableRow>
                  <TableCell
                    colSpan={4}
                    className="h-20 text-center text-muted-foreground"
                  >
                    {loading
                      ? t(WEBUI.queryRecorder.loadingStats)
                      : t(WEBUI.queryRecorder.noData)}
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
        </div>
        {hasMoreRows && (
          <Button
            variant="outline"
            size="sm"
            disabled={loading}
            onClick={() => void load(limit + TOP_PAGE_SIZE)}
          >
            {t(WEBUI.common.loadMore)}
          </Button>
        )}
      </CardContent>
    </Card>
  );
}

function DistributionCard({
  title,
  icon,
  tag,
  filters,
  fetcher,
  keyLabel,
  preferBarChart,
}: {
  title: string;
  icon: ReactNode;
  tag: string;
  filters: QueryRecordFilters;
  fetcher: (
    tag: string,
    options: QueryRecordFilters & { signal?: AbortSignal },
  ) => Promise<QueryRecorderDistributionResponse>;
  keyLabel: string;
  preferBarChart: boolean;
}) {
  const { t } = useI18n();
  const [data, setData] = useState<QueryRecorderDistributionResponse | null>(
    null,
  );
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const abortRef = useRef<AbortController | null>(null);

  const load = useCallback(async () => {
    abortRef.current?.abort();
    const controller = new AbortController();
    abortRef.current = controller;
    setLoading(true);
    setError(null);
    try {
      const response = await fetcher(tag, {
        ...filters,
        signal: controller.signal,
      });
      if (controller.signal.aborted || abortRef.current !== controller) {
        return;
      }
      setData(response);
    } catch (err) {
      if (controller.signal.aborted || abortRef.current !== controller) {
        return;
      }
      setError(
        err instanceof Error
          ? err.message
          : t(WEBUI.queryRecorder.readDistFailed),
      );
    } finally {
      if (abortRef.current === controller) {
        abortRef.current = null;
        setLoading(false);
      }
    }
  }, [tag, filters, fetcher, t]);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      void load();
    }, 0);
    return () => {
      window.clearTimeout(timer);
      abortRef.current?.abort();
    };
  }, [load]);

  const rows = useMemo(() => data?.rows ?? [], [data]);

  return (
    <Card className="mt-3">
      <CardHeader className="grid gap-3 p-4 pb-2 sm:grid-cols-[1fr_auto] sm:items-center">
        <div className="min-w-0">
          <CardTitle className="flex items-center gap-2 text-sm">
            {icon}
            {title}
          </CardTitle>
          <div className="mt-2 flex flex-wrap gap-2 text-xs text-muted-foreground">
            <span className="rounded-full border bg-muted/30 px-2 py-0.5">
              {t(WEBUI.queryRecorder.sampleCount, {
                count: data?.sample_size ?? 0,
              })}
            </span>
            <span className="rounded-full border bg-muted/30 px-2 py-0.5">
              {t(WEBUI.queryRecorder.bucketCount, { count: rows.length })}
            </span>
          </div>
        </div>
        <Button
          variant="outline"
          size="sm"
          disabled={loading}
          onClick={() => void load()}
        >
          <RefreshCw className="h-4 w-4" />
          {t(WEBUI.common.refresh)}
        </Button>
      </CardHeader>
      <CardContent className="space-y-4 p-4 pt-0">
        {error && (
          <div className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
            {error}
          </div>
        )}
        {rows.length > 0 && (
          <div className="h-[280px] w-full">
            <AnalyticsChart
              label={title}
              option={distributionOption(
                rows,
                keyLabel,
                preferBarChart ? "bar" : "pie",
              )}
              height={280}
            />
          </div>
        )}
        <div className="overflow-hidden rounded-md border">
          <Table className="min-w-[480px]">
            <TableHeader>
              <TableRow className="bg-muted/30 hover:bg-muted/30">
                <TableHead>{keyLabel}</TableHead>
                <TableHead>{t(WEBUI.queryRecorder.countColumn)}</TableHead>
                <TableHead>{t(WEBUI.queryRecorder.shareColumn)}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((row) => (
                <TableRow key={row.key}>
                  <TableCell className="font-mono">{row.key}</TableCell>
                  <TableCell className="font-mono">{row.count}</TableCell>
                  <TableCell className="font-mono">
                    {formatPercent(row.share)}
                  </TableCell>
                </TableRow>
              ))}
              {!rows.length && (
                <TableRow>
                  <TableCell
                    colSpan={3}
                    className="h-20 text-center text-muted-foreground"
                  >
                    {loading
                      ? t(WEBUI.queryRecorder.loadingDist)
                      : t(WEBUI.queryRecorder.noData)}
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
        </div>
      </CardContent>
    </Card>
  );
}

function LatencyCard({
  tag,
  filters,
}: {
  tag: string;
  filters: QueryRecordFilters;
}) {
  const { t } = useI18n();
  const [data, setData] = useState<QueryRecorderLatencySummary | null>(null);
  const [slowLimit, setSlowLimit] = useState(TOP_PAGE_SIZE);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const abortRef = useRef<AbortController | null>(null);

  const load = useCallback(
    async (requestedSlowLimit: number) => {
      abortRef.current?.abort();
      const controller = new AbortController();
      abortRef.current = controller;
      setLoading(true);
      setError(null);
      try {
        const response = await fetchQueryRecorderLatency(tag, {
          ...filters,
          slowLimit: requestedSlowLimit,
          signal: controller.signal,
        });
        if (controller.signal.aborted || abortRef.current !== controller) {
          return;
        }
        setData(response);
        setSlowLimit(requestedSlowLimit);
      } catch (err) {
        if (controller.signal.aborted || abortRef.current !== controller) {
          return;
        }
        setError(
          err instanceof Error
            ? err.message
            : t(WEBUI.queryRecorder.readLatencyFailed),
        );
      } finally {
        if (abortRef.current === controller) {
          abortRef.current = null;
          setLoading(false);
        }
      }
    },
    [tag, filters, t],
  );

  useEffect(() => {
    const timer = window.setTimeout(() => {
      void load(TOP_PAGE_SIZE);
    }, 0);
    return () => {
      window.clearTimeout(timer);
      abortRef.current?.abort();
    };
  }, [load]);

  const histogram = useMemo(() => {
    const buckets = data?.histogram ?? [];
    return buckets.map((bucket, index) => {
      const prev = index === 0 ? 0 : (buckets[index - 1]?.lt_ms ?? 0);
      const lt = bucket.lt_ms;
      const label =
        lt === null
          ? `${prev}+ ms`
          : prev === 0
            ? `<${lt} ms`
            : `${prev}–${lt} ms`;
      // Color bars semantically: green → fast, sky → normal, amber → slow,
      // rose → very slow — same tiers as queryElapsedClassName.
      const upperBound = lt ?? Number.POSITIVE_INFINITY;
      const fill =
        upperBound <= 20
          ? "var(--color-emerald-500)"
          : upperBound <= 100
            ? "var(--color-sky-500)"
            : upperBound <= 300
              ? "var(--color-amber-500)"
              : "var(--color-rose-500)";
      return { label, count: bucket.count, fill };
    });
  }, [data]);
  const slowTop = data?.slow_top ?? [];
  const hasMoreSlowRows = slowTop.length >= slowLimit;

  return (
    <Card className="mt-3">
      <CardHeader className="grid gap-3 p-4 pb-2 sm:grid-cols-[1fr_auto] sm:items-center">
        <div className="min-w-0">
          <CardTitle className="flex items-center gap-2 text-sm">
            <Timer className="h-4 w-4 text-primary" />
            {t(WEBUI.queryRecorder.latencyDistTitle)}
          </CardTitle>
          <div className="mt-2 flex flex-wrap gap-2 text-xs text-muted-foreground">
            <span className="rounded-full border bg-muted/30 px-2 py-0.5">
              {t(WEBUI.queryRecorder.sampleCount, {
                count: data?.sample_size ?? 0,
              })}
            </span>
          </div>
        </div>
        <Button
          variant="outline"
          size="sm"
          disabled={loading}
          onClick={() => void load(slowLimit)}
        >
          <RefreshCw className="h-4 w-4" />
          {t(WEBUI.common.refresh)}
        </Button>
      </CardHeader>
      <CardContent className="space-y-4 p-4 pt-0">
        {error && (
          <div className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
            {error}
          </div>
        )}
        <div className="grid grid-cols-2 gap-2 md:grid-cols-5">
          <MetricChip label="P50" value={`${data?.p50_ms ?? 0} ms`} />
          <MetricChip label="P95" value={`${data?.p95_ms ?? 0} ms`} />
          <MetricChip label="P99" value={`${data?.p99_ms ?? 0} ms`} />
          <MetricChip
            label={t(WEBUI.queryRecorder.avgMetricLabel)}
            value={`${data ? data.avg_ms.toFixed(1) : "0.0"} ms`}
          />
          <MetricChip
            label={t(WEBUI.queryRecorder.maxMetricLabel)}
            value={`${data?.max_ms ?? 0} ms`}
          />
        </div>
        {histogram.length > 0 && (
          <div className="h-[260px] w-full">
            <AnalyticsChart
              label={t(WEBUI.queryRecorder.latencyTab)}
              option={distributionOption(
                histogram.map((row) => ({ key: row.label, count: row.count })),
                t(WEBUI.queryRecorder.countColumn),
                "bar",
              )}
              height={260}
            />
          </div>
        )}
        <div>
          <div className="mb-2 flex items-center gap-2 text-xs text-muted-foreground">
            <ServerCrash className="h-3.5 w-3.5" />
            {t(WEBUI.queryRecorder.slowQueriesHeader)}
          </div>
          <div className="overflow-hidden rounded-md border">
            <Table className="min-w-[600px]">
              <TableHeader>
                <TableRow className="bg-muted/30 hover:bg-muted/30">
                  <TableHead>QNAME</TableHead>
                  <TableHead>{t(WEBUI.queryRecorder.countColumn)}</TableHead>
                  <TableHead>
                    {t(WEBUI.queryRecorder.avgElapsedColumn)}
                  </TableHead>
                  <TableHead>
                    {t(WEBUI.queryRecorder.maxElapsedColumn)}
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {slowTop.map((row) => (
                  <TableRow key={row.qname}>
                    <TableCell className="font-mono">{row.qname}</TableCell>
                    <TableCell className="font-mono">{row.count}</TableCell>
                    <TableCell className="font-mono">
                      {row.avg_ms.toFixed(1)} ms
                    </TableCell>
                    <TableCell className="font-mono">{row.max_ms} ms</TableCell>
                  </TableRow>
                ))}
                {!slowTop.length && (
                  <TableRow>
                    <TableCell
                      colSpan={4}
                      className="h-20 text-center text-muted-foreground"
                    >
                      {loading
                        ? t(WEBUI.queryRecorder.loadingSlowQueries)
                        : t(WEBUI.queryRecorder.noData)}
                    </TableCell>
                  </TableRow>
                )}
              </TableBody>
            </Table>
          </div>
          {hasMoreSlowRows && (
            <Button
              variant="outline"
              size="sm"
              className="mt-3"
              disabled={loading}
              onClick={() => void load(slowLimit + TOP_PAGE_SIZE)}
            >
              {t(WEBUI.common.loadMore)}
            </Button>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

function TimeseriesCard({
  tag,
  filters,
  range,
  onRefresh,
}: {
  tag: string;
  filters: QueryRecordFilters;
  range: InsightsRangeKey;
  onRefresh: () => void;
}) {
  const { locale, t } = useI18n();
  const [zoomVersion, setZoomVersion] = useState(0);
  const [data, setData] = useState<QueryRecorderTimeseriesResponse | null>(
    null,
  );
  const { bucket, buckets } = queryRecorderRange(range, filters.untilMs ?? 0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const abortRef = useRef<AbortController | null>(null);

  const load = useCallback(async () => {
    abortRef.current?.abort();
    const controller = new AbortController();
    abortRef.current = controller;
    setLoading(true);
    setError(null);
    try {
      const response = await fetchQueryRecorderTimeseries(tag, {
        ...filters,
        bucket,
        buckets,
        signal: controller.signal,
      });
      if (controller.signal.aborted || abortRef.current !== controller) {
        return;
      }
      setData(response);
    } catch (err) {
      if (controller.signal.aborted || abortRef.current !== controller) {
        return;
      }
      setError(
        err instanceof Error
          ? err.message
          : t(WEBUI.queryRecorder.readTrendFailed),
      );
    } finally {
      if (abortRef.current === controller) {
        abortRef.current = null;
        setLoading(false);
      }
    }
  }, [tag, filters, bucket, buckets, t]);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      void load();
    }, 0);
    return () => {
      window.clearTimeout(timer);
      abortRef.current?.abort();
    };
  }, [load]);

  const points = data?.points ?? [];
  const option = useMemo(
    () =>
      timeseriesOption(
        data?.points ?? [],
        {
          total: t(WEBUI.queryRecorder.totalQueriesSeries),
          errors: t(WEBUI.queryRecorder.errorSeries),
          missing: t(WEBUI.queryRecorder.noResponseSeries),
          average: t(WEBUI.queryRecorder.avgMetricLabel),
        },
        locale,
      ),
    [data, t, locale],
  );

  return (
    <Card className="mt-3">
      <CardHeader className="grid gap-3 p-4 pb-2 sm:grid-cols-[1fr_auto] sm:items-center">
        <div className="min-w-0">
          <CardTitle className="flex items-center gap-2 text-sm">
            <TrendingUp className="h-4 w-4 text-primary" />
            {t(WEBUI.queryRecorder.trendTitle)}
          </CardTitle>
          <div className="mt-2 flex flex-wrap gap-2 text-xs text-muted-foreground">
            <span className="rounded-full border bg-muted/30 px-2 py-0.5">
              {t(WEBUI.queryRecorder.sampleCount, {
                count: data?.sample_size ?? 0,
              })}
            </span>
            <span className="rounded-full border bg-muted/30 px-2 py-0.5">
              {t(
                {
                  minute: WEBUI.queryRecorder.bucketSizeMinute,
                  hour: WEBUI.queryRecorder.bucketSizeHour,
                  day: WEBUI.queryRecorder.bucketSizeDay,
                  month: WEBUI.queryRecorder.bucketSizeMonth,
                }[bucket],
              )}{" "}
              · UTC
            </span>
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={() => setZoomVersion((value) => value + 1)}
          >
            {t(WEBUI.queryRecorder.resetZoom)}
          </Button>
          <Button
            variant="outline"
            size="sm"
            disabled={loading}
            onClick={onRefresh}
          >
            <RefreshCw className="h-4 w-4" />
            {t(WEBUI.common.refresh)}
          </Button>
        </div>
      </CardHeader>
      <CardContent className="space-y-4 p-4 pt-0">
        {error && (
          <div className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
            {error}
          </div>
        )}
        <p className="text-xs text-muted-foreground">
          {t(WEBUI.queryRecorder.chartInteractionHint)}
        </p>
        <p className="text-xs text-muted-foreground">
          {t(WEBUI.queryRecorder.retainedHistoryHint)}
        </p>
        {points.length > 0 ? (
          <div className="min-w-0 w-full">
            <AnalyticsChart
              key={zoomVersion}
              label={t(WEBUI.queryRecorder.trendTitle)}
              option={option}
              height={360}
            />
          </div>
        ) : (
          <div className="rounded-md border bg-muted/20 px-3 py-8 text-center text-sm text-muted-foreground">
            {loading
              ? t(WEBUI.queryRecorder.loadingTrend)
              : t(WEBUI.queryRecorder.noData)}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function MetricChip({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-md border bg-muted/20 p-2">
      <div className="text-[10px] uppercase tracking-wide text-muted-foreground">
        {label}
      </div>
      <div className="font-mono text-sm">{value}</div>
    </div>
  );
}

function AnswerPreviewCell({ record }: { record: QueryRecordRow }) {
  const preview = record.answer_preview ?? [];
  if (preview.length === 0) {
    return <span className="text-muted-foreground">-</span>;
  }

  const title = preview
    .map(
      (answer) =>
        `${answer.name} ${answer.rr_type} ${answer.payload_text}${answer.truncated ? "…" : ""}`,
    )
    .join("\n");

  return (
    <div className="min-w-0 space-y-1" title={title}>
      {preview.slice(0, 2).map((answer, index) => (
        <div
          key={`${answer.name}-${answer.rr_type}-${answer.payload_text}-${index}`}
          className="flex min-w-0 items-center gap-1.5"
        >
          <Badge
            variant="outline"
            className="shrink-0 px-1 py-0 font-mono text-[10px]"
          >
            {answer.rr_type}
          </Badge>
          <span className="min-w-0 truncate font-mono text-xs">
            {answer.payload_text || "-"}
            {answer.truncated && "…"}
          </span>
        </div>
      ))}
      {preview.length > 2 && (
        <span className="block text-[10px] text-muted-foreground">
          +{preview.length - 2}
        </span>
      )}
    </div>
  );
}

function FilterField({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <div className="grid gap-1 text-xs text-muted-foreground">
      <span>{label}</span>
      {children}
    </div>
  );
}

function RecordDetailDialog({
  record,
  onClose,
}: {
  record: QueryRecordDetail | null;
  onClose: () => void;
}) {
  const { locale, t } = useI18n();
  const dependencyGraph = useAppStore((state) => state.dependencyGraph);
  const plugins = useAppStore((state) => state.plugins);

  return (
    <DnsRecordDetailDialog
      open={Boolean(record)}
      onOpenChange={(open) => !open && onClose()}
      title={t(WEBUI.queryRecorder.detailTitle, { id: record?.id ?? "" })}
      subtitle={
        record ? formatFullTime(record.created_at_ms, locale) : undefined
      }
      status={record ? queryStatusBadge(record) : undefined}
      summaryItems={
        record
          ? [
              {
                label: t(WEBUI.queryRecorder.clientLabel),
                value: record.client_ip,
                mono: true,
              },
              {
                label: t(WEBUI.queryRecorder.requestIdLabel),
                value: String(record.request_id),
                mono: true,
              },
              {
                label: t(WEBUI.queryRecorder.elapsedLabel),
                value: `${record.elapsed_ms}ms`,
                mono: true,
              },
              { label: "RCODE", value: record.rcode ?? "-", mono: true },
              {
                label: (
                  <span className="inline-flex items-center gap-1">
                    {t(WEBUI.dnsRecord.responseRecordsLabel)}
                    <Popover>
                      <PopoverTrigger asChild>
                        <button
                          type="button"
                          className="inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none"
                          aria-label={t(WEBUI.dnsRecord.responseRecordsAria)}
                        >
                          <Info className="h-3.5 w-3.5" />
                        </button>
                      </PopoverTrigger>
                      <PopoverContent
                        side="top"
                        className="w-auto max-w-[16rem] p-2 text-xs"
                      >
                        {t(WEBUI.dnsRecord.responseRecordsTooltip)}
                      </PopoverContent>
                    </Popover>
                  </span>
                ),
                value: `${record.answer_count} / ${record.authority_count} / ${record.additional_count}`,
                mono: true,
              },
              {
                label: t(WEBUI.queryRecorder.reqFlagsLabel),
                value: `RD=${flag(record.req_rd)} CD=${flag(record.req_cd)} AD=${flag(record.req_ad)}`,
                mono: true,
                wide: true,
              },
              {
                label: t(WEBUI.queryRecorder.respFlagsLabel),
                value: record.has_response
                  ? `AA=${flag(record.resp_aa)} TC=${flag(record.resp_tc)} RA=${flag(record.resp_ra)} AD=${flag(record.resp_ad)} CD=${flag(record.resp_cd)}`
                  : "-",
                mono: true,
                wide: true,
              },
            ]
          : []
      }
      questions={record?.questions_json}
      sections={
        record
          ? [
              {
                title: t(WEBUI.dnsRecord.answerSection),
                records: record.answers_json,
                emptyLabel: t(WEBUI.dnsRecord.emptyAnswer),
              },
              {
                title: t(WEBUI.dnsRecord.authoritySection),
                records: record.authorities_json,
                emptyLabel: t(WEBUI.dnsRecord.emptyAuthority),
              },
              {
                title: t(WEBUI.dnsRecord.additionalSection),
                records: record.additionals_json,
                emptyLabel: t(WEBUI.dnsRecord.emptyAdditional),
              },
              {
                title: t(WEBUI.dnsRecord.signatureSection),
                records: record.signature_json,
                emptyLabel: t(WEBUI.dnsRecord.emptySignature),
              },
            ]
          : []
      }
      error={record?.error ?? null}
      bottomBlocks={
        record
          ? [
              {
                title: t(WEBUI.queryRecorder.executionFlowTitle),
                children: (
                  <QueryRecordFlowCanvas
                    record={record}
                    dependencyGraph={dependencyGraph}
                    plugins={plugins}
                  />
                ),
              },
            ]
          : []
      }
      wide
    />
  );
}

function queryStatusBadge(record: QueryRecordRow | QueryRecordDetail) {
  if (record.error) {
    return <Badge variant="destructive">ERR</Badge>;
  }
  if (record.rcode?.toLowerCase() === "no error") {
    return <Badge variant="secondary">No Error</Badge>;
  }
  return <Badge variant="outline">{record.rcode ?? "-"}</Badge>;
}

function queryElapsedClassName(elapsedMs: number) {
  if (elapsedMs < 20) {
    return "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300";
  }
  if (elapsedMs < 100) {
    return "border-sky-500/30 bg-sky-500/10 text-sky-700 dark:text-sky-300";
  }
  if (elapsedMs < 300) {
    return "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300";
  }
  return "border-rose-500/30 bg-rose-500/10 text-rose-700 dark:text-rose-300";
}

function queryElapsedTitle(elapsedMs: number, t: TFn) {
  if (elapsedMs < 20) return t(WEBUI.queryRecorder.latencyFast);
  if (elapsedMs < 100) return t(WEBUI.queryRecorder.latencyNormal);
  if (elapsedMs < 300) return t(WEBUI.queryRecorder.latencySlow);
  return t(WEBUI.queryRecorder.latencyVerySlow);
}

function filtersFromForm(form: QueryRecordFilterForm): QueryRecordFilters {
  return {
    search: optionalTrimmed(form.search),
    qtype: form.qtype === "all" ? undefined : form.qtype,
    rcode: form.rcode === "all" ? undefined : form.rcode,
    status: form.status === "all" ? undefined : form.status,
    sinceMs: parseLocalDateTime(form.sinceLocal),
    untilMs: parseLocalDateTime(form.untilLocal),
  };
}

function optionalTrimmed(value: string) {
  const trimmed = value.trim();
  return trimmed ? trimmed : undefined;
}

function parseLocalDateTime(value: string) {
  if (!value) return undefined;
  const ms = new Date(value).getTime();
  return Number.isFinite(ms) ? ms : undefined;
}

type SseEvent = {
  event: string;
  data: string;
};

function parseSseEvent(chunk: string): SseEvent {
  const data: string[] = [];
  let event = "message";
  for (const rawLine of chunk.split("\n")) {
    const line = rawLine.endsWith("\r") ? rawLine.slice(0, -1) : rawLine;
    if (!line || line.startsWith(":")) continue;

    const separator = line.indexOf(":");
    const field = separator === -1 ? line : line.slice(0, separator);
    let value = separator === -1 ? "" : line.slice(separator + 1);
    if (value.startsWith(" ")) value = value.slice(1);

    if (field === "event") {
      event = value || "message";
    } else if (field === "data") {
      data.push(value);
    }
  }
  return { event, data: data.join("\n") };
}

function parseSseErrorMessage(data: string, t: TFn) {
  try {
    const parsed = JSON.parse(data) as unknown;
    if (
      parsed &&
      typeof parsed === "object" &&
      "message" in parsed &&
      typeof parsed.message === "string"
    ) {
      return parsed.message;
    }
  } catch {
    // Fall through to the raw server payload preview.
  }
  return t(WEBUI.queryRecorder.streamErrorPrefix, { data: truncateText(data) });
}

function parseStreamRecord(data: string): QueryRecordDetail | null {
  try {
    const parsed = JSON.parse(data) as unknown;
    return isQueryRecordDetail(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

function isQueryRecordDetail(value: unknown): value is QueryRecordDetail {
  if (!value || typeof value !== "object") return false;
  const record = value as Partial<QueryRecordDetail>;
  return (
    typeof record.id === "number" &&
    typeof record.created_at_ms === "number" &&
    typeof record.client_ip === "string" &&
    Array.isArray(record.questions_json) &&
    Array.isArray(record.steps)
  );
}

function recordMatchesFilters(
  record: QueryRecordRow | QueryRecordDetail,
  filters: QueryRecordFilters,
) {
  if (filters.sinceMs !== undefined && record.created_at_ms < filters.sinceMs) {
    return false;
  }
  if (filters.untilMs !== undefined && record.created_at_ms > filters.untilMs) {
    return false;
  }
  if (filters.search) {
    const needle = filters.search.toLowerCase();
    const matchesQuestion = record.questions_json.some((question) =>
      question.name.toLowerCase().includes(needle),
    );
    const matchesClient = record.client_ip.toLowerCase().includes(needle);
    if (!matchesQuestion && !matchesClient) return false;
  }
  if (filters.qname) {
    const needle = filters.qname.toLowerCase();
    if (
      !record.questions_json.some((question) =>
        question.name.toLowerCase().includes(needle),
      )
    ) {
      return false;
    }
  }
  if (filters.qtype) {
    const qtype = filters.qtype.toLowerCase();
    if (
      !record.questions_json.some(
        (question) => question.qtype.toLowerCase() === qtype,
      )
    ) {
      return false;
    }
  }
  if (
    filters.clientIp &&
    !record.client_ip.toLowerCase().includes(filters.clientIp.toLowerCase())
  ) {
    return false;
  }
  if (
    filters.rcode &&
    (record.rcode ?? "").toLowerCase() !== filters.rcode.toLowerCase()
  ) {
    return false;
  }
  if (filters.status === "error" && !record.error) return false;
  if (filters.status === "has_response" && !record.has_response) return false;
  if (
    filters.status === "no_response" &&
    (record.error || record.has_response)
  ) {
    return false;
  }
  if (filters.matcherTag) {
    const steps = (record as Partial<QueryRecordDetail>).steps;
    if (
      !steps?.some(
        (step) =>
          step.kind === "matcher" &&
          (step.outcome === "matched" ||
            step.outcome === "always_true_matched" ||
            step.outcome === "always_false_matched") &&
          step.tag === filters.matcherTag,
      )
    ) {
      return false;
    }
  }
  return true;
}

function countActiveFilters(filters: QueryRecordFilters) {
  return [
    filters.search,
    filters.qname,
    filters.qtype,
    filters.clientIp,
    filters.rcode,
    filters.status,
    filters.sinceMs,
    filters.untilMs,
    filters.matcherTag,
  ].filter((value) => value !== undefined && value !== "").length;
}

function safeRatio(numerator: number, denominator: number) {
  if (denominator <= 0) return 0;
  return numerator / denominator;
}

function formatPercent(value: number) {
  return `${(value * 100).toFixed(1)}%`;
}

function flag(value: unknown) {
  if (typeof value !== "boolean") return "-";
  return value ? "1" : "0";
}

function formatTime(ms: number, locale: ReturnType<typeof useI18n>["locale"]) {
  return new Date(ms).toLocaleTimeString(locale, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function formatFullTime(
  ms: number,
  locale: ReturnType<typeof useI18n>["locale"],
) {
  return new Date(ms).toLocaleString(locale);
}

function truncateText(value: string, max = 160) {
  const singleLine = value.replace(/\s+/g, " ").trim();
  return singleLine.length > max
    ? `${singleLine.slice(0, Math.max(0, max - 1))}…`
    : singleLine;
}

function formatQuestion(record: QueryRecordRow) {
  const first = record.questions_json[0];
  if (!first) return "-";
  return `${first.name} ${first.qtype}`;
}

function formatQuestionName(record: QueryRecordRow) {
  const first = record.questions_json[0];
  return first?.name ?? "-";
}

function formatQuestionType(record: QueryRecordRow) {
  const first = record.questions_json[0];
  return first?.qtype ?? "-";
}

export const queryRecorderPlugin: PluginComponentDefinition = {
  Detail: QueryRecorderDetail,
};
