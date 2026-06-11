"use client";

import type { PluginCardComponentProps } from "./types";
import { PluginCardTemplate } from "./plugin-card-template";
import { getPluginCatalogItem } from "./catalog";
import { WEBUI } from "@/lib/i18n";
import { useI18n } from "@/lib/i18n/provider";

export function DefaultPluginCard(props: PluginCardComponentProps) {
  const { locale, t } = useI18n();
  const definition = getPluginCatalogItem(props.plugin.pluginKind, locale);
  const configFields = definition?.configSchema.slice(0, 3) ?? [];

  return (
    <PluginCardTemplate {...props}>
      <div className="space-y-1">
        {configFields.map((field) => (
          <div
            key={field.key}
            className="flex min-w-0 items-center justify-between gap-3 text-xs leading-5"
          >
            <span className="truncate text-muted-foreground">
              {field.label}
            </span>
            <span className="truncate text-right font-mono text-foreground">
              {formatCardConfigValue(props.plugin.config[field.key], t)}
            </span>
          </div>
        ))}
      </div>
    </PluginCardTemplate>
  );
}

function formatCardConfigValue(
  value: unknown,
  t: ReturnType<typeof useI18n>["t"],
) {
  if (value === undefined || value === null || value === "") {
    return t(WEBUI.common.unconfigured);
  }
  if (typeof value === "boolean") {
    return value ? t(WEBUI.common.yes) : t(WEBUI.common.no);
  }
  if (typeof value === "number") return String(value);
  if (typeof value === "string") return value;
  if (Array.isArray(value))
    return value.length > 0
      ? t(WEBUI.common.itemCount, { count: value.length })
      : t(WEBUI.common.empty);
  if (typeof value === "object") {
    return Object.keys(value).length > 0
      ? t(WEBUI.common.configured)
      : t(WEBUI.common.empty);
  }
  return String(value);
}
