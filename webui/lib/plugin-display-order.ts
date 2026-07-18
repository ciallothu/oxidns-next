import type { PluginInstance, PluginType } from "@/lib/types";

/**
 * Dashboard ordering is a presentation concern.  Keep it independent from the
 * YAML declaration order so that moving a card never changes execution order.
 */
const KIND_RANK: Record<string, number> = {
  // Ingress: show the listeners which receive DNS traffic first.
  udp_server: 100,
  tcp_server: 110,
  http_server: 120,
  quic_server: 130,

  // Request orchestration and reusable data sources.
  sequence: 200,
  fallback: 210,
  domain_set: 300,
  dynamic_domain_set: 310,
  geosite: 320,
  adguard_rule: 330,
  ip_set: 340,
  geoip: 350,

  // Matchers are grouped after the providers they commonly consume.
  any_match: 400,
  qname: 410,
  question: 420,
  qtype: 430,
  qclass: 440,
  client_ip: 450,
  resp_ip: 460,
  ptr_ip: 470,
  cname: 480,
  rcode: 490,
  has_resp: 500,
  has_wanted_ans: 510,
  mark: 520,
  env: 530,
  random: 540,
  rate_limiter: 550,
  string_exp: 560,
  _true: 570,
  _false: 580,

  // Resolution and local answer plugins form the main request path.
  cache: 600,
  hosts: 610,
  forward: 620,
  arbitrary: 630,
  redirect: 640,
  black_hole: 650,

  // Response shaping and address-family selection.
  ecs_handler: 700,
  forward_edns0opt: 710,
  ttl: 720,
  ip_selector: 730,
  prefer_ipv4: 740,
  prefer_ipv6: 750,
  drop_resp: 760,
  reverse_lookup: 770,

  // Observability follows request processing.
  query_recorder: 800,
  metrics_collector: 810,
  query_summary: 820,
  debug_print: 830,

  // External side effects and system integrations.
  learn_domain: 900,
  ipset: 910,
  nftset: 920,
  ros_address_list: 930,
  http_request: 940,
  script: 950,

  // Background and maintenance work appears last.
  cron: 1000,
  download: 1010,
  reload_provider: 1020,
  reload: 1030,
  upgrade: 1040,
  sleep: 1050,
};

const TYPE_FALLBACK_RANK: Record<PluginType, number> = {
  server: 150,
  provider: 390,
  matcher: 590,
  executor: 850,
};

function compareText(left: string, right: string): number {
  if (left < right) return -1;
  if (left > right) return 1;
  return 0;
}

/** Stable, locale-independent default order based on each plugin's role. */
export function sortPluginsForDashboard(
  plugins: readonly PluginInstance[],
): PluginInstance[] {
  return [...plugins].sort((left, right) => {
    const leftRank = KIND_RANK[left.pluginKind] ?? TYPE_FALLBACK_RANK[left.type];
    const rightRank =
      KIND_RANK[right.pluginKind] ?? TYPE_FALLBACK_RANK[right.type];

    return (
      leftRank - rightRank ||
      compareText(left.pluginKind, right.pluginKind) ||
      compareText(left.name, right.name) ||
      compareText(left.id, right.id)
    );
  });
}

/**
 * Apply a user's display preference without losing newly-created plugins.
 * Invalid/deleted ids are ignored and new cards retain the functional order.
 */
export function applySavedPluginOrder(
  defaultOrder: readonly PluginInstance[],
  savedIds: readonly string[],
): PluginInstance[] {
  const pluginsById = new Map(defaultOrder.map((plugin) => [plugin.id, plugin]));
  const seen = new Set<string>();
  const ordered: PluginInstance[] = [];

  for (const id of savedIds) {
    const plugin = pluginsById.get(id);
    if (!plugin || seen.has(id)) continue;
    ordered.push(plugin);
    seen.add(id);
  }

  for (const plugin of defaultOrder) {
    if (seen.has(plugin.id)) continue;
    ordered.push(plugin);
  }

  return ordered;
}
