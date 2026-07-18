---
title: Configuration Overview
sidebar_position: 2
---

## Before Starting

OxiDNS Next uses YAML configuration. For day-to-day editing, it is easiest to understand the file as six top-level parts:

```yaml
runtime:
  worker_threads: 4

api:
  http: "127.0.0.1:9088"

log:
  level: info
  file: ./oxidns-next.log

network:
  outbound:
    default: direct
    profiles:
      direct:
        resolver: system
        proxy: none

include: []

plugins:
  - tag: seq_main
    type: sequence
    args:
      - exec: "forward 1.1.1.1"
```

Where:

- `runtime`
  - Runtime parameters.
- `api`
  - Management API settings.
- `log`
  - Log output settings.
- `network`
  - Shared outbound networking settings, such as resolver and proxy choices for HTTP downloads, upgrade checks, and webhook requests.
- `include`
  - Load plugin definitions from other configuration files.
- `plugins`
  - All plugin instance definitions. OxiDNS Next composes the full DNS pipeline from plugins.

After editing a config, validate it before starting:

```bash
oxidns-next check -c config.yaml
```

If the config uses relative paths and the runtime working directory is not the config directory, pass the working directory explicitly. `-d` is the single base for all runtime relative paths, including logs, SQLite files, rule files, and `api.http.webui.root`; paths do not become relative to `/etc/oxidns-next` just because the config file lives there:

```bash
oxidns-next check -c /etc/oxidns-next/config.yaml -d /var/lib/oxidns-next
```

In the Debian default layout, the config file lives at `/etc/oxidns-next/config.yaml`, while runtime-relative resources live under `/var/lib/oxidns-next`.

When the plugin composition is still undecided, start from [Common Scenarios](scenarios.md), then return to this page for field details.

## Environment Variable Substitution

During startup, `oxidns-next check`, management API validation, and validation before saving a config, OxiDNS Next first **parses the YAML into a data structure** and then expands `${VAR}` placeholders inside string scalars. The `config.yaml` file itself is not rewritten, so the WebUI still reads and saves the original placeholders.

Supported syntax:

| Syntax | Behavior |
| --- | --- |
| `${VAR}` | Use the value of process environment variable `VAR`; fail if it is undefined |
| `${VAR:-default}` | Use `default` when `VAR` is undefined or an empty string |
| `${env:VAR}` | Explicitly read process environment variable `VAR`; useful when the name conflicts with a runtime placeholder |
| `${env:VAR:-default}` | Explicitly read process environment variable `VAR`; use `default` when it is undefined or empty |
| `$${...}` | Emit a literal `${...}` |

Runtime placeholders used by executors such as `script` and `http_request` are preserved until request execution, so values like `${qname}`, `${client_ip}`, and `${resp_ip}` are not treated as process environment variables during config loading. Use the explicit form, such as `${env:qname}`, if you really need to read an environment variable with the same name.

Undefined variables fail fast, and the error includes the variable name and the YAML path of the offending scalar (for example `plugins[0].args.password`) so empty passwords or certificate paths do not silently pass validation.

Example:

```yaml
api:
  http:
    listen: ${API_LISTEN:-0.0.0.0:8080}
    ssl:
      cert: ${API_TLS_CERT}
      key: ${API_TLS_KEY}
    auth:
      type: accounts
      database: ${OXIDNS_NEXT_AUTH_DB:-./data/oxidns-next-auth.db}
      bootstrap_token_env: OXIDNS_NEXT_BOOTSTRAP_TOKEN
```

Because substitution happens after YAML parsing, an environment value may contain any characters — `*`, `&`, `:`, `#`, `'`, `"`, `\`, newlines, even binary bytes — without breaking the config syntax. You do not need to manually quote values that contain special characters. When the entire scalar is exactly one placeholder (e.g. `timeout: ${CACHE_TTL}`), the expanded value is re-parsed once against the YAML 1.2 scalar rules, so number / boolean / `null`-shaped environment values still match numeric / boolean / null fields; everywhere else the value lands as a plain string. `include` paths support placeholders too:

```yaml
include:
  - ${OXIDNS_NEXT_CONF_DIR}/plugins/common.yaml
```

## Top-Level Fields

### `include`

```yaml
# []string, load plugin settings from other configuration files.
include:
  - ./plugins/common.yaml
  - ./plugins/server.yaml
```

Field notes:

- `include`
  - Loads only `plugins` from included files. It does not merge included `runtime`, `api`, or `log` settings.
  - Merge order is include-first: recursively load each `include` in array order, then append the current file's `plugins`.
  - Relative paths are resolved from the directory of the configuration file that declares the `include`.
  - Includes may recurse up to 8 levels.
  - All merged plugin `tag` values must still be globally unique.

### `runtime`

```yaml
runtime:
  worker_threads: 4
```

Field notes:

- `worker_threads`
  - Meaning: Number of Tokio multi-thread runtime workers.
  - Default: Uses system available parallelism when omitted.
  - Constraint: Must not be `0`.

### `log`

```yaml
log:
  level: info
  file: ./oxidns-next.log
  query_file: ./data/query-events.log
  rotation:
    type: daily
    max_files: 7
```

Field notes:

- `level`
  - Allowed values: `off` `trace` `debug` `info` `warn` `error`
  - Default: `info`
- `file`
  - Meaning: Optional log file path.
  - If omitted, logs go only to stdout.
  - When configured, OxiDNS Next writes to both stdout and the log file.
  - Log files are written as UTF-8 plain text without terminal ANSI color escape codes.
- `query_file`
  - Meaning: Optional DNS query-event log file used only by query diagnostics such as `debug_print` and `query_summary`.
  - If omitted, no query-event text file is written. Structured searchable history remains independently available through the `query_recorder` SQLite database.
  - Configure this field when text output from `debug_print` or `query_summary` is wanted. Without it, those query diagnostics do not enter system logs; searchable history is unaffected.
  - Query events may contain client addresses and domain names; restrict file access and choose a retention period that matches your privacy policy.
- `rotation`
  - Meaning: Log file rotation policy.
  - Default: `never`

`rotation` supports the following forms:

- `type: never`
- `type: minutely`
  - Rotate every minute.
- `type: hourly`
  - Rotate every hour.
- `type: daily`
  - Rotate every day.
- `type: weekly`
  - Rotate every week.
  - Optional `max_files` controls how many rotated files are retained; `0` disables automatic cleanup.

### `network`

`network.outbound` centralizes outbound policy for internal HTTP clients and upstreams. When omitted, behavior stays compatible: HTTP clients use system DNS with direct connections, and upstreams keep their own settings.

```yaml
network:
  outbound:
    default: direct
    profiles:
      direct:
        resolver: system
        proxy: none
      remote:
        resolver:
          nameservers:
            - addr: "1.1.1.1:53"
            - addr: "tls://dns.google:853"
              dial_addr: 8.8.8.8
            - addr: "https://cloudflare-dns.com/dns-query"
              dial_addr: 1.1.1.1
          ip_version: 4
          timeout: 5s
          proxy: none
        proxy:
          socks5: 127.0.0.1:1080
```

Field notes:

- `outbound.default`
  - Meaning: Which profile HTTP clients and upstreams use when they do not set `outbound` explicitly.
  - Default: none; without a default profile, OxiDNS Next uses system DNS + direct connections.
  - Constraint: If set, it must reference an existing entry in `profiles`.
  - Note: The default profile proxy is applied strictly to upstreams. Startup fails if a default SOCKS5 proxy is applied to UDP, DoQ, or DoH3 upstreams, because those connection models do not support profile proxying.
- `outbound.profiles.<name>.resolver`
  - `system`: Use system DNS. HTTP clients perform this lookup asynchronously so it does not block runtime worker threads.
  - `nameservers`: Resolve target names through configured DNS nameservers. Supports `udp://`, `tcp://`, `tls://`, `https://`, `doh://`, `h3://`, `quic://`, and `doq://`; no scheme defaults to UDP.
  - Protocol features: UDP/TCP are always available. DoT requires `resolver-dot`, DoH requires `resolver-doh`, DoQ requires `resolver-doq`, and DoH3 requires `resolver-doh3`. Legacy `upstream-*` features still enable the shared DNS client dependencies for existing build scripts, but new `network.outbound.resolver.nameservers` configs should enable `resolver-*` explicitly.
  - `ip_version`: Optional, `4` queries A records and `6` queries AAAA records. When omitted, IPv4 is used.
  - `timeout`: Optional resolver query timeout. Defaults to `5s`.
  - `proxy`: Optional. `none` connects nameservers directly; `profile` lets TCP/DoT/DoH nameservers reuse this profile's SOCKS5 proxy. UDP/DoQ/DoH3 nameservers do not support SOCKS5.
  - Domain-based nameservers must set `dial_addr`; the hostname in `addr` is kept for SNI/certificate validation and `dial_addr` is used for the actual connection.
- `outbound.profiles.<name>.proxy`
  - `none` or `direct`: Connect directly.
  - `socks5`: Connect through a SOCKS5 proxy. The format is the same as upstream `socks5`.

`download`, `upgrade`, and `http_request` can reference a profile with `args.outbound: remote`. The legacy `socks5` field remains supported. When both `outbound` and `socks5` are set on the same plugin, `socks5` overrides the profile proxy while the resolver still comes from the outbound profile. `forward` upstreams use `network.outbound.default` when `outbound` is omitted; they can also set `outbound: remote` to select another profile. Local upstream `dial_addr`, `bootstrap`, and `socks5` fields override profile-injected values.

### `api`

`api.http` supports two forms.

Shorthand:

```yaml
api:
  http: "127.0.0.1:9088"
```

Expanded form:

```yaml
api:
  http:
    listen: "127.0.0.1:9443"
    ssl:
      cert: "/etc/oxidns-next/api.crt"
      key: "/etc/oxidns-next/api.key"
      client_ca: "/etc/oxidns-next/client-ca.crt"
      require_client_cert: true
    auth:
      type: accounts
      database: "./data/oxidns-next-auth.db"
      bootstrap_token_env: OXIDNS_NEXT_BOOTSTRAP_TOKEN
      session_ttl_seconds: 43200
      cookie_same_site: lax
      public_url: "https://dns.example.com"
      passkey:
        rp_id: "dns.example.com"
        origins: ["https://dns.example.com"]
    webui:
      root: "/etc/oxidns-next/webui"
      index: "index.html"
```

Field notes:

- `http.listen`
  - API listen address. Supports `ip:port`, `[ipv6]:port`, and `:port`.
  - `:port` binds as dual-stack `[::]:port`; use `0.0.0.0:port` for IPv4-only.
- `http.ssl.cert`
  - API certificate file.
- `http.ssl.key`
  - API private key file.
- `http.ssl.client_ca`
  - Optional client certificate CA.
- `http.ssl.require_client_cert`
  - Whether mutual TLS is required.
- `http.auth`
  - New deployments use `type: accounts`, with SQLite storage for local accounts, TOTP, passkeys, OIDC bindings, and sessions.
  - `type: basic` remains only as a one-time upstream-config migration path. Remove the plaintext YAML password after import.
- `http.auth.database`
  - Account database path. Defaults to `./data/oxidns-next-auth.db`; relative paths resolve against the working directory. The database and its backups contain password hashes, TOTP secrets, passkeys, and session security data and must be protected as credentials. On Unix, runtime tightens the database mode to `0600`.
- `http.auth.bootstrap_token` / `bootstrap_token_env`
  - One-time token required to create the first administrator from anything other than a direct loopback request, including a local reverse proxy. Configure only one. Reverse-proxy and remote bootstrap should use `bootstrap_token_env`; remove the environment variable afterward and do not retain the token in YAML.
- `http.auth.session_ttl_seconds`
  - Session lifetime from 300 through 604800 seconds. Defaults to 43200.
- `http.auth.cookie_secure`
  - Optional override for automatic Secure-cookie handling. HTTPS production deployments normally leave this unset.
- `http.auth.cookie_same_site`
  - Supports `lax` (default), `strict`, and `none`; a cross-site WebUI using `none` also requires a Secure cookie and an exact CORS origin.
- `http.auth.public_url`
  - Absolute browser-visible HTTP(S) URL used to derive passkey and callback origins. When a reverse proxy terminates TLS, it is also the exact trusted origin accepted by public authentication endpoints. Configure the URL that browsers actually use for the API; `X-Forwarded-*` headers are not trusted.
- `http.auth.passkey`
  - `rp_id` and `origins` can be explicit. Without them, they must be derivable from `public_url`.
- `http.auth.oidc`
  - Configures `issuer_url`, `client_id`, a client secret, `redirect_url`, and `allowed_users`.
  - Each `allowed_users` entry explicitly maps an identity-provider claim to an existing local account. OIDC does not create administrators.
  - Inject the client secret through `client_secret_env` instead of writing it in YAML. `client_secret` remains only for compatibility, and the two forms cannot be configured together.
- `http.cors.allowed_origins`
  - Optional WebUI/API cross-origin allowlist.
  - With authentication disabled, an omitted rule is inferred from `http.listen`: `0.0.0.0` and `[::]` allow any origin, while a specific IP allows any WebUI port on the same host.
  - Account authentication does not use that permissive inference. A same-origin WebUI needs no CORS rule; a cross-origin WebUI carrying the session cookie must list every exact origin explicitly.
  - When configured explicitly, entries are matched exactly against the browser's `Origin`.
  - Use `"*"` to allow any origin, but not for credentialed browser requests.
- `http.webui.root`
  - Optional WebUI static file directory. When enabled, the WebUI is mounted at `/` and the management API is available under `/api/*`.
  - Relative paths resolve against `-d/--working-dir`; with the Debian service default `-d /var/lib/oxidns-next`, `root: "./webui"` means `/var/lib/oxidns-next/webui`.
  - See [WebUI Deployment](webui.md) for build steps, publish directories, and standalone nginx deployment.
- `http.webui.index`
  - Optional index file name. Defaults to `index.html`.

Validation rules:

- `listen` must not be empty.
- `cert` and `key` must be configured together.
- `require_client_cert: true` requires `client_ca`.
- `accounts.database` must not be empty; `bootstrap_token` and `bootstrap_token_env` cannot both be configured.
- `session_ttl_seconds` must be between 300 and 604800.
- `cookie_same_site: none` cannot be combined with `cookie_secure: false`, and runtime configuration must make the cookie Secure.
- Enabled OIDC requires valid issuer, client ID, redirect URL, scopes containing `openid`, and at least one `allowed_users` mapping.
- Enabled passkeys require a browser scope through either `public_url` or `rp_id` plus `origins`.
- `webui.root` must not be empty.
- `webui.index`, when configured, must not be empty.

### `plugins`

Each plugin definition uses the same outer structure:

```yaml
- tag: cache_main
  type: cache
  args:
    size: 4096
```

General rules:

- `tag`
  - Unique plugin instance identifier.
  - Must not be empty.
  - Must be unique across the whole config.
- `type`
  - Plugin type name.
  - Must match a registered plugin factory.
- `args`
  - Plugin parameters.
  - Different plugins accept different shapes: object, string, array, or null.

## Responsibilities of the Four Plugin Categories

### `server`

Purpose: Accept DNS requests and send them into an executor entry.

Traits:

- Does not implement complex policy logic.
- Usually configures a bind address, TLS parameters, and an entry executor.

### `executor`

Purpose: Perform actions.

Typical actions include:

- Query upstreams
- Generate local answers
- Read and write cache
- Adjust TTL
- Handle ECS
- Run fallback and concurrent races
- Perform observability and system integrations

### `matcher`

Purpose: Evaluate conditions for use in `sequence` rules.

Typical match dimensions include:

- Query name
- Query type
- Client IP
- Response IP
- Response code
- Environment variables
- Sampling outcome
- Rate-limit state

### `provider`

Purpose: Provide reusable datasets for matchers or other plugins.

Current main provider types:

- `domain_set`
- `ip_set`
- `geoip`
- `geosite`
- `adguard_rule`

## The `sequence` Orchestration Model

`sequence` is the policy hub of OxiDNS Next. Most non-trivial configs use it as the primary entry.

Example:

```yaml
- tag: seq_main
  type: sequence
  args:
    - matches:
        - "$lan_clients"
        - "qtype A,28"
      exec: "$cache_main"
    - matches: "!has_resp"
      exec: "$forward_main"
    - exec: "accept"
```

Each rule has two key fields:

- `matches`
  - One matcher expression or an array of expressions.
  - When it is an array, every condition must be true for the rule to match.
- `exec`
  - The action to execute when the rule matches.

## Referencing Plugins and Quick Setup

### Reference Existing Plugins

Use `$tag` to reference a plugin that has already been defined:

```yaml
- exec: "$forward_main"
- matches:
    - "$is_internal"
    - "!has_resp"
  exec: "$cache_main"
```

### Quick Setup

If a `sequence` rule uses `type + arguments` instead of `$tag`, OxiDNS Next creates a temporary plugin on the fly.

Example:

```yaml
- exec: "forward 1.1.1.1 8.8.8.8"
- matches: "qname domain:example.com"
  exec: "ttl 300"
```

Common quick setup forms today:

- matcher
  - `_true`
  - `_false`
  - `qname ...`
  - `qtype ...`
  - `qclass ...`
  - `client_ip ...`
  - `resp_ip ...`
  - `ptr_ip ...`
  - `cname ...`
  - `mark ...`
  - `env ...`
  - `random ...`
  - `rate_limiter ...`
  - `rcode ...`
  - `has_resp`
  - `has_wanted_ans`
  - `string_exp ...`
- executor
  - `forward ...`
  - `cache ...`
  - `ttl ...`
  - `prefer_ipv4`
  - `prefer_ipv6`
  - `sleep ...`
  - `debug_print ...`
  - `query_summary ...`
  - `metrics_collector ...`
  - `black_hole ...`
  - `drop_resp`
  - `ecs_handler ...`
  - `forward_edns0opt ...`
  - `ipset ...`
  - `nftset ...`
  - `upgrade ...`
  - `download ...`
  - `reload_provider ...`
  - `reload`

## Built-In `sequence` Control Flow

Besides calling plugins, `sequence.args[].exec` can also use built-in control flow:

### `accept`

- Ends the current `sequence` immediately.
- This is an explicit early stop, so callers do not continue with later rules.
- Does not build a response by itself.
- Typical use:
  - Close out the pipeline after `cache`, `hosts`, or `arbitrary` has already written a response.
  - Stop later `forward` or side-effect stages once a branch has already made the decision.

### `return`

- Ends the current `sequence` immediately and returns control to the caller.
- Does not build a response.
- If the current `sequence` was entered via `jump`, the caller resumes at the rule after `jump`.
- If the current `sequence` is the top-level entry, this acts like an early exit from the current rule chain.

### `reject [rcode]`

- Builds a DNS response from the current request immediately and ends the current `sequence`.
- The default `rcode` is `REFUSED`, so plain `reject` means “reject this request”.
- A decimal numeric code or English RCODE name can be provided explicitly; English names are case-insensitive. Common mappings and meanings are listed in the [DNS Code Reference](dns-codes.md#rcode-response-codes), for example:
  - `reject 2` => `SERVFAIL`
  - `reject SERVFAIL` / `reject servfail` => `SERVFAIL`
  - `reject 3` => `NXDOMAIN`
  - `reject NXDOMAIN` => `NXDOMAIN`
- `reject` only supports base DNS RCODEs `0..15`; extended RCODEs require an EDNS OPT and are not generated by this built-in action.
- `reject 0` returns a plain `NOERROR` response and does not add an SOA automatically.
- Callers do not continue with later rules.
- A typical use is returning a specific error code directly, for example:

```yaml
- matches: "qtype HTTPS"
  exec: "reject NXDOMAIN"
```

### `mark ...`

- Inserts one or more unsigned integer marks into `DnsContext.marks`.
- Supported forms:
  - `mark 1`
  - `mark 1 2 3`
  - `mark 1,2,3`
- Continues to the next rule in the current `sequence`.
- Does not build a response and does not terminate the current `sequence`.

### `jump seq_tag`

- Calls another `sequence`; conceptually this behaves like a subroutine call.
- The parameter must be the target `sequence` tag without a leading `$`.
- If the called `sequence`:
  - reaches its tail normally, the current `sequence` resumes at the rule after `jump`.
  - executes `return`, the current `sequence` also resumes at the rule after `jump`.
  - executes `accept`, `reject`, or another operation that returns `Stop`, the current `sequence` stops as well.

### `goto seq_tag`

- Transfers control to another `sequence`; conceptually this behaves like a one-way jump.
- The parameter must be the target `sequence` tag without a leading `$`.
- The current `sequence` never resumes after `goto`:
  - If the target `sequence` reaches its tail, control does not return to the rules after `goto`.
  - If the target `sequence` executes `return`, that `return` is propagated outward and still does not return to the rules after `goto`.
  - If the target `sequence` executes `accept`, `reject`, or another `Stop`, that result propagates outward directly.
- This is useful when ownership of the request should be handed off permanently to another policy branch.

Example:

```yaml
- matches: "$rate_ok"
  exec: "mark 100"
- matches: "!$rate_ok"
  exec: "reject 2"
```

Example showing the difference between `jump` and `goto`:

```yaml
- tag: child_seq
  type: sequence
  args:
    - exec: "mark 2"
    - exec: "return"

- tag: parent_jump
  type: sequence
  args:
    - exec: "mark 1"
    - exec: "jump child_seq"
    - exec: "mark 3"

- tag: parent_goto
  type: sequence
  args:
    - exec: "mark 1"
    - exec: "goto child_seq"
    - exec: "mark 3"
```

- `parent_jump` ends with marks `1,2,3` because execution resumes after `jump`.
- `parent_goto` ends with marks `1,2` because execution never returns after `goto`.

## Common Rule Syntax

### Domain Rules

These forms appear in plugins such as `qname`, `cname`, `domain_set`, `hosts`, and `redirect`:

- `full:example.com`
  - Exact match.
- `domain:example.com`
  - Suffix match.
- `keyword:cdn`
  - Substring match.
- `regexp:^api[0-9]+\\.example\\.com$`
  - Regular-expression match.
- `example.com`
  - Without a prefix, common domain-rule users such as `qname`, `cname`, and
    `domain_set` usually treat it as `domain:example.com`; `hosts` and
    `redirect` treat it as an exact `full:example.com` match.

### IP Rules

These forms appear in `client_ip`, `resp_ip`, `ptr_ip`, `ip_set`, and related plugins:

- Single IP: `1.1.1.1`
- CIDR: `192.168.0.0/16`
- IPv6 CIDR: `2400:3200::/32`

### Provider References

Matchers and providers can reference providers through:

- `$tag`
  - References a defined provider with the required match capability.
  - Domain-oriented references can target `domain_set` or `geosite`.
  - IP-oriented references can target `ip_set` or `geoip`.
- `&/path/to/file`
  - Loads rules directly from a file.

Example:

```yaml
args:
  - "domain:example.com"
  - "$core_domains"
  - "&/etc/oxidns-next/domains.txt"
```
