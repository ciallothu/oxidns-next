"use client";

import { useEffect, useMemo, useState } from "react";
import {
  GitBranch,
  LayoutGrid,
  List,
  RotateCcw,
  Search,
} from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { CreatePluginDialog } from "@/components/plugins/create-plugin-dialog";
import { PluginDeleteButton } from "@/components/plugins/plugin-delete-button";
import { SortablePluginGrid } from "@/components/plugins/sortable-plugin-grid";
import { TopologyView } from "@/components/plugins/plugin-topology-view";
import {
  getPluginCatalogItem,
  renderPluginKindIcon,
} from "@/components/plugins/catalog";
import { pluginTypeColors, pluginTypeIcons } from "@/components/plugins/display";
import { isPluginKindSupported } from "@/lib/build-capabilities";
import { LOCALES, WEBUI } from "@/lib/i18n";
import {
  getPluginSearchText,
  pluginTypeLabel,
} from "@/lib/i18n/plugin-defined";
import { useI18n } from "@/lib/i18n/provider";
import {
  applySavedPluginOrder,
  sortPluginsForDashboard,
} from "@/lib/plugin-display-order";
import { useAuthStore } from "@/lib/auth-store";
import { useAppStore } from "@/lib/store";
import type { PluginInstance, PluginType } from "@/lib/types";
import { cn } from "@/lib/utils";

const DISPLAY_ORDER_PREFIX = "oxidns-next:plugin-display-order:";
const LEGACY_DISPLAY_ORDER_KEY = "oxidns-next:plugin-display-order";

function loadDisplayOrder(storageKey: string): string[] {
  try {
    // The first Next preview used one global key. Remove it so plugin tags
    // from another backend or user cannot leak into this authenticated scope.
    localStorage.removeItem(LEGACY_DISPLAY_ORDER_KEY);
    const value = localStorage.getItem(storageKey);
    if (!value) return [];
    const parsed: unknown = JSON.parse(value);
    return Array.isArray(parsed)
      ? parsed.filter((id): id is string => typeof id === "string")
      : [];
  } catch {
    return [];
  }
}

function saveDisplayOrder(storageKey: string, ids: readonly string[]): void {
  try {
    localStorage.setItem(storageKey, JSON.stringify(ids));
  } catch {
    // A private/locked-down browser may reject localStorage.  The default
    // functional ordering remains available for the current render.
  }
}

/**
 * The dashboard's complete plugin-management workspace.  Card order is kept
 * as a browser display preference and never mutates YAML execution order.
 */
export function PluginWorkspace() {
  const { locale, t } = useI18n();
  const [activeTab, setActiveTab] = useState<PluginType | "all">("all");
  const [viewMode, setViewMode] = useState<"grid" | "table" | "topology">(
    "grid",
  );
  const [search, setSearch] = useState("");
  const [savedOrder, setSavedOrder] = useState<string[]>([]);
  const serverUrl = useAuthStore((state) => state.serverConfig.url);
  const userId = useAuthStore((state) => state.user?.id ?? "anonymous");
  const displayOrderStorageKey = useMemo(
    () => `${DISPLAY_ORDER_PREFIX}${serverUrl.trim()}|${userId}`,
    [serverUrl, userId],
  );

  const plugins = useAppStore((state) => state.plugins);
  const buildInfo = useAppStore((state) => state.buildInfo);
  const dependencyGraph = useAppStore((state) => state.dependencyGraph);
  const setSelectedPlugin = useAppStore((state) => state.setSelectedPlugin);
  const setDetailOpen = useAppStore((state) => state.setDetailOpen);

  useEffect(() => {
    setSavedOrder(loadDisplayOrder(displayOrderStorageKey));
  }, [displayOrderStorageKey]);

  const defaultOrder = useMemo(
    () => sortPluginsForDashboard(plugins),
    [plugins],
  );
  const orderedPlugins = useMemo(
    () => applySavedPluginOrder(defaultOrder, savedOrder),
    [defaultOrder, savedOrder],
  );

  const filteredPlugins = useMemo(() => {
    const normalizedSearch = search.trim().toLowerCase();
    return orderedPlugins.filter((plugin) => {
      if (activeTab !== "all" && plugin.type !== activeTab) return false;
      if (!normalizedSearch) return true;

      const definition = getPluginCatalogItem(plugin.pluginKind, locale);
      const baseDefinition = getPluginCatalogItem(plugin.pluginKind);
      const searchableText = [
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
        .toLowerCase();

      return searchableText.includes(normalizedSearch);
    });
  }, [activeTab, locale, orderedPlugins, search]);

  const pluginsByType = useMemo(
    () => ({
      server: plugins.filter((plugin) => plugin.type === "server"),
      executor: plugins.filter((plugin) => plugin.type === "executor"),
      matcher: plugins.filter((plugin) => plugin.type === "matcher"),
      provider: plugins.filter((plugin) => plugin.type === "provider"),
    }),
    [plugins],
  );

  const handleSelect = (plugin: PluginInstance) => {
    setSelectedPlugin(plugin);
    setDetailOpen(true);
  };

  const handleVisibleReorder = (visibleIds: string[]) => {
    const visibleSet = new Set(filteredPlugins.map((plugin) => plugin.id));
    let visibleIndex = 0;
    const mergedOrder = orderedPlugins.map((plugin) =>
      visibleSet.has(plugin.id) ? visibleIds[visibleIndex++] : plugin.id,
    );
    setSavedOrder(mergedOrder);
    saveDisplayOrder(displayOrderStorageKey, mergedOrder);
  };

  const resetDisplayOrder = () => {
    setSavedOrder([]);
    try {
      localStorage.removeItem(displayOrderStorageKey);
    } catch {
      // The in-memory reset still takes effect when storage is unavailable.
    }
  };

  return (
    <Tabs
      value={activeTab}
      onValueChange={(value) =>
        setActiveTab(value as PluginType | "all")
      }
      className="space-y-4"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="relative min-w-[14rem] max-w-md flex-1">
          <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            placeholder={t(WEBUI.plugins.searchPlaceholder)}
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            className="pl-9"
          />
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="ghost"
            size="icon-sm"
            title={t(WEBUI.plugins.resetDisplayOrder)}
            aria-label={t(WEBUI.plugins.resetDisplayOrder)}
            onClick={resetDisplayOrder}
            disabled={savedOrder.length === 0}
          >
            <RotateCcw className="h-4 w-4" />
          </Button>
          <div className="flex items-center rounded-md border">
            <Button
              variant={viewMode === "grid" ? "secondary" : "ghost"}
              size="icon-sm"
              className="rounded-r-none"
              onClick={() => setViewMode("grid")}
            >
              <LayoutGrid className="h-4 w-4" />
            </Button>
            <Button
              variant={viewMode === "table" ? "secondary" : "ghost"}
              size="icon-sm"
              className="rounded-none"
              onClick={() => setViewMode("table")}
            >
              <List className="h-4 w-4" />
            </Button>
            <Button
              variant={viewMode === "topology" ? "secondary" : "ghost"}
              size="icon-sm"
              className="rounded-l-none"
              onClick={() => setViewMode("topology")}
            >
              <GitBranch className="h-4 w-4" />
            </Button>
          </div>
          <CreatePluginDialog
            defaultType={activeTab !== "all" ? activeTab : undefined}
          />
        </div>
      </div>

      {viewMode !== "topology" && (
        <div className="oxidns-next-no-scrollbar overflow-x-auto">
          <TabsList>
            <TabsTrigger value="all">
              {t(WEBUI.common.all)}
              <Badge variant="secondary" className="ml-1.5 text-xs">
                {plugins.length}
              </Badge>
            </TabsTrigger>
            {(Object.keys(pluginsByType) as PluginType[]).map((type) => (
              <TabsTrigger key={type} value={type} className="gap-1.5">
                {pluginTypeIcons[type]}
                {pluginTypeLabel(type, locale)}
                <Badge variant="secondary" className="ml-1 text-xs">
                  {pluginsByType[type].length}
                </Badge>
              </TabsTrigger>
            ))}
          </TabsList>
        </div>
      )}

      {viewMode === "grid" ? (
        <SortablePluginGrid
          plugins={filteredPlugins}
          onReorder={handleVisibleReorder}
          disabled={search.trim() !== ""}
        />
      ) : viewMode === "table" ? (
        <div className="overflow-hidden rounded-lg border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t(WEBUI.common.name)}</TableHead>
                <TableHead>{t(WEBUI.common.type)}</TableHead>
                <TableHead>{t(WEBUI.common.plugin)}</TableHead>
                <TableHead className="w-[80px]" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {filteredPlugins.map((plugin) => (
                <TableRow
                  key={plugin.id}
                  className="group cursor-pointer"
                  onClick={() => handleSelect(plugin)}
                >
                  <TableCell className="font-mono font-medium">
                    {plugin.name}
                  </TableCell>
                  <TableCell>
                    <Badge
                      variant="outline"
                      className={cn(
                        "gap-1",
                        pluginTypeColors[plugin.type],
                      )}
                    >
                      {pluginTypeIcons[plugin.type]}
                      {pluginTypeLabel(plugin.type, locale)}
                    </Badge>
                  </TableCell>
                  <TableCell>
                    <PluginKindBadge
                      pluginKind={plugin.pluginKind}
                      supported={isPluginKindSupported(
                        buildInfo,
                        plugin.type,
                        plugin.pluginKind,
                      )}
                    />
                  </TableCell>
                  <TableCell>
                    <div className="flex justify-end opacity-0 transition-opacity group-hover:opacity-100">
                      <PluginDeleteButton
                        plugin={plugin}
                        className="h-7 w-7 hover:text-destructive"
                      />
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      ) : (
        <TopologyView
          plugins={orderedPlugins}
          dependencyGraph={dependencyGraph}
          onSelect={handleSelect}
        />
      )}

      {viewMode !== "topology" && filteredPlugins.length === 0 && (
        <div className="rounded-lg border border-dashed p-12 text-center text-muted-foreground">
          <p>{t(WEBUI.plugins.noMatches)}</p>
          {search && (
            <p className="mt-1 text-sm">
              {t(WEBUI.plugins.tryAdjustSearch)}
              <button
                onClick={() => setSearch("")}
                className="ml-1 text-primary hover:underline"
              >
                {t(WEBUI.common.clearSearch)}
              </button>
            </p>
          )}
        </div>
      )}
    </Tabs>
  );
}

function PluginKindBadge({
  pluginKind,
  supported,
}: {
  pluginKind: string;
  supported: boolean;
}) {
  const { locale, t } = useI18n();
  const definition = getPluginCatalogItem(pluginKind, locale);

  return (
    <Badge
      variant="outline"
      className={cn(
        "gap-1.5",
        !supported && "border-dashed text-muted-foreground",
      )}
    >
      {definition &&
        renderPluginKindIcon(definition.icon, { className: "h-3 w-3" })}
      {definition?.name ?? pluginKind}
      {!supported && ` · ${t(WEBUI.common.notCompiled)}`}
    </Badge>
  );
}
