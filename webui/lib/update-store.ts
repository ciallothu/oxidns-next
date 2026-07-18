"use client";

import { create } from "zustand";
import {
  fetchBuildInfo,
  fetchHealth,
  fetchUpgradeCheck,
  fetchUpgradeStatus,
  triggerUpgradeApply,
} from "./oxidns-next-api";
import { WEBUI, tClient } from "./i18n";
import {
  createProcessInstanceBaseline,
  hasProcessIdentityBaseline,
  processInstanceChanged,
  type ProcessInstanceBaseline,
} from "./process-instance";
import { useAppStore } from "./store";
import {
  assertApiSessionCurrent,
  captureApiSession,
  isApiSessionCurrent,
  isSupersededApiRequest,
} from "./api-client";

const STORAGE_KEY_PREFIX = "oxidns-next:upgrade-config:v2";
const LEGACY_STORAGE_KEYS = [
  "oxidns:upgrade-config",
  "oxidns-next:upgrade-config",
];

export type UpgradeBundle = "auto" | "full" | "minimal" | "standard";

export interface UpgradeConfig {
  repository: string;
  bundle: UpgradeBundle;
  outbound: string;
  socks5: string;
  githubToken: string;
  allowPrerelease: boolean;
  autoCheck: boolean;
}

export const DEFAULT_UPGRADE_CONFIG: UpgradeConfig = {
  repository: "ciallothu/oxidns-next",
  bundle: "auto",
  outbound: "",
  socks5: "",
  githubToken: "",
  allowPrerelease: false,
  autoCheck: true,
};

type PersistedUpgradeConfig = Omit<
  UpgradeConfig,
  "githubToken" | "socks5"
>;

export interface UpdateInfo {
  currentVersion: string;
  latestVersion: string;
  updateAvailable: boolean;
  assetName: string;
  releaseUrl: string;
}

export type UpgradeApplyPhase =
  | "requesting"
  | "applying"
  | "waiting_up"
  | "verifying"
  | "completed";

interface UpdateState {
  upgradeConfigScope: string | null;
  upgradeSessionGeneration: number | null;
  upgradeConfig: UpgradeConfig;
  updateInfo: UpdateInfo | null;
  isChecking: boolean;
  isApplying: boolean;
  applyPhase: UpgradeApplyPhase | null;
  lastCheckedAt: number | null;
  lastAppliedVersion: string | null;
  checkError: string | null;
  applyError: string | null;

  syncSessionScope: (
    serverUrl: string,
    userId: string | null,
    sessionGeneration: number,
  ) => void;
  setUpgradeConfig: (config: Partial<UpgradeConfig>) => void;
  checkForUpdates: (currentVersion: string) => Promise<void>;
  triggerUpgrade: () => Promise<void>;
  resetApplyState: () => void;
}

function loadUpgradeConfig(storageKey: string): UpgradeConfig {
  try {
    const stored = localStorage.getItem(storageKey);
    if (stored) {
      const parsed = JSON.parse(stored) as Partial<UpgradeConfig>;
      return {
        ...DEFAULT_UPGRADE_CONFIG,
        ...pickPersistedUpgradeConfig(parsed),
        // Both fields can carry credentials and are deliberately memory-only,
        // even if an older scoped value somehow contains them.
        githubToken: "",
        socks5: "",
      };
    }
  } catch {
    // ignore
  }
  return { ...DEFAULT_UPGRADE_CONFIG };
}

function saveUpgradeConfig(storageKey: string, config: UpgradeConfig): void {
  try {
    localStorage.setItem(
      storageKey,
      JSON.stringify(pickPersistedUpgradeConfig(config)),
    );
  } catch {
    // ignore
  }
}

function pickPersistedUpgradeConfig(
  config: Partial<UpgradeConfig>,
): Partial<PersistedUpgradeConfig> {
  return {
    ...(config.repository !== undefined
      ? { repository: config.repository }
      : {}),
    ...(config.bundle !== undefined ? { bundle: config.bundle } : {}),
    ...(config.outbound !== undefined ? { outbound: config.outbound } : {}),
    ...(config.allowPrerelease !== undefined
      ? { allowPrerelease: config.allowPrerelease }
      : {}),
    ...(config.autoCheck !== undefined ? { autoCheck: config.autoCheck } : {}),
  };
}

function upgradeConfigStorageKey(serverUrl: string, userId: string) {
  const normalizedServer = serverUrl.trim().replace(/\/$/, "");
  return `${STORAGE_KEY_PREFIX}:${encodeURIComponent(normalizedServer)}:${encodeURIComponent(userId)}`;
}

function clearLegacyUpgradeConfig() {
  if (typeof window === "undefined") return;
  for (const key of LEGACY_STORAGE_KEYS) window.localStorage.removeItem(key);
}

function clearedRuntimeState() {
  return {
    updateInfo: null,
    isChecking: false,
    isApplying: false,
    applyPhase: null,
    lastCheckedAt: null,
    lastAppliedVersion: null,
    checkError: null,
    applyError: null,
  };
}

export const useUpdateStore = create<UpdateState>((set, get) => ({
  upgradeConfigScope: null,
  upgradeSessionGeneration: null,
  upgradeConfig: { ...DEFAULT_UPGRADE_CONFIG },
  updateInfo: null,
  isChecking: false,
  isApplying: false,
  applyPhase: null,
  lastCheckedAt: null,
  lastAppliedVersion: null,
  checkError: null,
  applyError: null,

  syncSessionScope: (serverUrl, userId, sessionGeneration) => {
    clearLegacyUpgradeConfig();
    const nextScope = userId
      ? upgradeConfigStorageKey(serverUrl, userId)
      : null;
    if (
      get().upgradeConfigScope === nextScope &&
      get().upgradeSessionGeneration === sessionGeneration
    ) {
      return;
    }

    set({
      upgradeConfigScope: nextScope,
      upgradeSessionGeneration: sessionGeneration,
      upgradeConfig: nextScope
        ? loadUpgradeConfig(nextScope)
        : { ...DEFAULT_UPGRADE_CONFIG },
      ...clearedRuntimeState(),
    });
  },

  setUpgradeConfig: (partial) => {
    const next = { ...get().upgradeConfig, ...partial };
    const scope = get().upgradeConfigScope;
    if (scope) saveUpgradeConfig(scope, next);
    set({ upgradeConfig: next });
  },

  checkForUpdates: async (currentVersion: string) => {
    const session = captureApiSession();
    const { upgradeConfig } = get();
    set({ isChecking: true, checkError: null });
    try {
      const result = await fetchUpgradeCheck({
        repository: upgradeConfig.repository,
        bundle: upgradeConfig.bundle,
        outbound: upgradeConfig.outbound || undefined,
        socks5: upgradeConfig.socks5 || undefined,
        githubToken: upgradeConfig.githubToken.trim() || undefined,
        allowPrerelease: upgradeConfig.allowPrerelease,
      });
      assertApiSessionCurrent(session);
      set({
        updateInfo: {
          currentVersion,
          latestVersion: result.latest_version,
          updateAvailable: result.update_available,
          assetName: result.asset_name,
          releaseUrl: result.release_url,
        },
        lastCheckedAt: Date.now(),
        isChecking: false,
      });
    } catch (error) {
      if (
        isSupersededApiRequest(error) ||
        !isApiSessionCurrent(session)
      ) {
        return;
      }
      set({
        checkError:
          error instanceof Error
            ? error.message
            : tClient(WEBUI.storeErrors.updateCheckFailed),
        isChecking: false,
        lastCheckedAt: Date.now(),
      });
    }
  },

  triggerUpgrade: async () => {
    const session = captureApiSession();
    const { upgradeConfig, updateInfo } = get();
    const targetVersion = updateInfo?.latestVersion ?? null;
    let baseline = createProcessInstanceBaseline();
    try {
      baseline = createProcessInstanceBaseline(await fetchHealth());
    } catch (error) {
      if (
        isSupersededApiRequest(error) ||
        !isApiSessionCurrent(session)
      ) {
        return;
      }
      // Upgrade completion can still be detected through a temporary outage or
      // a fresh uptime signature if the initial health probe is unavailable.
    }

    set({
      isApplying: true,
      applyPhase: "requesting",
      applyError: null,
      lastAppliedVersion: null,
    });
    try {
      await triggerUpgradeApply({
        repository: upgradeConfig.repository,
        bundle: upgradeConfig.bundle,
        outbound: upgradeConfig.outbound || undefined,
        socks5: upgradeConfig.socks5 || undefined,
        githubToken: upgradeConfig.githubToken.trim() || undefined,
        allowPrerelease: upgradeConfig.allowPrerelease,
      });
      const installedVersion = await pollUpgradeCompletion({
        baseline,
        targetVersion,
        assertCurrent: () => assertApiSessionCurrent(session),
        onPhase: (phase) => {
          if (isApiSessionCurrent(session)) set({ applyPhase: phase });
        },
      });
      assertApiSessionCurrent(session);
      await useAppStore.getState().refreshRuntimeState();
      assertApiSessionCurrent(session);
      set((state) => ({
        applyPhase: "completed",
        isApplying: false,
        lastAppliedVersion: installedVersion,
        updateInfo: state.updateInfo
          ? {
              ...state.updateInfo,
              currentVersion: installedVersion,
              latestVersion: installedVersion,
              updateAvailable: false,
            }
          : state.updateInfo,
      }));

      // The backend may have replaced the bundled WebUI assets too. Reloading
      // after a verified backend version keeps the console code in sync.
      await delay(1200);
      assertApiSessionCurrent(session);
      if (typeof window !== "undefined") window.location.reload();
    } catch (error) {
      if (
        isSupersededApiRequest(error) ||
        !isApiSessionCurrent(session)
      ) {
        return;
      }
      set({
        applyError:
          error instanceof Error
            ? error.message
            : tClient(WEBUI.storeErrors.upgradeStartFailed),
        isApplying: false,
        applyPhase: null,
      });
    }
  },

  resetApplyState: () =>
    set({
      isApplying: false,
      applyPhase: null,
      applyError: null,
      lastAppliedVersion: null,
    }),
}));

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

class UpgradeApplyFailedError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "UpgradeApplyFailedError";
  }
}

const UPGRADE_APPLY_TIMEOUT_MS = 10 * 60_000;
const UPGRADE_RECONNECT_TIMEOUT_MS = 2 * 60_000;

async function pollUpgradeCompletion({
  baseline,
  targetVersion,
  assertCurrent,
  onPhase,
}: {
  baseline: ProcessInstanceBaseline;
  targetVersion: string | null;
  assertCurrent: () => void;
  onPhase: (phase: UpgradeApplyPhase) => void;
}): Promise<string> {
  let sawDown = false;

  onPhase("applying");
  const applyDeadline = Date.now() + UPGRADE_APPLY_TIMEOUT_MS;
  while (Date.now() < applyDeadline) {
    await delay(1500);
    assertCurrent();
    try {
      const status = await fetchUpgradeStatus();
      if (status.state === "failed") {
        throw new UpgradeApplyFailedError(
          status.error ?? tClient(WEBUI.storeErrors.upgradeFailed),
        );
      }
      if (status.state === "skipped" || status.state === "completed") {
        return status.installed_version ?? targetVersion ?? "";
      }
      if (status.state === "restarting") {
        break;
      }

      const health = await fetchHealth();
      if (
        targetVersion &&
        versionsEqual(health.version, targetVersion) &&
        processInstanceChanged(health, baseline)
      ) {
        return verifyUpgradeVersion(
          targetVersion,
          health.version,
          onPhase,
          assertCurrent,
        );
      }
    } catch (error) {
      if (
        error instanceof UpgradeApplyFailedError ||
        isSupersededApiRequest(error)
      ) {
        throw error;
      }
      sawDown = true;
      break;
    }
  }

  if (!sawDown && Date.now() >= applyDeadline) {
    throw new Error(tClient(WEBUI.storeErrors.upgradeRestartNotObserved));
  }

  onPhase("waiting_up");
  const reconnectDeadline = Date.now() + UPGRADE_RECONNECT_TIMEOUT_MS;
  while (Date.now() < reconnectDeadline) {
    await delay(1500);
    assertCurrent();
    try {
      const health = await fetchHealth();
      const fresh =
        processInstanceChanged(health, baseline) ||
        (sawDown && !hasProcessIdentityBaseline(baseline));
      if (!fresh) continue;
      return verifyUpgradeVersion(
        targetVersion,
        health.version,
        onPhase,
        assertCurrent,
      );
    } catch (error) {
      if (isSupersededApiRequest(error)) throw error;
      sawDown = true;
      // The service is still starting.
    }
  }

  throw new Error(tClient(WEBUI.storeErrors.upgradeRestartTimeout));
}

async function verifyUpgradeVersion(
  targetVersion: string | null,
  healthVersion: string,
  onPhase: (phase: UpgradeApplyPhase) => void,
  assertCurrent?: () => void,
): Promise<string> {
  onPhase("verifying");
  const verifyDeadline = Date.now() + 45_000;
  let lastVersion = healthVersion;

  while (Date.now() < verifyDeadline) {
    assertCurrent?.();
    try {
      const [{ build }, health] = await Promise.all([
        fetchBuildInfo(),
        fetchHealth(),
      ]);
      lastVersion = build.version || health.version || lastVersion;
      if (!targetVersion || versionsEqual(lastVersion, targetVersion)) {
        return lastVersion;
      }
    } catch (error) {
      if (isSupersededApiRequest(error)) throw error;
      // API routes may still be warming up immediately after process start.
    }
    await delay(1000);
  }

  throw new Error(
    tClient(WEBUI.storeErrors.upgradeVerifyTimeout, {
      version: targetVersion ?? lastVersion,
    }),
  );
}

function versionsEqual(left: string, right: string): boolean {
  return normalizeVersion(left) === normalizeVersion(right);
}

function normalizeVersion(version: string): string {
  return version.trim().replace(/^v/i, "");
}
