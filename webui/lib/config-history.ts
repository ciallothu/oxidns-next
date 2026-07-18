"use client";

// Client-only config snapshot history. Persisted in localStorage and scoped
// per OxiDNS Next instance (server URL + config path) so multiple backends do not
// share history. The backend stores nothing; history lives on the device.
//
// Safe snapshots store the *raw* YAML text because serde_yaml_ng round-trips
// are lossy (comments / key order). Configs containing inline credentials are
// deliberately excluded: passwords, tokens and secrets must never be copied
// into persistent browser storage. Environment-variable references remain
// eligible because the referenced value is not present in the YAML.

import { useAuthStore } from "./auth-store";
import { parseDocument } from "yaml";

export type ApplyStatus =
  | "not-applied"
  | "applying"
  | "applied"
  | "apply-failed";

export interface ConfigSnapshot {
  id: string;
  createdAt: number;
  content: string;
  version: string;
  source: "server" | "save";
  pluginCount: number;
  size: number;
  applyStatus: ApplyStatus;
  applyError?: string;
  appliedAt?: number;
}

export interface RecordSnapshotInput {
  content: string;
  version: string;
  source: ConfigSnapshot["source"];
  pluginCount: number;
  applyStatus: ApplyStatus;
}

const KEY_PREFIX = "oxidns-next:config-history:";
const MAX_ENTRIES = 30;

export function getScopeKey(configPath: string): string {
  let serverUrl = "";
  let userId = "anonymous";
  try {
    const auth = useAuthStore.getState();
    serverUrl = auth.serverConfig.url.trim();
    userId = auth.user?.id ?? "anonymous";
  } catch {
    serverUrl = "";
  }
  return `${serverUrl}|${userId}|${configPath}`;
}

function storageKey(scope: string) {
  return `${KEY_PREFIX}${scope}`;
}

// One entry per version (= per content hash), keeping the most recent
// occurrence, newest first. Live status (running / pending) is derived at render
// time from runningVersion / configVersion, NOT from a frozen per-entry
// applyStatus, so an old version never gets stuck looking "applied".
function dedupeByVersion(list: ConfigSnapshot[]): ConfigSnapshot[] {
  const newest = new Map<string, ConfigSnapshot>();
  for (const s of list) {
    const cur = newest.get(s.version);
    if (!cur || s.createdAt > cur.createdAt) newest.set(s.version, s);
  }
  return [...newest.values()].sort((a, b) => b.createdAt - a.createdAt);
}

export function listSnapshots(scope: string): ConfigSnapshot[] {
  if (typeof window === "undefined") return [];
  try {
    const raw = window.localStorage.getItem(storageKey(scope));
    if (!raw) return [];
    const parsed = JSON.parse(raw) as ConfigSnapshot[];
    if (!Array.isArray(parsed)) return [];
    const safe = parsed.filter(
      (snapshot) =>
        typeof snapshot.content === "string" &&
        isSafeSnapshotContent(snapshot.content),
    );
    // Upgrade cleanup: older builds persisted complete YAML files, including
    // inline Basic/OIDC/plugin credentials. Remove those entries as soon as
    // this authenticated scope is read.
    if (safe.length !== parsed.length) {
      try {
        window.localStorage.setItem(storageKey(scope), JSON.stringify(safe));
      } catch {
        // The unsafe entries are still excluded from the in-memory result.
      }
    }
    return dedupeByVersion(safe);
  } catch {
    return [];
  }
}

function persist(scope: string, list: ConfigSnapshot[]): ConfigSnapshot[] {
  if (typeof window === "undefined") return list;
  try {
    window.localStorage.setItem(storageKey(scope), JSON.stringify(list));
  } catch {
    // Quota exceeded or storage disabled — degrade silently. The returned
    // list is still used for in-memory state this session.
  }
  return list;
}

function trim(list: ConfigSnapshot[]): ConfigSnapshot[] {
  if (list.length <= MAX_ENTRIES) return list;
  const kept = list.slice(0, MAX_ENTRIES);
  if (kept.some((s) => s.applyStatus === "applied")) return kept;
  // Always keep the most recent known-good config as a rollback anchor even
  // when it would otherwise age out of the window.
  const lastGood = list.find((s) => s.applyStatus === "applied");
  if (!lastGood) return kept;
  return [...kept.slice(0, MAX_ENTRIES - 1), lastGood];
}

export function recordSnapshot(
  scope: string,
  input: RecordSnapshotInput,
): ConfigSnapshot[] {
  if (!isSafeSnapshotContent(input.content)) {
    return listSnapshots(scope);
  }
  // Upsert by version: a re-save of identical content is still a real event,
  // so drop any prior entry for this version and re-add it fresh at the head
  // (newest timestamp) instead of silently skipping it.
  const list = listSnapshots(scope).filter((s) => s.version !== input.version);
  const createdAt = Date.now();
  const entry: ConfigSnapshot = {
    id: `${createdAt}-${input.version.slice(0, 8)}`,
    createdAt,
    content: input.content,
    version: input.version,
    source: input.source,
    pluginCount: input.pluginCount,
    size: input.content.length,
    applyStatus: input.applyStatus,
    appliedAt: input.applyStatus === "applied" ? createdAt : undefined,
  };
  return persist(scope, trim([entry, ...list]));
}

const SENSITIVE_KEY =
  /(?:^|_)(?:password|passwd|passphrase|secret|token|authorization|api_key|private_key|credential|credentials|cookie|bearer|access_key)$/i;
const SENSITIVE_VALUE_KEY =
  /(?:^|_)(?:password|passwd|passphrase|secret|token|authorization|credential|credentials|cookie|bearer)_(?:value|hash|data|json|contents|header)$/i;
const ENV_REFERENCE = /^\$\{[A-Za-z_][A-Za-z0-9_]*\}$/;
const URI_CREDENTIALS =
  /(?:[a-z][a-z0-9+.-]*:\/\/)?[^\s/:@]+:[^\s/@]+@[^\s]+/i;

/** Return false when raw YAML would persist an inline credential. */
export function isSafeSnapshotContent(content: string): boolean {
  let value: unknown;
  try {
    const document = parseDocument(content);
    if (document.errors.length > 0) return false;
    value = document.toJS();
  } catch {
    return false;
  }
  return !containsInlineCredential(value);
}

function containsInlineCredential(value: unknown): boolean {
  if (Array.isArray(value)) return value.some(containsInlineCredential);
  if (!value || typeof value !== "object") {
    return typeof value === "string" && URI_CREDENTIALS.test(value);
  }

  return Object.entries(value as Record<string, unknown>).some(
    ([key, child]) => {
      const normalizedKey = normalizeConfigKey(key);
      if (
        !normalizedKey.endsWith("_env") &&
        (SENSITIVE_KEY.test(normalizedKey) ||
          SENSITIVE_VALUE_KEY.test(normalizedKey)) &&
        hasInlineValue(child)
      ) {
        return true;
      }
      return containsInlineCredential(child);
    },
  );
}

function normalizeConfigKey(key: string): string {
  return key
    .trim()
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/[^A-Za-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "")
    .toLowerCase();
}

function hasInlineValue(value: unknown): boolean {
  if (value === null || value === undefined || value === "") return false;
  if (typeof value === "string") return !ENV_REFERENCE.test(value.trim());
  if (Array.isArray(value)) return value.length > 0;
  if (typeof value === "object") return Object.keys(value).length > 0;
  return true;
}

export function annotateApply(
  scope: string,
  version: string,
  status: ApplyStatus,
  error?: string,
): ConfigSnapshot[] {
  let done = false;
  const next = listSnapshots(scope).map((s) => {
    if (done || s.version !== version) return s;
    done = true;
    return {
      ...s,
      applyStatus: status,
      applyError: status === "apply-failed" ? error : undefined,
      appliedAt: status === "applied" ? Date.now() : s.appliedAt,
    };
  });
  return persist(scope, next);
}

export function deleteSnapshot(scope: string, id: string): ConfigSnapshot[] {
  return persist(
    scope,
    listSnapshots(scope).filter((s) => s.id !== id),
  );
}

export function clearSnapshots(scope: string): ConfigSnapshot[] {
  if (typeof window !== "undefined") {
    try {
      window.localStorage.removeItem(storageKey(scope));
    } catch {
      // ignore
    }
  }
  return [];
}
