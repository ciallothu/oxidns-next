import type { ChartOption } from "@/components/ui/analytics-chart";
import type { QueryRecorderTimeseriesResponse } from "./oxidns-next-api";

export function distributionOption(
  rows: { key: string; count: number }[],
  name: string,
  mode: "bar" | "horizontal" | "pie",
): ChartOption {
  if (mode === "pie")
    return {
      tooltip: { trigger: "item", renderMode: "richText", confine: true },
      legend: { type: "scroll", bottom: 0 },
      series: [
        {
          type: "pie",
          name,
          radius: ["35%", "65%"],
          center: ["50%", "43%"],
          label: { show: false },
          data: rows.map((row) => ({ name: row.key, value: row.count })),
        },
      ],
    };
  const horizontal = mode === "horizontal";
  const category = {
    type: "category",
    data: rows.map((row) => row.key),
    inverse: horizontal,
    axisLabel: {
      hideOverlap: !horizontal,
      width: horizontal ? 120 : 70,
      overflow: "truncate",
      interval: horizontal ? 0 : "auto",
    },
    axisTick: { show: false },
  };
  const value = { type: "value", minInterval: 1 };
  return {
    grid: { left: 12, right: 20, top: 16, bottom: 24, containLabel: true },
    tooltip: {
      trigger: "axis",
      renderMode: "richText",
      confine: true,
      axisPointer: { type: "shadow" },
    },
    xAxis: horizontal ? value : category,
    yAxis: horizontal ? category : value,
    series: [
      {
        type: "bar",
        name,
        data: rows.map((row) => row.count),
        barMaxWidth: 32,
        itemStyle: { borderRadius: horizontal ? [0, 3, 3, 0] : [3, 3, 0, 0] },
      },
    ],
  };
}

export function timeseriesOption(
  points: QueryRecorderTimeseriesResponse["points"],
  names: { total: string; errors: string; missing: string; average: string },
  locale: string,
): ChartOption {
  const span = (points.at(-1)?.bucket_ms ?? 0) - (points[0]?.bucket_ms ?? 0);
  const axisFormat = new Intl.DateTimeFormat(locale, {
    timeZone: "UTC",
    ...(span >= 60 * 86_400_000
      ? { year: "numeric", month: "2-digit" }
      : span >= 2 * 86_400_000
        ? { month: "2-digit", day: "2-digit" }
        : {
            month: "2-digit",
            day: "2-digit",
            hour: "2-digit",
            minute: "2-digit",
          }),
  });
  const fullFormat = new Intl.DateTimeFormat(locale, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    timeZone: "UTC",
  });
  const formatTime = (value: number) => axisFormat.format(value);
  const numberFormat = new Intl.NumberFormat(locale, {
    maximumFractionDigits: 2,
  });
  return {
    useUTC: true,
    grid: { left: 10, right: 10, top: 64, bottom: 78, containLabel: true },
    legend: { type: "scroll", top: 0 },
    tooltip: {
      trigger: "axis",
      renderMode: "richText",
      confine: true,
      valueFormatter: (value: unknown) =>
        typeof value === "number" ? numberFormat.format(value) : "–",
      axisPointer: {
        type: "line",
        label: {
          formatter: (params: { value: number }) =>
            `${fullFormat.format(params.value)} UTC`,
        },
      },
    },
    xAxis: {
      type: "time",
      min: points[0]?.bucket_ms,
      max: points.at(-1)?.bucket_ms,
      axisLabel: { hideOverlap: true, formatter: formatTime },
      axisPointer: { snap: true },
    },
    yAxis: [
      { type: "value", minInterval: 1 },
      { type: "value", name: "ms", splitLine: { show: false } },
    ],
    dataZoom: [
      {
        type: "inside",
        filterMode: "none",
        zoomOnMouseWheel: "ctrl",
        moveOnMouseWheel: false,
        moveOnMouseMove: true,
      },
      {
        type: "slider",
        filterMode: "none",
        height: 24,
        bottom: 8,
        labelFormatter: formatTime,
      },
    ],
    series: [
      { key: "total", name: names.total, axis: 0 },
      { key: "error_count", name: names.errors, axis: 0 },
      { key: "no_response_count", name: names.missing, axis: 0 },
      { key: "avg_ms", name: `${names.average} (ms)`, axis: 1 },
      { key: "p95_ms", name: "P95 (ms)", axis: 1 },
    ].map(({ key, name, axis }) => ({
      id: key,
      name,
      type: "line",
      yAxisIndex: axis,
      smooth: false,
      showSymbol: points.length <= 31,
      symbolSize: 4,
      connectNulls: false,
      data: points.map((point) => [
        point.bucket_ms,
        axis === 1 && point.total === 0
          ? null
          : point[key as keyof typeof point],
      ]),
      lineStyle: { width: 2, type: key === "p95_ms" ? "dashed" : "solid" },
      emphasis: { focus: "series" },
    })),
  };
}
