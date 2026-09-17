import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

export interface PluginCardItem {
  key: string;
  label: string;
  value: string;
}

export function pluginCardGridColumnClass(itemCount: number): string {
  return itemCount < 4 ? "grid-cols-1" : "grid-cols-2";
}

export function PluginCardItemSurface({ children }: { children: ReactNode }) {
  return (
    <div className="flex h-[5.25rem] items-center overflow-hidden rounded-md bg-muted/25 px-3 py-2">
      {children}
    </div>
  );
}

export function PluginCardItemGrid({ items }: { items: PluginCardItem[] }) {
  return (
    <div
      className={cn(
        "grid w-full gap-x-5 gap-y-1",
        pluginCardGridColumnClass(items.length),
      )}
    >
      {items.map((item) => (
        <div
          key={item.key}
          className="grid h-5 min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-2 text-[11px] leading-5"
        >
          <span
            className="min-w-0 truncate text-muted-foreground"
            title={item.label}
          >
            {item.label}
          </span>
          <span
            className="max-w-[9.5rem] min-w-0 truncate text-right font-mono text-[11px] font-semibold tracking-[-0.01em] text-foreground tabular-nums"
            title={item.value}
          >
            {item.value}
          </span>
        </div>
      ))}
    </div>
  );
}
