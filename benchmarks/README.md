# DNS benchmark pack

This directory contains the reproducible benchmark used for periodic OxiDNS
documentation snapshots.  It intentionally keeps product comparisons,
native-equivalent comparisons, and product-specific diagnostics separate.

`run_publishable_compare.py` is the only runner.  It records QPS, loss,
p50/p95/p99/max latency, process CPU, RSS, thread count, raw repeats, aggregated
TSV, a Markdown report, and SVG charts.

Each completed run also writes a SHA-256 manifest of the selected scenario
catalog, runner, engine configs, query files, and rule data to
`environment.json`. This ties every published measurement to the exact inputs
used on the server, including generated normalized domain-set assets.

## Test scopes

### Four-engine common matrix

These scenarios run on OxiDNS, mosdns, AdGuard Home, and SmartDNS:

| Scenario | What it represents |
|---|---|
| `02-cache-hotpath` | Positive warm-cache hits |
| `47-server-local-udp` | Minimal UDP listener and local response path |
| `48-server-local-tcp` | Minimal TCP listener/session and local response path |
| `50-common-local-answers` | A/AAAA local overrides across five names |
| `51-common-domain-set` | Positive lookup in one normalized 143k-domain corpus |
| `52-negative-cache-hotpath` | Warm NXDOMAIN cache hits with SOA-backed negative TTL |

Fairness means equivalent queries and asserted DNS response semantics, not
byte-identical configuration.  Each product uses its documented idiomatic path.
For example, OxiDNS uses `short_circuit: true` where a hit can safely stop the
sequence, while mosdns uses `has_resp` plus `accept`.  Product-specific speedups
are legitimate in this matrix when they preserve the same observable result.

The semantic gate checks addresses, record families, RCODEs, and TTLs rather
than accepting any syntactically valid response.  Common local answers use TTL
10, common domain-set answers use TTL 60, and the deterministic cache authority
uses TTL 300.  Negative-cache probes also require an SOA whose TTL and Minimum
are both 300.  Cache probes are issued twice so the second packet verifies the
cached response, with a small allowance for normal TTL aging.

The positive and negative cache scenarios use a deterministic UDP authority at
`127.0.0.1:5453`.  It only fills each freshly started cache.  The runner records
the fixture request counter immediately before and after every timed interval and
fails the run if the counter changes.  Therefore timed cache results cannot
silently include public-network or upstream-server work.

The shared domain corpus is generated once from `data/`.  The runner strips
supported `full:` prefixes, retains plain and `domain:` entries, and excludes
regex/keyword rules, invalid names, and duplicates that cannot be represented
equivalently by every product.  All engines then load that same generated file.

### OxiDNS/mosdns native-equivalent suite

`native-specialized` runs four pipelines which both engines can load from the
same YAML and data files:

| Scenario | What it represents |
|---|---|
| `08-domain-set` | Native plain/full/regexp domain rules with hits and misses |
| `09-ip-set` | Response-side CIDR hits and misses |
| `42-composite-local-rewrite` | Redirect, local answer, and TTL rewrite |
| `43-composite-provider-chain` | Domain provider followed by response-IP provider |

These results must not be merged into the four-engine rankings.  AdGuard Home and
SmartDNS do not expose equivalent in-memory pipelines for all four scenarios;
converting only the input text would not make their control flow equivalent.

### OxiDNS feature A/B suite

`oxidns-features` compares the same OxiDNS cache pipeline with:

- `60-cache-short-circuit`: stop directly on a cache hit;
- `61-cache-explicit-accept`: continue to `has_resp` plus `accept`.

This is an OxiDNS feature-cost diagnostic, not a product ranking.

## Layout

```text
benchmarks/
├── configs/
│   ├── common/{oxidns,mosdns,adguardhome,smartdns}/
│   ├── native/
│   └── oxidns/
├── data/
├── queries/
├── scenarios.tsv
├── run_publishable_compare.py
└── prepare_server.sh
```

`scenarios.tsv` explicitly declares supported engines and every configuration
path.  The runner never infers support from a matching filename, so an
unsupported engine cannot accidentally enter a comparison.

Columns are:

```text
label | engines | oxidns_config | mosdns_config | adguardhome_config |
smartdns_config | query_file | mode | family | warmup_query_file | tags |
description | notes
```

## Run it

On a clean Debian/Ubuntu benchmark host:

```bash
./prepare_server.sh
DNSPERF_BIN_PATH=./.tools/dnsperf-install/bin/dnsperf \
  ./run_publishable_compare.py --publish-docs
```

`prepare_server.sh` downloads the latest stable OxiDNS, AdGuard Home, and
SmartDNS releases by default.  mosdns and dnsperf are pinned at the top of that
script.  Set `OXIDNS_VERSION=vX.Y.Z` to pin OxiDNS, or
`BENCH_BUILD_OXIDNS=1` to build the current checkout.

Useful commands:

```bash
# Validate all paths and support declarations without starting binaries.
./run_publishable_compare.py --dry-run
./run_publishable_compare.py --dry-run native-specialized
./run_publishable_compare.py --dry-run oxidns-features

# Short semantic/configuration smoke tests.
BENCH_LOAD_LEVELS=1 BENCH_REPEATS=1 BENCH_SECONDS=2 WARMUP_SECONDS=2 \
  ./run_publishable_compare.py

# Full four-engine periodic snapshot.
BENCH_LOAD_LEVELS=1,4,16,64,256,1024 BENCH_REPEATS=3 \
  ./run_publishable_compare.py --publish-docs

# Separate native-equivalent suite.
./run_publishable_compare.py --engines oxidns,mosdns \
  --publish-native-specialized native-specialized

# OxiDNS-only short-circuit A/B result.
./run_publishable_compare.py --engines oxidns oxidns-features
```

The runner alternates engine order, starts a fresh process for every repeat/load
point, executes packet-level semantic probes before timing, and stores those
probes in `semantic-validation.json`.  Published points should use at least
three repeats.  A maximum stable point must have median loss no greater than
0.1%.

## What was deliberately removed

- Public UDP/DoH forwarding, concurrent upstream, and fallback tests were
  removed from the default pack because upstream location, route, TLS reuse,
  and Internet loss dominate a same-host engine comparison.  A future upstream
  matrix should use a separate deterministic upstream and a separate load host.
- Per-plugin matcher/executor YAML files were removed because a DNS server load
  test cannot isolate nanosecond-scale plugin overhead reliably.  Such checks
  belong in Rust/Go microbenchmarks and regression tests inside each project.
- Duplicate scenarios which differed only by port or query filename were
  removed.
- Observer/logging and artificial sleep scenarios were removed because they
  measure configured I/O or injected delay, not DNS engine capacity.

## Interpretation limits

This pack is representative of hot, same-host request-path cost and concurrency
scaling for the declared scenarios.  It is not a production-capacity claim for
other hardware and does not cover cold startup/reload, cache-miss-heavy traffic,
DoT/DoH/DoQ, multi-machine networking, public upstream quality, DNSSEC
validation, or host side effects such as ipset/nftset.  Those require separate
fixtures and should be reported as separate matrices.

The runner generates measurements, tables, and charts.  Documentation winner
prose must be reviewed and written from the complete curves, stable-point table,
repeat variance, semantic evidence, CPU/RSS samples, and test limitations; it
must not be inferred from a single QPS bar.
