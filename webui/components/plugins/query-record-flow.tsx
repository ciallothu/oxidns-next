"use client";

import { useMemo, type ReactNode } from "react";
import {
  AlertTriangle,
  Check,
  Circle,
  CornerDownRight,
  GitBranch,
  Play,
  X,
} from "lucide-react";
import { Badge } from "@/components/ui/badge";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import type { PluginInstance } from "@/lib/types";
import type {
  DependencyGraphReport,
  QueryRecordDetail,
  QueryRecorderStep,
  SequenceFlowExpression,
  SequenceFlowReport,
  SequenceFlowRule,
} from "@/lib/oxidns-next-api";
import { cn } from "@/lib/utils";
import {
  pluginKindAccentHex,
  pluginKindBadgeOutlineClass,
  pluginKindIconBgClass,
} from "@/components/plugins/display";
import { WEBUI } from "@/lib/i18n";
import { useI18n } from "@/lib/i18n/provider";

type MatchStatus = "matched" | "not_matched" | "unchecked";
type ActionStatus =
  | "not_executed"
  | "entered"
  | "next"
  | "stop"
  | "return"
  | "error";

interface RuntimeIndexes {
  matchers: Map<string, QueryRecorderStep[]>;
  actions: Map<string, QueryRecorderStep[]>;
  stepsBySequence: Map<string, QueryRecorderStep[]>;
}

interface SequenceRuntime {
  flow: SequenceFlowReport;
  steps: QueryRecorderStep[];
}

type FlowModel =
  | { mode: "empty" }
  | {
      mode: "fallback";
      reason: string;
      steps: QueryRecorderStep[];
    }
  | {
      mode: "flow";
      sequences: SequenceRuntime[];
      runtime: RuntimeIndexes;
      pluginByTag: Map<string, PluginInstance>;
    };

type TFn = (
  key: string,
  params?: Record<string, string | number | boolean | null | undefined>,
) => string;

export function QueryRecordFlowCanvas({
  record,
  dependencyGraph,
  plugins,
}: {
  record: QueryRecordDetail;
  dependencyGraph: DependencyGraphReport | null;
  plugins: PluginInstance[];
}) {
  const { t } = useI18n();
  const model = useMemo(
    () => buildFlowModel(record, dependencyGraph, plugins, t),
    [dependencyGraph, plugins, record, t],
  );

  if (model.mode === "empty") {
    return (
      <div className="flex min-h-36 items-center justify-center rounded-md border border-dashed bg-muted/10 px-4 text-center text-sm text-muted-foreground">
        {t(WEBUI.queryRecordFlow.noSequenceEvents)}
      </div>
    );
  }

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
        {model.mode === "flow" ? (
          <>
            <span className="rounded-full border bg-muted/30 px-2 py-0.5">
              {t(WEBUI.queryRecordFlow.sequenceCount, {
                count: model.sequences.length,
              })}
            </span>
            <span className="rounded-full border bg-muted/30 px-2 py-0.5">
              {t(WEBUI.queryRecordFlow.eventCount, {
                count: record.steps.length,
              })}
            </span>
          </>
        ) : (
          <>
            <span className="inline-flex items-center gap-1.5 rounded-full border border-amber-300/70 bg-amber-500/10 px-2 py-0.5 text-amber-700 dark:text-amber-300">
              <AlertTriangle aria-hidden="true" className="h-3 w-3" />
              {model.reason}
            </span>
            <span className="rounded-full border bg-muted/30 px-2 py-0.5">
              {t(WEBUI.queryRecordFlow.rawEventOrder)}
            </span>
            <span className="rounded-full border bg-muted/30 px-2 py-0.5">
              {t(WEBUI.queryRecordFlow.eventCount, {
                count: record.steps.length,
              })}
            </span>
          </>
        )}
        <span className="rounded-full border border-emerald-500/30 bg-emerald-500/10 px-2 py-0.5 text-emerald-700 dark:text-emerald-300">
          {t(WEBUI.queryRecordFlow.matched)}
        </span>
        <span className="rounded-full border border-rose-500/30 bg-rose-500/10 px-2 py-0.5 text-rose-700 dark:text-rose-300">
          {t(WEBUI.queryRecordFlow.notMatched)}
        </span>
        <span className="rounded-full border bg-muted/30 px-2 py-0.5">
          {t(WEBUI.queryRecordFlow.unchecked)}
        </span>
      </div>

      <div className="touch-pan-y rounded-md border bg-muted/10 p-2 sm:p-3">
        {model.mode === "flow" ? (
          <ol className="mx-auto max-w-5xl space-y-3">
            {model.sequences.map((sequence, index) => (
              <li
                key={sequence.flow.tag}
                className="relative pl-4 sm:pl-6"
              >
                {index < model.sequences.length - 1 && (
                  <span
                    aria-hidden="true"
                    className="absolute top-4 bottom-[-1rem] left-[0.3rem] border-l border-dashed border-sky-500/35 sm:left-[0.55rem]"
                  />
                )}
                <span
                  aria-hidden="true"
                  className="absolute top-4 left-0 h-2.5 w-2.5 rounded-full border-2 border-sky-500 bg-background sm:left-1"
                />
                <QuerySequencePanel
                  sequence={sequence}
                  runtime={model.runtime}
                  pluginByTag={model.pluginByTag}
                />
              </li>
            ))}
          </ol>
        ) : (
          <ol className="mx-auto max-w-3xl divide-y overflow-hidden rounded-md border bg-card/60">
            {model.steps.map((step) => (
              <li key={step.event_index}>
                <QueryStepRow step={step} />
              </li>
            ))}
          </ol>
        )}
      </div>
    </div>
  );
}

function buildFlowModel(
  record: QueryRecordDetail,
  dependencyGraph: DependencyGraphReport | null,
  plugins: PluginInstance[],
  t: TFn,
): FlowModel {
  const steps = record.steps ?? [];
  if (steps.length === 0) return { mode: "empty" };

  const runtime = buildRuntimeIndexes(steps);
  const pluginByTag = new Map(plugins.map((plugin) => [plugin.name, plugin]));
  const flowByTag = new Map(
    (dependencyGraph?.sequence_flows ?? []).map((flow) => [flow.tag, flow]),
  );

  if (!dependencyGraph || flowByTag.size === 0) {
    return {
      mode: "fallback",
      reason: t(WEBUI.queryRecordFlow.topologyUnavailable),
      steps,
    };
  }

  const sequenceTags = orderedSequenceTags(steps);
  const missing = sequenceTags.filter((tag) => !flowByTag.has(tag));
  if (missing.length > 0) {
    return {
      mode: "fallback",
      reason: t(WEBUI.queryRecordFlow.missingSequenceConfig, {
        names: missing.join(", "),
      }),
      steps,
    };
  }

  const sequences = sequenceTags.map((tag) => {
    const sequenceSteps = runtime.stepsBySequence.get(tag) ?? [];
    return {
      flow: flowByTag.get(tag)!,
      steps: sequenceSteps,
    };
  });

  return {
    mode: "flow",
    sequences,
    runtime,
    pluginByTag,
  };
}

function buildRuntimeIndexes(steps: QueryRecorderStep[]): RuntimeIndexes {
  const matchers = new Map<string, QueryRecorderStep[]>();
  const actions = new Map<string, QueryRecorderStep[]>();
  const stepsBySequence = new Map<string, QueryRecorderStep[]>();

  for (const step of steps) {
    pushMapValue(stepsBySequence, step.sequence_tag, step);
    if (typeof step.node_index !== "number" || !step.tag) continue;

    if (step.kind === "matcher") {
      pushMapValue(
        matchers,
        matcherKey(step.sequence_tag, step.node_index, step.tag),
        step,
      );
    }

    if (step.kind === "executor" || step.kind === "builtin") {
      pushMapValue(
        actions,
        actionKey(step.sequence_tag, step.node_index, step.kind, step.tag),
        step,
      );
    }
  }

  for (const entries of [
    ...matchers.values(),
    ...actions.values(),
    ...stepsBySequence.values(),
  ]) {
    entries.sort((a, b) => a.event_index - b.event_index);
  }

  return { matchers, actions, stepsBySequence };
}

function QuerySequencePanel({
  sequence,
  runtime,
  pluginByTag,
}: {
  sequence: SequenceRuntime;
  runtime: RuntimeIndexes;
  pluginByTag: Map<string, PluginInstance>;
}) {
  const { t } = useI18n();
  const flow = sequence.flow;
  const accent = pluginKindAccentHex("executor");
  const eventRange = eventRangeLabel(sequence.steps);

  return (
    <div
      className="overflow-hidden rounded-lg border bg-card shadow-sm"
      style={{ borderLeftColor: accent, borderLeftWidth: 4 }}
    >
      <div
        className="flex items-center gap-2 px-3 py-2.5"
        style={{ backgroundColor: `${accent}12` }}
      >
        <span
          className={cn(
            "shrink-0 rounded-full p-1",
            pluginKindIconBgClass("executor"),
          )}
        >
          <GitBranch aria-hidden="true" className="h-3.5 w-3.5" />
        </span>
        <div className="min-w-0 flex-1">
          <div className="truncate font-mono text-sm font-semibold">
            {flow.tag}
          </div>
          <div className="truncate font-mono text-[10px] text-muted-foreground">
            {t(WEBUI.queryRecordFlow.eventsRange, { range: eventRange })}
          </div>
        </div>
        <Badge variant="secondary" className="shrink-0 px-1.5 py-0 text-[10px]">
          {t(WEBUI.queryRecordFlow.rulesCount, { count: flow.rules.length })}
        </Badge>
      </div>

      <div className="border-t">
        {flow.rules.map((rule, index) => (
          <SequenceRuleRow
            key={rule.index}
            sequenceTag={flow.tag}
            rule={rule}
            ruleOffset={index}
            isLast={index === flow.rules.length - 1}
            runtime={runtime}
            pluginByTag={pluginByTag}
          />
        ))}
      </div>
    </div>
  );
}

function SequenceRuleRow({
  sequenceTag,
  rule,
  ruleOffset,
  isLast,
  runtime,
  pluginByTag,
}: {
  sequenceTag: string;
  rule: SequenceFlowRule;
  ruleOffset: number;
  isLast: boolean;
  runtime: RuntimeIndexes;
  pluginByTag: Map<string, PluginInstance>;
}) {
  const { t } = useI18n();
  const matchStatuses = rule.matches.map((expression, matchIndex) =>
    getMatchStatus(sequenceTag, rule.index, matchIndex, expression, runtime),
  );
  const actionStatus = getActionStatus(
    sequenceTag,
    rule.index,
    rule.exec,
    runtime,
  );
  const missed = matchStatuses.some(
    (status) => status.status === "not_matched",
  );
  const ran = actionStatus.status !== "not_executed";

  return (
    <div
      className={cn(
        "grid grid-cols-[2.35rem_minmax(0,1fr)] items-center gap-x-2 gap-y-1.5 px-2 py-2 transition-colors sm:grid-cols-[2.35rem_minmax(0,1fr)_1rem_minmax(9rem,0.78fr)] sm:px-3",
        !isLast && "border-b border-dashed",
        ruleOffset % 2 === 1 && "bg-muted/15",
        missed && "bg-rose-500/5",
        ran && !missed && "bg-sky-500/5",
      )}
    >
      <div className="flex flex-col items-center gap-0.5">
        <span
          className={cn(
            "rounded px-1.5 py-px text-center font-mono text-[10px] font-semibold",
            ran
              ? "bg-sky-100 text-sky-700 dark:bg-sky-900/60 dark:text-sky-200"
              : missed
                ? "bg-rose-100 text-rose-700 dark:bg-rose-950 dark:text-rose-300"
                : "bg-muted text-muted-foreground",
          )}
        >
          #{rule.index}
        </span>
      </div>

      <div className="flex min-w-0 flex-wrap items-center gap-1.5">
        {rule.matches.length === 0 ? (
          <span className="rounded border border-emerald-500/25 bg-emerald-500/10 px-1.5 py-0.5 text-[10px] text-emerald-700 dark:text-emerald-300">
            {t(WEBUI.queryRecordFlow.alwaysMatch)}
          </span>
        ) : (
          rule.matches.map((expression, matchIndex) => (
            <MatcherStatusChip
              key={`${expression.field}-${expression.raw}-${matchIndex}`}
              sequenceTag={sequenceTag}
              ruleIndex={rule.index}
              matchIndex={matchIndex}
              expression={expression}
              runtime={runtime}
              pluginByTag={pluginByTag}
            />
          ))
        )}
      </div>

      <CornerDownRight
        aria-hidden="true"
        className={cn(
          "hidden h-3.5 w-3.5 shrink-0 sm:block",
          ran ? "text-sky-500" : "text-muted-foreground/40",
        )}
      />

      <div className="col-start-2 flex min-w-0 items-center overflow-hidden sm:col-start-auto">
        <ActionStatusChip
          sequenceTag={sequenceTag}
          ruleIndex={rule.index}
          expression={rule.exec}
          runtime={runtime}
          pluginByTag={pluginByTag}
        />
      </div>
    </div>
  );
}

function MatcherStatusChip({
  sequenceTag,
  ruleIndex,
  matchIndex,
  expression,
  runtime,
  pluginByTag,
}: {
  sequenceTag: string;
  ruleIndex: number;
  matchIndex: number;
  expression: SequenceFlowExpression;
  runtime: RuntimeIndexes;
  pluginByTag: Map<string, PluginInstance>;
}) {
  const { t } = useI18n();
  const result = getMatchStatus(
    sequenceTag,
    ruleIndex,
    matchIndex,
    expression,
    runtime,
  );
  const plugin =
    result.runtimeTag === undefined
      ? undefined
      : pluginByTag.get(result.runtimeTag);

  return (
    <StatusPopover
      title={sequenceExpressionLabel(expression)}
      events={result.events}
      fallback={t(WEBUI.queryRecordFlow.fieldFallback, {
        field: expression.field,
      })}
    >
      <span
        className={cn(
          "inline-flex max-w-[12rem] items-center gap-1.5 rounded border px-2 py-1 text-left text-[10px] transition-colors",
          matchStatusClass(result.status),
        )}
      >
        {expression.inverted && <InvertMark />}
        {matchStatusIcon(result.status)}
        <span className="min-w-0 flex-1 truncate font-mono">
          {sequenceExpressionLabel(expression)}
        </span>
        {plugin && (
          <span
            className={cn(
              "shrink-0 rounded px-1 py-px text-[9px]",
              pluginKindBadgeOutlineClass(plugin.type),
            )}
          >
            {plugin.pluginKind}
          </span>
        )}
        <StatusSuffix
          events={result.events}
          label={matchStatusLabel(result.status, t)}
        />
      </span>
    </StatusPopover>
  );
}

function ActionStatusChip({
  sequenceTag,
  ruleIndex,
  expression,
  runtime,
  pluginByTag,
}: {
  sequenceTag: string;
  ruleIndex: number;
  expression: SequenceFlowExpression | undefined;
  runtime: RuntimeIndexes;
  pluginByTag: Map<string, PluginInstance>;
}) {
  const { t } = useI18n();
  const result = getActionStatus(sequenceTag, ruleIndex, expression, runtime);
  const plugin =
    result.runtimeTag === undefined
      ? undefined
      : pluginByTag.get(result.runtimeTag);

  return (
    <StatusPopover
      title={sequenceActionLabel(expression, t)}
      events={result.events}
      fallback={
        expression
          ? t(WEBUI.queryRecordFlow.fieldFallback, {
              field: expression.field,
            })
          : t(WEBUI.queryRecordFlow.noExecFallback)
      }
    >
      <span
        className={cn(
          "inline-flex max-w-full items-center gap-1.5 rounded border px-2 py-1 text-left text-[10px] transition-colors",
          actionStatusClass(result.status),
        )}
      >
        {actionStatusIcon(result.status)}
        <span className="min-w-0 flex-1 truncate font-mono">
          {sequenceActionLabel(expression, t)}
        </span>
        {plugin && (
          <span
            className={cn(
              "shrink-0 rounded px-1 py-px text-[9px]",
              pluginKindBadgeOutlineClass(plugin.type),
            )}
          >
            {plugin.pluginKind}
          </span>
        )}
        <StatusSuffix
          events={result.events}
          label={actionStatusLabel(result.status, t)}
        />
      </span>
    </StatusPopover>
  );
}

function StatusPopover({
  title,
  events,
  fallback,
  children,
}: {
  title: string;
  events: QueryRecorderStep[];
  fallback: string;
  children: ReactNode;
}) {
  const { t } = useI18n();
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button type="button" className="min-w-0 max-w-full text-left">
          {children}
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-80 text-xs" align="start">
        <div className="space-y-2">
          <div className="font-medium">{title}</div>
          <div className="font-mono text-[10px] text-muted-foreground">
            {fallback}
          </div>
          {events.length > 0 ? (
            <div className="max-h-44 space-y-1 overflow-auto">
              {events.map((event) => (
                <div
                  key={event.event_index}
                  className="grid grid-cols-[3rem_1fr_auto] gap-2 rounded border bg-muted/20 px-2 py-1 font-mono text-[10px]"
                >
                  <span className="text-muted-foreground">
                    #{event.event_index}
                  </span>
                  <span className="min-w-0 truncate">
                    {event.sequence_tag}
                    {typeof event.node_index === "number"
                      ? ` / ${event.node_index}`
                      : ""}
                  </span>
                  <span>{event.outcome}</span>
                </div>
              ))}
            </div>
          ) : (
            <div className="rounded border border-dashed bg-muted/10 px-2 py-3 text-muted-foreground">
              {t(WEBUI.queryRecordFlow.noNodeEvents)}
            </div>
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
}

function StatusSuffix({
  events,
  label,
}: {
  events: QueryRecorderStep[];
  label: string;
}) {
  return (
    <span className="inline-flex shrink-0 items-center gap-1">
      <span className="rounded bg-background/70 px-1 py-px text-[9px]">
        {label}
      </span>
      {events.length > 1 && (
        <span className="rounded bg-background/70 px-1 py-px font-mono text-[9px]">
          x{events.length}
        </span>
      )}
    </span>
  );
}

function QueryStepRow({ step }: { step: QueryRecorderStep }) {
  return (
    <div className="grid min-w-0 gap-2 px-3 py-2.5 sm:grid-cols-[4rem_minmax(0,1fr)_auto] sm:items-center">
      <div className="flex min-w-0 items-center justify-between gap-2 sm:block">
        <span className="font-mono text-xs text-muted-foreground">
          #{step.event_index}
        </span>
        <Badge
          variant="outline"
          className="font-mono text-[10px] sm:hidden"
        >
          {step.outcome}
        </Badge>
      </div>
      <div className="min-w-0">
        <div className="truncate font-mono text-xs">
          {step.sequence_tag}
          {typeof step.node_index === "number" ? ` / ${step.node_index}` : ""}
        </div>
        <div className="mt-1 flex min-w-0 items-center gap-1.5">
          <span
            className={cn(
              "rounded px-1.5 py-0.5 font-mono text-[10px]",
              step.kind === "matcher"
                ? "bg-amber-500/10 text-amber-700 dark:text-amber-300"
                : "bg-sky-500/10 text-sky-700 dark:text-sky-300",
            )}
          >
            {step.kind}
          </span>
          <span className="min-w-0 truncate font-mono text-[10px] text-muted-foreground">
            {step.tag ?? "-"}
          </span>
        </div>
      </div>
      <Badge
        variant="outline"
        className="hidden font-mono text-[10px] sm:inline-flex"
      >
        {step.outcome}
      </Badge>
    </div>
  );
}

function getMatchStatus(
  sequenceTag: string,
  ruleIndex: number,
  matchIndex: number,
  expression: SequenceFlowExpression,
  runtime: RuntimeIndexes,
) {
  const runtimeTag = matcherRuntimeTag(
    sequenceTag,
    ruleIndex,
    matchIndex,
    expression,
  );
  const events = runtimeTag
    ? (runtime.matchers.get(matcherKey(sequenceTag, ruleIndex, runtimeTag)) ??
      [])
    : [];
  const last = events[events.length - 1];
  const status: MatchStatus =
    last?.outcome === "matched"
      ? "matched"
      : last?.outcome === "not_matched"
        ? "not_matched"
        : "unchecked";
  return { status, events, runtimeTag };
}

function getActionStatus(
  sequenceTag: string,
  ruleIndex: number,
  expression: SequenceFlowExpression | undefined,
  runtime: RuntimeIndexes,
) {
  const target = actionRuntimeTarget(sequenceTag, ruleIndex, expression);
  const events = target
    ? (runtime.actions.get(
        actionKey(sequenceTag, ruleIndex, target.kind, target.tag),
      ) ?? [])
    : [];
  return {
    status: reduceActionStatus(events),
    events,
    runtimeTag: target?.tag,
  };
}

function reduceActionStatus(events: QueryRecorderStep[]): ActionStatus {
  if (events.length === 0) return "not_executed";
  if (events.some((event) => event.outcome === "error")) return "error";
  if (events.some((event) => event.outcome === "stop")) return "stop";
  if (events.some((event) => event.outcome === "return")) return "return";
  if (events.some((event) => event.outcome === "next")) return "next";
  if (events.some((event) => event.outcome === "entered")) return "entered";
  return "not_executed";
}

function matcherRuntimeTag(
  sequenceTag: string,
  ruleIndex: number,
  matchIndex: number,
  expression: SequenceFlowExpression,
) {
  if (expression.target_tag) return expression.target_tag;
  if (expression.kind === "quick_setup" && expression.plugin_type) {
    return `@qs:match:${sequenceTag}:${ruleIndex}:${matchIndex}:${expression.plugin_type}`;
  }
  return undefined;
}

function actionRuntimeTarget(
  sequenceTag: string,
  ruleIndex: number,
  expression: SequenceFlowExpression | undefined,
) {
  if (!expression) return undefined;
  if (expression.kind === "builtin" && expression.builtin) {
    return { kind: "builtin", tag: expression.builtin };
  }
  if (expression.target_tag) {
    return { kind: "executor", tag: expression.target_tag };
  }
  if (expression.kind === "quick_setup" && expression.plugin_type) {
    return {
      kind: "executor",
      tag: `@qs:exec:${sequenceTag}:${ruleIndex}:${expression.plugin_type}`,
    };
  }
  return undefined;
}

function orderedSequenceTags(steps: QueryRecorderStep[]) {
  const seen = new Set<string>();
  const tags: string[] = [];
  for (const step of steps
    .slice()
    .sort((a, b) => a.event_index - b.event_index)) {
    if (seen.has(step.sequence_tag)) continue;
    seen.add(step.sequence_tag);
    tags.push(step.sequence_tag);
  }
  return tags;
}

function matcherKey(sequenceTag: string, ruleIndex: number, tag: string) {
  return `${sequenceTag}|${ruleIndex}|${tag}`;
}

function actionKey(
  sequenceTag: string,
  ruleIndex: number,
  kind: string,
  tag: string,
) {
  return `${sequenceTag}|${ruleIndex}|${kind}|${tag}`;
}

function pushMapValue<K, V>(map: Map<K, V[]>, key: K, value: V) {
  const values = map.get(key) ?? [];
  values.push(value);
  map.set(key, values);
}

function sequenceExpressionLabel(expression: SequenceFlowExpression) {
  const not = expression.inverted ? "!" : "";
  if (expression.kind === "quick_setup") {
    const param = expression.param
      ? ` ${compactText(expression.param, 18)}`
      : "";
    return `${not}quick(${expression.plugin_type ?? "?"})${param}`;
  }
  if (expression.target_tag) return `${not}$${expression.target_tag}`;
  return `${not}${compactText(expression.raw, 26)}`;
}

function sequenceActionLabel(
  expression: SequenceFlowExpression | undefined,
  t: TFn,
) {
  if (!expression) return t(WEBUI.queryRecordFlow.noExec);
  if (expression.kind === "builtin") {
    const param = expression.param
      ? ` ${compactText(expression.param, 18)}`
      : "";
    return `${expression.builtin ?? "builtin"}${param}`;
  }
  return sequenceExpressionLabel(expression);
}

function compactText(value: string, maxLength: number) {
  return value.length > maxLength ? `${value.slice(0, maxLength - 1)}…` : value;
}

function eventRangeLabel(steps: QueryRecorderStep[]) {
  if (steps.length === 0) return "-";
  const first = steps[0]?.event_index;
  const last = steps[steps.length - 1]?.event_index;
  return first === last ? `#${first}` : `#${first}-#${last}`;
}

function InvertMark() {
  const { t } = useI18n();
  return (
    <span
      aria-label={t(WEBUI.topology.invertLabel)}
      className="inline-flex h-4 w-4 shrink-0 items-center justify-center rounded border border-rose-400 bg-rose-100 font-mono text-[11px] font-bold leading-none text-rose-600 dark:border-rose-600 dark:bg-rose-950 dark:text-rose-400"
    >
      !
    </span>
  );
}

function matchStatusIcon(status: MatchStatus) {
  if (status === "matched") {
    return <Check aria-hidden="true" className="h-3 w-3 shrink-0" />;
  }
  if (status === "not_matched") {
    return <X aria-hidden="true" className="h-3 w-3 shrink-0" />;
  }
  return <Circle aria-hidden="true" className="h-3 w-3 shrink-0" />;
}

function actionStatusIcon(status: ActionStatus) {
  if (status === "error") {
    return (
      <AlertTriangle aria-hidden="true" className="h-3 w-3 shrink-0" />
    );
  }
  if (status === "not_executed") {
    return <Circle aria-hidden="true" className="h-3 w-3 shrink-0" />;
  }
  return <Play aria-hidden="true" className="h-3 w-3 shrink-0" />;
}

function matchStatusLabel(status: MatchStatus, t: TFn) {
  if (status === "matched") return t(WEBUI.queryRecordFlow.matched);
  if (status === "not_matched") return t(WEBUI.queryRecordFlow.notMatched);
  return t(WEBUI.queryRecordFlow.unchecked);
}

function actionStatusLabel(status: ActionStatus, t: TFn) {
  switch (status) {
    case "entered":
      return t(WEBUI.queryRecordFlow.entered);
    case "next":
      return t(WEBUI.queryRecordFlow.next);
    case "stop":
      return t(WEBUI.queryRecordFlow.stop);
    case "return":
      return t(WEBUI.queryRecordFlow.return);
    case "error":
      return t(WEBUI.queryRecordFlow.error);
    default:
      return t(WEBUI.queryRecordFlow.notExecuted);
  }
}

function matchStatusClass(status: MatchStatus) {
  if (status === "matched") {
    return "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 hover:border-emerald-500/60 dark:text-emerald-300";
  }
  if (status === "not_matched") {
    return "border-rose-500/30 bg-rose-500/10 text-rose-700 hover:border-rose-500/60 dark:text-rose-300";
  }
  return "border-border bg-muted/30 text-muted-foreground hover:border-primary/40";
}

function actionStatusClass(status: ActionStatus) {
  if (status === "error") {
    return "border-destructive/40 bg-destructive/10 text-destructive hover:border-destructive/70";
  }
  if (status === "stop" || status === "return") {
    return "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 hover:border-emerald-500/60 dark:text-emerald-300";
  }
  if (status === "next" || status === "entered") {
    return "border-sky-500/30 bg-sky-500/10 text-sky-700 hover:border-sky-500/60 dark:text-sky-300";
  }
  return "border-border bg-muted/30 text-muted-foreground hover:border-primary/40";
}
