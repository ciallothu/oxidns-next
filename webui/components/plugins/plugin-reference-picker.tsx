/*
 * SPDX-FileCopyrightText: 2025 Sven Shi
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

"use client";

import dynamic from "next/dynamic";
import { useRef, useState } from "react";
import type { WheelEvent, TouchEvent } from "react";
import { Plus, Search } from "lucide-react";
import {
  getPluginCatalogItem,
  renderPluginKindIcon,
} from "@/components/plugins/catalog";
import { pluginKindIconBgClass } from "@/components/plugins/display";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import type { PluginInstance, PluginType } from "@/lib/types";
import { isPluginKindSupported } from "@/lib/build-capabilities";
import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";
import { LOCALES, WEBUI } from "@/lib/i18n";
import {
  getPluginSearchText,
  pluginTypeLabel,
} from "@/lib/i18n/plugin-defined";
import { useI18n } from "@/lib/i18n/provider";

const CreatePluginDialog = dynamic(
  () =>
    import("@/components/plugins/create-plugin-dialog").then(
      (module) => module.CreatePluginDialog,
    ),
  { ssr: false },
);

interface PluginReferencePickerProps {
  plugins: PluginInstance[];
  value: string;
  referenceTypes?: PluginType[];
  referencePlugins?: string[];
  disabled?: boolean;
  placeholder?: string;
  className?: string;
  allowCreate?: boolean;
  createDescription?: string;
  onChange: (value: string) => void;
}

export function PluginReferencePicker({
  plugins,
  value,
  referenceTypes,
  referencePlugins,
  disabled = false,
  placeholder,
  className,
  allowCreate = false,
  createDescription,
  onChange,
}: PluginReferencePickerProps) {
  const { locale, t } = useI18n();
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [createOpen, setCreateOpen] = useState(false);
  const buildInfo = useAppStore((s) => s.buildInfo);
  const listRef = useRef<HTMLDivElement | null>(null);
  const touchYRef = useRef<number | null>(null);
  const normalizedValue = stripReferencePrefix(value);
  const normalizedSearch = search.trim().toLowerCase();
  const selectedPlugin = plugins.find(
    (plugin) => plugin.name === normalizedValue,
  );
  const resolvedPlaceholder =
    placeholder ?? t(WEBUI.plugins.referencePlaceholder);
  const selectedSupported = selectedPlugin
    ? isPluginKindSupported(
        buildInfo,
        selectedPlugin.type,
        selectedPlugin.pluginKind,
      )
    : true;
  const createType = referenceTypes?.[0];

  const filteredPlugins = plugins.filter((plugin) => {
    if (
      referenceTypes &&
      referenceTypes.length > 0 &&
      !referenceTypes.includes(plugin.type)
    ) {
      return false;
    }
    if (
      referencePlugins &&
      referencePlugins.length > 0 &&
      !referencePlugins.includes(plugin.pluginKind)
    ) {
      return false;
    }
    if (!normalizedSearch) return true;

    const definition = getPluginCatalogItem(plugin.pluginKind, locale);
    const baseDefinition = getPluginCatalogItem(plugin.pluginKind);
    return [
      plugin.name,
      plugin.pluginKind,
      plugin.type,
      pluginTypeLabel(plugin.type, locale),
      definition?.name,
      definition?.description,
      baseDefinition
        ? getPluginSearchText(baseDefinition, [...LOCALES])
        : undefined,
    ]
      .filter((text): text is string => Boolean(text))
      .join(" ")
      .toLowerCase()
      .includes(normalizedSearch);
  });

  return (
    <>
      <Popover open={open && !disabled} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <Button
            type="button"
            variant="outline"
            className={cn(
              "h-auto min-h-9 w-full min-w-0 flex-1 justify-between gap-2 bg-background px-2 py-1 font-normal text-foreground",
              className,
              selectedPlugin && "hover:bg-background/80",
            )}
            disabled={disabled}
          >
            {selectedPlugin ? (
              <PluginReferenceCompact
                plugin={selectedPlugin}
                supported={selectedSupported}
              />
            ) : normalizedValue ? (
              <span className="min-w-0 flex-1 truncate text-left font-mono text-xs">
                {normalizedValue}
              </span>
            ) : (
              <span className="text-xs text-muted-foreground">
                {resolvedPlaceholder}
              </span>
            )}
          </Button>
        </PopoverTrigger>
        <PopoverContent
          align="start"
          side="bottom"
          className="z-[1100] w-[26rem] max-w-[calc(100vw-3rem)] p-2"
        >
          <div className="relative">
            <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              placeholder={t(WEBUI.plugins.referenceSearchPlaceholder)}
              className="h-8 pl-9"
            />
          </div>
          <div
            ref={listRef}
            className="mt-2 max-h-64 space-y-1 overflow-y-auto overscroll-contain"
            onWheel={(event) => {
              scrollListByWheel(event, listRef.current);
            }}
            onTouchStart={(event) => {
              touchYRef.current = event.touches[0]?.clientY ?? null;
            }}
            onTouchMove={(event) => {
              scrollListByTouch(event, listRef.current, touchYRef);
            }}
          >
            {filteredPlugins.map((plugin) => (
              <PluginReferenceOption
                key={plugin.id}
                plugin={plugin}
                supported={isPluginKindSupported(
                  buildInfo,
                  plugin.type,
                  plugin.pluginKind,
                )}
                onPick={() => {
                  onChange(plugin.name);
                  setOpen(false);
                  setSearch("");
                }}
              />
            ))}
            {filteredPlugins.length === 0 && (
              <div className="rounded-md border border-dashed p-3 text-center text-xs text-muted-foreground">
                {t(WEBUI.plugins.noMatches)}
              </div>
            )}
          </div>
          {allowCreate && createType && (
            <div className="mt-2 border-t pt-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="w-full"
                onClick={() => {
                  setCreateOpen(true);
                  setOpen(false);
                }}
              >
                <Plus className="h-4 w-4" />
                {t(WEBUI.plugins.quickCreate)}{" "}
                {pluginTypeLabel(createType, locale)}
              </Button>
            </div>
          )}
        </PopoverContent>
      </Popover>
      {allowCreate && createType && (
        <CreatePluginDialog
          key={`${createType}-${search.trim()}`}
          open={createOpen}
          onOpenChange={setCreateOpen}
          defaultType={createType}
          supportedTypes={referenceTypes}
          supportedPluginKinds={referencePlugins}
          defaultName={search.trim()}
          onCreated={onChange}
          trigger={null}
          title={t(WEBUI.plugins.quickCreate)}
          description={
            createDescription ?? t(WEBUI.plugins.quickCreateDescription)
          }
          createButtonLabel={t(WEBUI.plugins.createAndReference)}
        />
      )}
    </>
  );
}

function scrollListByWheel(
  event: WheelEvent<HTMLDivElement>,
  list: HTMLDivElement | null,
) {
  if (!list) return;
  event.preventDefault();
  event.stopPropagation();
  list.scrollTop += event.deltaY;
}

function scrollListByTouch(
  event: TouchEvent<HTMLDivElement>,
  list: HTMLDivElement | null,
  touchYRef: React.MutableRefObject<number | null>,
) {
  const nextY = event.touches[0]?.clientY ?? null;
  if (!list || nextY === null || touchYRef.current === null) {
    touchYRef.current = nextY;
    return;
  }

  event.preventDefault();
  event.stopPropagation();
  list.scrollTop += touchYRef.current - nextY;
  touchYRef.current = nextY;
}

function PluginReferenceOption({
  plugin,
  supported,
  onPick,
}: {
  plugin: PluginInstance;
  supported: boolean;
  onPick: () => void;
}) {
  const { t } = useI18n();
  return (
    <button
      type="button"
      disabled={!supported}
      title={supported ? undefined : t(WEBUI.common.unsupportedBuild)}
      className={cn(
        "flex w-full items-center gap-2 rounded-md border bg-background p-2 text-left",
        supported
          ? "hover:bg-accent"
          : "cursor-not-allowed border-dashed opacity-55",
      )}
      onClick={onPick}
    >
      <PluginReferenceCompact plugin={plugin} supported={supported} />
    </button>
  );
}

function PluginReferenceCompact({
  plugin,
  supported = true,
}: {
  plugin: PluginInstance;
  supported?: boolean;
}) {
  const { locale, t } = useI18n();
  const definition = getPluginCatalogItem(plugin.pluginKind, locale);

  return (
    <span className="flex min-w-0 flex-1 items-center gap-2">
      <span
        className={cn(
          "shrink-0 rounded-md p-1",
          pluginKindIconBgClass(plugin.type),
        )}
      >
        {renderPluginKindIcon(definition?.icon ?? "Database", {
          className: "h-3.5 w-3.5 shrink-0",
        })}
      </span>
      <span className="min-w-0 flex-1 text-left">
        <span className="block truncate font-mono text-xs font-medium">
          {plugin.name}
        </span>
        <span className="block truncate text-[10px] leading-tight text-muted-foreground">
          {pluginTypeLabel(plugin.type, locale)} ·{" "}
          {definition?.name ?? plugin.pluginKind}
        </span>
      </span>
      {!supported && (
        <span className="shrink-0 rounded border px-1 py-0.5 text-[10px] text-muted-foreground">
          {t(WEBUI.common.notCompiled)}
        </span>
      )}
    </span>
  );
}

function stripReferencePrefix(value: unknown): string {
  const text = typeof value === "string" ? value.trim() : "";
  const withoutInvert = text.startsWith("!") ? text.slice(1) : text;
  return withoutInvert.startsWith("$") ? withoutInvert.slice(1) : withoutInvert;
}
