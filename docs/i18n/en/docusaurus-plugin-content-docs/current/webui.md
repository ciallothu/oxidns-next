---
title: WebUI Deployment
sidebar_position: 5
---

The OxiDNS Next WebUI is a separately built static frontend. It is not compiled into the Rust backend binary. There are two recommended deployment modes:

- Backend-hosted WebUI: the OxiDNS Next management HTTP service serves the WebUI static directory directly. This is the simplest path for bare-metal hosts, NAS boxes, and small servers without nginx.
- Standalone nginx deployment: nginx serves the WebUI static files and reverse-proxies `/api/*` to the OxiDNS Next backend. This is better for environments that already use a domain, HTTPS, a gateway, or a shared service entry point.

In both modes, the WebUI defaults to the relative backend URL `/api`. When the WebUI page and `/api/*` share the same browser origin, no CORS setup is needed.

## Console Information Architecture

- The dashboard combines the system overview with every plugin and provides search, category, table, topology, create, and delete controls. Plugins have a functional default order covering ingress, orchestration, data sources, matching, resolution, response handling, observability, system integration, and maintenance. Dragging only stores a browser display preference and never rewrites YAML execution order.
- Query Log lives at `/query-log`. It uses `query_recorder` SQLite history and live events, with domain or client-address keyword and date-range filters. The default `config.yaml` enables a recorder with seven-day retention. The database contains client addresses, domains, and response data and must be treated as sensitive; on Unix its permissions are tightened to `0600` when opened or created.
- System Logs lives at `/logs` and shows runtime/system events without per-query diagnostic events.
- The former `/plugins` URL remains a compatibility entry and redirects to the dashboard plugin section.

## Use The WebUI Included In Release Packages

Official release archives include a prebuilt `webui/` directory:

```text
oxidns-next
config.yaml
LICENSE
webui/
```

When OxiDNS Next runs from the extracted release directory, the default `webui.root: "./webui"` config works directly. Docker images also place the same WebUI static files under `/etc/oxidns-next/webui`.

Debian packages install the service with `-c /etc/oxidns-next/config.yaml -d /var/lib/oxidns-next`. Therefore the default `webui.root: "./webui"` means `/var/lib/oxidns-next/webui`, which the post-install step links to `/usr/share/oxidns-next/webui`.

Manual WebUI builds are only needed when building from source, developing the WebUI, or publishing static files separately through nginx or caddy.

## Build The WebUI Manually

The WebUI lives in the repository's `webui/` directory. Production builds are exported to `webui/out`:

```bash
cd webui
pnpm install --frozen-lockfile
pnpm build
```

After building, publish `out/` to a server directory, for example:

```bash
sudo mkdir -p /etc/oxidns-next/webui
sudo rsync -a --delete out/ /etc/oxidns-next/webui/
```

The examples below use `/etc/oxidns-next/webui` as the static directory.

## Mode 1: Backend-Hosted WebUI

This mode only needs the OxiDNS Next management HTTP port. The WebUI is mounted at `/`, and the management API is mounted under `/api/*`.

```yaml
api:
  http:
    listen: "0.0.0.0:9199"
    auth:
      type: accounts
      database: "./data/oxidns-next-auth.db"
      session_ttl_seconds: 43200
    webui:
      root: "/etc/oxidns-next/webui"
      index: "index.html"
```

Then open:

```text
http://server-ip:9199/
```

The WebUI calls same-origin endpoints such as `/api/health`, `/api/config`, and `/api/plugins/...`. Static files can be read publicly, while protected `/api/*` endpoints require the HttpOnly session cookie created by login. Mutating requests also validate a CSRF token.

The default release configuration uses `type: accounts`. When the account database is empty, create the first local administrator from the machine running the listener. Remote bootstrap should inject its one-time token through `bootstrap_token_env`; remove that environment variable and reload after initialization. Use `client_secret_env` for OIDC client secrets as well. Never put administrator passwords, bootstrap tokens, OIDC client secrets, or session tokens in YAML or browser storage.

### Account security options

- **Local login**: passwords are stored as Argon2 hashes in a separate SQLite account database and are not stored by the browser.
- **TOTP**: bind an authenticator from Settings and save the one-time recovery codes. Once enabled, password login continues to a second verification step.
- **Passkeys**: require HTTPS or a browser-recognized local secure context. Public deployments should set `public_url`, or explicitly configure `passkey.rp_id` and `passkey.origins`.
- **OIDC**: uses discovery, state, nonce, and PKCE. Every `allowed_users` entry explicitly maps an identity-provider claim to an existing local account; OIDC never creates an administrator implicitly.

Field notes:

- `api.http.webui.root`
  - WebUI static file directory. This requires the expanded `api.http` form.
  - Relative paths resolve against OxiDNS Next `-d/--working-dir`, not against the configuration file directory.
  - The shorthand `api.http: "ip:port"` only configures the listen address and cannot mount WebUI files.
- `api.http.webui.index`
  - Index file name. Defaults to `index.html`.
  - `/`, directory paths, and unmatched frontend deep links fall back to this file.

Static serving behavior:

- `/api` and `/api/*` always go to the management API and never fall back to the WebUI.
- Non-API `GET`/`HEAD` requests look up static files.
- Unmatched non-API paths return `index.html`, so refreshing frontend routes such as `/settings` works.
- The backend rejects path traversal attempts such as `..`, absolute paths, and invalid percent-decoded paths.

## Mode 2: Standalone nginx Deployment

In this mode, OxiDNS Next listens only on a local management API address, and nginx exposes the WebUI plus `/api` proxy externally.

OxiDNS Next can use a simple API-only config:

```yaml
api:
  http:
    listen: "127.0.0.1:9199"
    auth:
      type: accounts
      database: "./data/oxidns-next-auth.db"
      public_url: "https://oxidns-next.example.com"
```

nginx example:

```nginx
server {
    listen 80;
    server_name oxidns-next.example.com;

    root /etc/oxidns-next/webui;
    index index.html;

    location = /api {
        proxy_pass http://127.0.0.1:9199;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    location /api/ {
        proxy_pass http://127.0.0.1:9199;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    location / {
        try_files $uri $uri/ /index.html;
    }
}
```

Here `proxy_pass http://127.0.0.1:9199;` forwards browser requests such as `/api/health` to the backend unchanged. Do not use a form that strips the `/api` prefix. The OxiDNS Next backend only accepts API routes under `/api/*`.

If nginx terminates HTTPS, add `listen 443 ssl`, certificates, and the HTTP-to-HTTPS redirect in nginx. The OxiDNS Next backend can keep listening on plain `127.0.0.1:9199` because it is not exposed publicly.

## WebUI Backend URL

Keep the WebUI backend URL at the default value:

```text
/api
```

This works for:

- Backend-hosted WebUI.
- nginx, caddy, or another gateway proxying `/api/*` to OxiDNS Next.
- Docker Compose setups with a single entry container for WebUI and API.

Only use an absolute URL for development or temporary debugging, for example:

```text
http://192.168.1.10:9199/api
```

Absolute URLs trigger browser CORS rules, so the backend must allow the WebUI origin. The default CORS inference below is used only when authentication is disabled:

- `listen: "0.0.0.0:9199"` or `listen: "[::]:9199"` allows any origin.
- Listening on a specific IP allows any WebUI port on the same host.
- When `api.http.cors.allowed_origins` is configured explicitly, entries are matched exactly against the browser `Origin`.

With account authentication enabled, a cross-origin WebUI must list exact `allowed_origins`; a same-origin deployment needs no extra CORS configuration.

## Reverse Proxy And Authentication Notes

Account sessions use HttpOnly cookies, so the WebUI and `/api/*` should remain same-origin. OxiDNS Next does not provide a public hosted Console; do not expose the management API for arbitrary third-party pages to call directly.

OIDC and passkey deployments must configure `public_url` with the final browser-visible HTTPS address. The reverse proxy should preserve `Host` and restrict the backend listener to trusted proxy traffic. OxiDNS Next does not trust `X-Forwarded-*` to infer the authentication origin or scheme; `public_url` is authoritative.

## Common Checks

- Opening `http://server:9199/` or the nginx domain shows the WebUI.
- Browser Network shows `/api/health` returning `200`, not a static file.
- On first use, `/api/auth/session` reports that setup is required. After creating the local administrator, refresh into the login screen.
- After login, the browser should receive an HttpOnly session cookie, and mutating requests should include `X-CSRF-Token`. Do not store passwords or session tokens in browser localStorage.
- If passkeys fail, confirm the page uses HTTPS (or localhost) and that `public_url`, the RP ID, and the origin agree.
- If refreshing `/settings`, `/query-log`, or the compatibility route `/plugins` returns 404, the static server is missing the `index.html` fallback. For nginx, check `try_files $uri $uri/ /index.html;`.
- If `/api/health` returns 404 through nginx, check whether `proxy_pass` accidentally strips the `/api` prefix.
