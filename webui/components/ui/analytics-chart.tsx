"use client";

import { useEffect, useRef } from "react";
import { useTheme } from "next-themes";
import { useI18n } from "@/lib/i18n/provider";
import {
  use as registerCharts,
  init,
  type EChartsCoreOption,
} from "echarts/core";
import { BarChart, LineChart, PieChart } from "echarts/charts";
import {
  AriaComponent,
  DataZoomComponent,
  GridComponent,
  LegendComponent,
  TooltipComponent,
} from "echarts/components";
import { CanvasRenderer } from "echarts/renderers";

registerCharts([
  BarChart,
  LineChart,
  PieChart,
  AriaComponent,
  DataZoomComponent,
  GridComponent,
  LegendComponent,
  TooltipComponent,
  CanvasRenderer,
]);

export type ChartOption = EChartsCoreOption;

/** One lifecycle for charts in pages, tabs and animated detail sheets. */
export function AnalyticsChart({
  option,
  label,
  height = 320,
}: {
  option: ChartOption;
  label: string;
  height?: number;
}) {
  const container = useRef<HTMLDivElement>(null);
  const chart = useRef<ReturnType<typeof init> | null>(null);
  const { resolvedTheme } = useTheme();
  const { locale } = useI18n();
  const latest = useRef({ option, label });

  useEffect(() => {
    latest.current = { option, label };
  }, [option, label]);

  useEffect(() => {
    const element = container.current;
    if (!element) return;
    let frame = 0;
    const dark = resolvedTheme !== "light";
    const render = () => {
      if (element.clientWidth === 0 || element.clientHeight === 0) return;
      if (!chart.current) {
        chart.current = init(
          element,
          {
            color: dark
              ? ["#2dd4bf", "#fb7185", "#fbbf24", "#60a5fa", "#c084fc"]
              : ["#0f766e", "#e11d48", "#b45309", "#2563eb", "#9333ea"],
            backgroundColor: "transparent",
            textStyle: {
              color: dark ? "#d4d4d8" : "#3f3f46",
              fontFamily: getComputedStyle(element).fontFamily,
            },
            categoryAxis: {
              axisLine: { lineStyle: { color: dark ? "#52525b" : "#d4d4d8" } },
              axisLabel: { color: dark ? "#a1a1aa" : "#52525b" },
            },
            valueAxis: {
              axisLabel: { color: dark ? "#a1a1aa" : "#52525b" },
              splitLine: {
                lineStyle: {
                  color: dark ? "#3f3f46" : "#e4e4e7",
                  type: "dashed",
                },
              },
            },
            timeAxis: { axisLabel: { color: dark ? "#a1a1aa" : "#52525b" } },
            legend: { textStyle: { color: dark ? "#d4d4d8" : "#3f3f46" } },
            tooltip: {
              backgroundColor: dark ? "#18181b" : "#ffffff",
              borderColor: dark ? "#52525b" : "#d4d4d8",
              textStyle: { color: dark ? "#fafafa" : "#18181b" },
            },
          },
          {
            renderer: "canvas",
            devicePixelRatio: Math.min(window.devicePixelRatio || 1, 2),
            locale: locale === "zh-CN" ? "ZH" : "EN",
          },
        );
        chart.current.setOption({
          animation: false,
          aria: { enabled: true, label: { description: latest.current.label } },
          ...latest.current.option,
        });
      }
      chart.current.resize();
    };
    const observer = new ResizeObserver(() => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(render);
    });
    observer.observe(element);
    render();
    return () => {
      observer.disconnect();
      cancelAnimationFrame(frame);
      chart.current?.dispose();
      chart.current = null;
    };
  }, [resolvedTheme, locale]);

  useEffect(() => {
    chart.current?.setOption(
      {
        animation: false,
        aria: { enabled: true, label: { description: label } },
        ...option,
      },
      { replaceMerge: ["series"], lazyUpdate: true },
    );
  }, [option, label]);

  return (
    <div
      ref={container}
      role="img"
      aria-label={label}
      className="relative min-w-0 w-full overflow-hidden"
      style={{ height }}
    />
  );
}
