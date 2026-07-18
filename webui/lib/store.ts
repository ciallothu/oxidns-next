"use client";

import { create } from "zustand";
import type { PluginInstance } from "./types";
import {
  configFromPlugins,
  createDefaultOxiDnsNextConfig,
  parseOxiDnsNextYaml,
  pluginsFromConfig,
  serializePluginsPreserving,
  stringifyOxiDnsNextConfig,
  topLevelConfigChanged,
  type OxiDnsNextConfig,
} from "./oxidns-next-config";
import {
  fetchBuildInfo,
  fetchControl,
  fetchConfigFile,
  fetchHealth,
  fetchPrometheusMetrics,
  fetchReloadStatus,
  fetchSystem,
  requestReload,
  requestRestart,
  saveConfigFile,
  validateConfigText,
  type BuildInfo,
  type ConfigFileResponse,
  type ConfigValidateResponse,
  type ControlResponse,
  type DependencyGraphReport,
  type HealthResponse,
  type ReloadSnapshot,
  type SystemResponse,
} from "./oxidns-next-api";
import {
  parsePrometheusMetrics,
  type OutboundMetricsMap,
  type PluginMetricsMap,
} from "./metrics";
import {
  getIncomingPluginReferences,
  getReplacementCandidates,
  removeSafePluginReferences,
  renamePluginConfigTag,
  replacePluginReferences,
  type PluginReferenceImpact,
} from "./plugin-reference-operations";
import {
  annotateApply,
  clearSnapshots,
  deleteSnapshot,
  getScopeKey,
  listSnapshots,
  recordSnapshot,
  type ConfigSnapshot,
} from "./config-history";
import { WEBUI, tClient } from "./i18n";
import {
  createProcessInstanceBaseline,
  hasProcessIdentityBaseline,
  processInstanceChanged,
  type ProcessInstanceBaseline,
} from "./process-instance";
import {
  captureApiSession,
  isApiSessionCurrent,
  isSupersededApiRequest,
  type ApiSessionSnapshot,
} from "./api-client";

type StoreSet = (
  partial: Partial<AppState> | ((state: AppState) => Partial<AppState>),
) => void;

export type RestartPhase =
  | "saving"
  | "requesting"
  | "waiting_down"
  | "waiting_up"
  | "reloading";

export type PluginDeletePreview =
  | {
      status: "ready";
      plugin: PluginInstance;
      references: PluginReferenceImpact[];
      canRemoveReferences: boolean;
      replacementCandidates: PluginInstance[];
    }
  | { status: "blocked"; message: string };

export type PluginRenameResult =
  | { status: "renamed" }
  | {
      status: "needs-confirmation";
      references: PluginReferenceImpact[];
    }
  | { status: "invalid"; message: string };

interface AppState {
  plugins: PluginInstance[];
  health: HealthResponse | null;
  buildInfo: BuildInfo | null;
  control: ControlResponse | null;
  system: SystemResponse | null;
  reloadStatus: ReloadSnapshot | null;
  pluginMetrics: PluginMetricsMap;
  outboundMetrics: OutboundMetricsMap;
  dependencyGraph: DependencyGraphReport | null;
  configDiagnostics: string[];
  configHistory: ConfigSnapshot[];
  selectedPlugin: PluginInstance | null;
  detailOpen: boolean;
  editorMode: boolean;
  historyOpen: boolean;
  isConfigLoading: boolean;
  isConfigSaving: boolean;
  isApplying: boolean;
  isRestarting: boolean;
  /**
   * Current phase of an in-flight restart, surfaced by the blocking overlay.
   * `null` when no restart is in progress.
   */
  restartPhase: RestartPhase | null;
  configModel: OxiDnsNextConfig;
  configText: string;
  configVersion: string | null;
  /** Version the backend is actually running now (proxy: last loaded/applied). */
  runningVersion: string | null;
  configPath: string;
  configError: string | null;
  yamlConfig: string;
  /** Editing a pasted/uploaded config with no backend connection. */
  isOfflineMode: boolean;
  /** Name of the uploaded file, used as the export download name. */
  offlineFileName: string | null;

  setSelectedPlugin: (plugin: PluginInstance | null) => void;
  setDetailOpen: (open: boolean) => void;
  setEditorMode: (mode: boolean) => void;
  setHistoryOpen: (open: boolean) => void;
  resetBackendState: () => void;
  setYamlConfig: (config: string) => void;
  enterOfflineConfig: (text: string, fileName?: string) => void;
  exitOfflineMode: () => void;
  loadConfig: () => Promise<void>;
  refreshRuntimeState: () => Promise<void>;
  refreshMetrics: () => Promise<void>;
  validateCurrentConfig: () => Promise<void>;
  saveConfig: () => Promise<void>;
  applyConfig: () => Promise<void>;
  restartApp: () => Promise<void>;
  restoreSnapshot: (id: string) => void;
  rollbackToSnapshot: (id: string) => Promise<void>;
  deleteConfigSnapshot: (id: string) => void;
  clearConfigHistory: () => void;
  togglePluginEnabled: (id: string) => void;
  reorderPlugins: (orderedVisibleIds: string[]) => Promise<void>;
  updatePluginConfig: (id: string, config: Record<string, unknown>) => void;
  previewPluginDelete: (id: string) => Promise<PluginDeletePreview>;
  confirmDeletePlugin: (id: string) => Promise<void>;
  replaceAndDeletePlugin: (id: string, replacementTag: string) => Promise<void>;
  removeReferencesAndDeletePlugin: (id: string) => Promise<void>;
  enterEditorForPluginReferences: () => void;
  addPlugin: (
    plugin: Omit<PluginInstance, "id" | "createdAt" | "updatedAt" | "metrics">,
  ) => void;
  renamePlugin: (
    id: string,
    name: string,
    options?: { confirmed?: boolean },
  ) => Promise<PluginRenameResult>;
}

let queuedConfigSave: Promise<void> = Promise.resolve();
let latestConfigSaveId = 0;
let backendStateEpoch = 0;

interface BackendOperation {
  epoch: number;
  session: ApiSessionSnapshot;
}

class ObsoleteBackendOperationError extends Error {
  constructor() {
    super("Backend operation was superseded");
    this.name = "AbortError";
  }
}

function captureBackendOperation(): BackendOperation {
  return { epoch: backendStateEpoch, session: captureApiSession() };
}

function isBackendOperationCurrent(operation: BackendOperation) {
  return (
    operation.epoch === backendStateEpoch &&
    isApiSessionCurrent(operation.session)
  );
}

function assertBackendOperationCurrent(operation: BackendOperation) {
  if (!isBackendOperationCurrent(operation)) {
    throw new ObsoleteBackendOperationError();
  }
}

function isObsoleteBackendError(
  error: unknown,
  operation: BackendOperation,
) {
  return (
    error instanceof ObsoleteBackendOperationError ||
    isSupersededApiRequest(error) ||
    !isBackendOperationCurrent(operation)
  );
}

function enqueueConfigSave(
  set: StoreSet,
  operation: BackendOperation,
  task: () => Promise<void>,
): Promise<void> {
  const saveId = ++latestConfigSaveId;
  if (isBackendOperationCurrent(operation)) set({ isConfigSaving: true });

  const run = () => task();
  const current = queuedConfigSave.then(run, run);
  queuedConfigSave = current.catch(() => {});

  return current.finally(() => {
    if (
      saveId === latestConfigSaveId &&
      isBackendOperationCurrent(operation)
    ) {
      set({ isConfigSaving: false });
    }
  });
}

const initialConfigModel = createDefaultOxiDnsNextConfig();
const initialConfigText = stringifyOxiDnsNextConfig(initialConfigModel);

export const useAppStore = create<AppState>((set, get) => ({
  plugins: [],
  health: null,
  buildInfo: null,
  control: null,
  system: null,
  reloadStatus: null,
  pluginMetrics: {},
  outboundMetrics: {},
  dependencyGraph: null,
  configDiagnostics: [],
  configHistory: [],
  selectedPlugin: null,
  detailOpen: false,
  editorMode: false,
  historyOpen: false,
  isConfigLoading: false,
  isConfigSaving: false,
  isApplying: false,
  isRestarting: false,
  restartPhase: null,
  configModel: initialConfigModel,
  configText: initialConfigText,
  configVersion: null,
  runningVersion: null,
  configPath: "/etc/oxidns-next/config.yaml",
  configError: null,
  yamlConfig: initialConfigText,
  isOfflineMode: false,
  offlineFileName: null,

  setSelectedPlugin: (plugin) => set({ selectedPlugin: plugin }),
  setDetailOpen: (open) => set({ detailOpen: open }),
  setEditorMode: (mode) => set({ editorMode: mode }),
  setHistoryOpen: (open) => set({ historyOpen: open }),
  resetBackendState: () => {
    backendStateEpoch += 1;
    latestConfigSaveId += 1;
    queuedConfigSave = Promise.resolve();
    set({
      plugins: [],
      health: null,
      buildInfo: null,
      control: null,
      system: null,
      reloadStatus: null,
      pluginMetrics: {},
      outboundMetrics: {},
      dependencyGraph: null,
      configDiagnostics: [],
      configHistory: [],
      selectedPlugin: null,
      detailOpen: false,
      editorMode: false,
      historyOpen: false,
      isConfigLoading: false,
      isConfigSaving: false,
      isApplying: false,
      isRestarting: false,
      restartPhase: null,
      configModel: initialConfigModel,
      configText: initialConfigText,
      configVersion: null,
      runningVersion: null,
      configPath: "/etc/oxidns-next/config.yaml",
      configError: null,
      yamlConfig: initialConfigText,
      isOfflineMode: false,
      offlineFileName: null,
    });
  },
  setYamlConfig: (config) => {
    const parsed = parseOxiDnsNextYaml(config);
    if (!parsed.config) {
      set({
        configText: config,
        yamlConfig: config,
        configError:
          parsed.diagnostics[0] ?? tClient(WEBUI.storeErrors.configParseFailed),
        configDiagnostics: parsed.diagnostics,
      });
      return;
    }

    const plugins = pluginsFromConfig(parsed.config);
    set({
      configModel: parsed.config,
      configText: config,
      yamlConfig: config,
      plugins,
      selectedPlugin: syncSelectedPlugin(get().selectedPlugin, plugins),
      configError: parsed.diagnostics[0] ?? null,
      configDiagnostics: parsed.diagnostics,
    });
  },

  // Import a pasted/uploaded config for editing without a backend. Resets
  // every backend-derived field first so stale dependency graphs, history,
  // and (critically) configVersion can't leak in — a stale configVersion
  // would corrupt the editor's dirty/reset baseline. setYamlConfig runs the
  // existing client-side parse path; its set() payload omits the offline
  // keys so the flags below survive.
  enterOfflineConfig: (text, fileName) => {
    set({
      isOfflineMode: true,
      offlineFileName: fileName ?? null,
      configPath: fileName ?? tClient(WEBUI.storeErrors.unnamedOfflineConfig),
      configVersion: null,
      runningVersion: null,
      dependencyGraph: null,
      configHistory: [],
      reloadStatus: null,
      health: null,
      buildInfo: null,
      control: null,
      system: null,
    });
    get().setYamlConfig(text);
  },

  // Leave offline mode. When still disconnected this returns the user to the
  // import screen; on reconnect the layout's loadConfig() authoritatively
  // repopulates config state, so no manual backend restore is needed here.
  exitOfflineMode: () => set({ isOfflineMode: false, offlineFileName: null }),

  loadConfig: async () => {
    const operation = captureBackendOperation();
    set({ isConfigLoading: true, configError: null });
    try {
      const response = await fetchConfigFile();
      if (!isBackendOperationCurrent(operation)) return;
      applyConfigFileResponse(response, set);
      const scope = getScopeKey(response.path);
      recordSnapshot(scope, {
        content: response.content,
        version: response.version,
        source: "server",
        pluginCount: pluginCountOf(response.content),
        applyStatus: "applied",
      });
      // The backend is running exactly what it just served us from disk.
      set({
        configHistory: listSnapshots(scope),
        runningVersion: response.version,
      });
      await get().validateCurrentConfig();
      if (!isBackendOperationCurrent(operation)) return;
      await get().refreshRuntimeState();
    } catch (error) {
      if (isObsoleteBackendError(error, operation)) return;
      set({
        configError:
          error instanceof Error
            ? error.message
            : tClient(WEBUI.storeErrors.readConfigFailed),
      });
    } finally {
      if (isBackendOperationCurrent(operation)) {
        set({ isConfigLoading: false });
      }
    }
  },

  refreshRuntimeState: async () => {
    const operation = captureBackendOperation();
    const results = await Promise.allSettled([
      fetchHealth(),
      fetchControl(),
      fetchSystem(),
      fetchReloadStatus(),
      fetchBuildInfo(),
    ]);
    if (!isBackendOperationCurrent(operation)) return;
    const [health, control, system, reloadStatus, buildInfo] = results;
    const nextReload =
      reloadStatus.status === "fulfilled"
        ? reloadStatus.value
        : get().reloadStatus;
    const nextSystem = system.status === "fulfilled" ? system.value : null;
    const nextBuildInfo =
      buildInfo.status === "fulfilled"
        ? buildInfo.value.build
        : nextSystem
          ? (nextSystem.build ?? null)
          : get().buildInfo;
    set({
      health: health.status === "fulfilled" ? health.value : get().health,
      buildInfo: nextBuildInfo,
      control: control.status === "fulfilled" ? control.value : get().control,
      system: nextSystem ?? get().system,
      reloadStatus: nextReload,
      // The backend authoritatively reports what config it is running; prefer
      // it over the load-time disk-version guess so the "not applied" state
      // survives page reloads. Falls back to the prior value for older
      // backends that don't report running_version.
      ...(nextReload?.running_version
        ? { runningVersion: nextReload.running_version }
        : {}),
    });
    await get().refreshMetrics();
  },

  refreshMetrics: async () => {
    const operation = captureBackendOperation();
    try {
      const text = await fetchPrometheusMetrics();
      if (!isBackendOperationCurrent(operation)) return;
      const metrics = parsePrometheusMetrics(text);
      set({ pluginMetrics: metrics.byTag, outboundMetrics: metrics.outbound });
    } catch (error) {
      if (isObsoleteBackendError(error, operation)) return;
      // Metrics are best-effort observability; keep the last snapshot on
      // transient errors (e.g. API hub torn down during reload).
    }
  },

  validateCurrentConfig: async () => {
    const operation = captureBackendOperation();
    const state = get();
    if (state.configError) return;
    try {
      const response = await validateConfigText(state.configText);
      if (!isBackendOperationCurrent(operation)) return;
      applyConfigValidationResponse(response, set);
    } catch (error) {
      if (isObsoleteBackendError(error, operation)) return;
      const message =
        error instanceof Error
          ? error.message
          : tClient(WEBUI.configEditor.configValidationFailed);
      set({
        configError: message,
        configDiagnostics: [message],
        dependencyGraph: null,
      });
      throw error;
    }
  },

  // Save only. Hot-reload is a separate explicit step (applyConfig) so the
  // disk write and the running-config swap are never coupled.
  saveConfig: () => {
    const operation = captureBackendOperation();
    return enqueueConfigSave(set, operation, async () => {
      // A queued save may not start until after logout or a backend switch.
      // Reject it before reading the replacement scope's state or issuing any
      // request; resetting the queue cannot cancel callbacks already chained
      // onto the previous promise.
      assertBackendOperationCurrent(operation);
      const state = get();
      if (state.configError) throw new Error(state.configError);

      set({ configError: null });
      try {
        const validation = await validateConfigText(state.configText);
        assertBackendOperationCurrent(operation);
        applyConfigValidationResponse(validation, set);
        const content = state.configText;
        const response = await saveConfigFile({
          content,
          baseVersion: state.configVersion,
          validate: true,
          reload: false,
        });
        assertBackendOperationCurrent(operation);
        const scope = getScopeKey(response.path);
        recordSnapshot(scope, {
          content,
          version: response.version,
          source: "save",
          pluginCount: pluginCountOf(content),
          applyStatus: "not-applied",
        });
        set({
          configVersion: response.version,
          configPath: response.path,
          reloadStatus: response.reload ?? get().reloadStatus,
          configHistory: listSnapshots(scope),
        });
        await get().refreshRuntimeState();
      } catch (error) {
        if (isObsoleteBackendError(error, operation)) throw error;
        const message =
          error instanceof Error
            ? error.message
            : tClient(WEBUI.storeErrors.saveConfigFailed);
        set({ configError: message });
        throw error;
      }
    });
  },

  // Trigger a backend hot-reload of the on-disk config and wait for the
  // outcome. The backend already rolls the running pipeline back to the
  // previous in-memory config if assembly fails (src/app.rs handle_reload),
  // so a failed apply leaves the service running on the old config; we only
  // surface that state and annotate the snapshot.
  applyConfig: async () => {
    const operation = captureBackendOperation();
    const before = get();
    const scope = getScopeKey(before.configPath);
    const version = before.configVersion;
    set({ isApplying: true });
    try {
      let baseline: number | undefined;
      try {
        baseline = (await fetchReloadStatus()).last_completed_ms;
        assertBackendOperationCurrent(operation);
      } catch (error) {
        if (isObsoleteBackendError(error, operation)) throw error;
        baseline = undefined;
      }

      let snapshot: ReloadSnapshot;
      try {
        await requestReload();
        assertBackendOperationCurrent(operation);
        snapshot = await pollReload(baseline, () =>
          assertBackendOperationCurrent(operation),
        );
        assertBackendOperationCurrent(operation);
      } catch (error) {
        if (isObsoleteBackendError(error, operation)) throw error;
        // requestReload / polling threw (reload busy, network, API torn down
        // and never recovered) — surface it as a failed apply instead of a
        // silent no-op so the pill turns red rather than staying unchanged.
        const message =
          error instanceof Error
            ? error.message
            : tClient(WEBUI.storeErrors.hotReloadTriggerFailed);
        if (version) {
          annotateApply(scope, version, "apply-failed", message);
          set({ configHistory: listSnapshots(scope) });
        }
        throw new Error(message);
      }

      assertBackendOperationCurrent(operation);
      set({ reloadStatus: snapshot });
      const failed =
        snapshot.status === "failed" || Boolean(snapshot.last_error);
      if (version) {
        annotateApply(
          scope,
          version,
          failed ? "apply-failed" : "applied",
          snapshot.last_error,
        );
        set({
          configHistory: listSnapshots(scope),
          // On success the backend is now running this config. Prefer the
          // authoritative version it reports; fall back to the applied one.
          ...(failed
            ? {}
            : { runningVersion: snapshot.running_version ?? version }),
        });
      }
      await get().refreshRuntimeState();
      assertBackendOperationCurrent(operation);
      if (failed) {
        throw new Error(
          snapshot.last_error ||
            tClient(WEBUI.storeErrors.hotReloadNotSuccessful),
        );
      }
    } finally {
      if (isBackendOperationCurrent(operation)) set({ isApplying: false });
    }
  },

  // Save the current config to disk and restart the server process. After the
  // restart request is accepted the client polls the health endpoint until a
  // fresh backend instance is observed, then reloads the config from it.
  restartApp: async () => {
    const operation = captureBackendOperation();
    set({ isRestarting: true, restartPhase: "saving" });
    let savedVersion: string | null = null;
    try {
      await get().saveConfig();
      assertBackendOperationCurrent(operation);
      savedVersion = get().configVersion;
      let baseline = createProcessInstanceBaseline();
      try {
        baseline = createProcessInstanceBaseline(await fetchHealth());
        assertBackendOperationCurrent(operation);
      } catch (error) {
        if (isObsoleteBackendError(error, operation)) throw error;
        // Health probe failures here are fine; pollReconnect can still use
        // an observed outage or fresh uptime signature as fallback evidence.
      }
      set({ restartPhase: "requesting" });
      await requestRestart();
      assertBackendOperationCurrent(operation);
      await pollReconnect(
        baseline,
        (phase) => {
          if (isBackendOperationCurrent(operation)) set({ restartPhase: phase });
        },
        () => assertBackendOperationCurrent(operation),
      );
      assertBackendOperationCurrent(operation);
      set({ restartPhase: "reloading" });
      await get().loadConfig();
      assertBackendOperationCurrent(operation);
    } catch (error) {
      if (isObsoleteBackendError(error, operation)) throw error;
      if (savedVersion) {
        const scope = getScopeKey(get().configPath);
        annotateApply(
          scope,
          savedVersion,
          "apply-failed",
          error instanceof Error
            ? error.message
            : tClient(WEBUI.storeErrors.restartFailed),
        );
        set({ configHistory: listSnapshots(scope) });
      }
      throw error;
    } finally {
      if (isBackendOperationCurrent(operation)) {
        set({ isRestarting: false, restartPhase: null });
      }
    }
  },

  // Load a historical snapshot back into the editor only. It is NOT persisted
  // or applied; the operator still goes through Save -> Apply, so a rollback
  // also produces its own history entry.
  restoreSnapshot: (id) => {
    const entry = get().configHistory.find((s) => s.id === id);
    if (!entry) return;
    get().setYamlConfig(entry.content);
  },

  // One-click rollback usable in BOTH console and editor mode: load the
  // snapshot, persist it to disk, then choose hot-reload or full restart based
  // on whether the rollback touches restart-only top-level fields.
  rollbackToSnapshot: async (id) => {
    const operation = captureBackendOperation();
    const entry = get().configHistory.find((s) => s.id === id);
    if (!entry) return;
    const running = get().configHistory.find(
      (s) => s.version === get().runningVersion,
    );
    const requiresRestart = Boolean(
      running && topLevelConfigChanged(entry.content, running.content),
    );
    get().setYamlConfig(entry.content);
    await get().saveConfig();
    assertBackendOperationCurrent(operation);
    if (requiresRestart) {
      await get().restartApp();
    } else {
      await get().applyConfig();
    }
    assertBackendOperationCurrent(operation);
  },

  deleteConfigSnapshot: (id) => {
    const scope = getScopeKey(get().configPath);
    deleteSnapshot(scope, id);
    set({ configHistory: listSnapshots(scope) });
  },

  clearConfigHistory: () => {
    const scope = getScopeKey(get().configPath);
    clearSnapshots(scope);
    set({ configHistory: [] });
  },

  togglePluginEnabled: (id) =>
    set((state) => {
      void id;
      const plugins: PluginInstance[] = state.plugins.map((p) => p);
      return { plugins };
    }),

  // Reorder plugins in the config file to match a drag-and-drop arrangement.
  // `orderedVisibleIds` is the new order of the *currently visible* cards
  // (a single type tab, or all of them). Plugins outside that visible subset
  // keep their absolute positions; only the slots the visible plugins occupy
  // are refilled in the new order, so reordering within one type tab never
  // disturbs the relative position of other types. The change is staged into
  // the editor buffer and persisted to disk (mirroring add/edit/delete), then
  // surfaced as an "apply changes" pill for the operator to hot-reload.
  reorderPlugins: async (orderedVisibleIds) => {
    const state = get();
    if (state.configError) return;

    const visible = new Set(orderedVisibleIds);
    const byId = new Map(state.plugins.map((p) => [p.id, p] as const));
    const queue = orderedVisibleIds
      .map((id) => byId.get(id))
      .filter((p): p is PluginInstance => Boolean(p));
    if (queue.length === 0) return;

    let next = 0;
    const reordered = state.plugins.map((p) =>
      visible.has(p.id) ? queue[next++] : p,
    );
    const unchanged = reordered.every((p, i) => p.id === state.plugins[i].id);
    if (unchanged) return;

    // No tags are passed as changed: every plugin reuses its original YAML
    // node verbatim (comments/blank lines preserved) — only the node order
    // changes.
    set(syncPluginsToConfig(state, () => reordered, []));
    if (!get().isOfflineMode) await get().saveConfig();
  },

  updatePluginConfig: (id, config) =>
    set((state) => {
      const tag = state.plugins.find((p) => p.id === id)?.name;
      return syncPluginsToConfig(
        state,
        (plugins) =>
          plugins.map((p) =>
            p.id === id
              ? { ...p, config, updatedAt: new Date().toISOString() }
              : p,
          ),
        tag ? [tag] : [],
      );
    }),

  previewPluginDelete: async (id) => {
    const operation = captureBackendOperation();
    const state = get();
    if (state.configError) {
      return {
        status: "blocked",
        message: tClient(WEBUI.storeErrors.configHasErrorsBeforeDelete),
      };
    }
    const plugin = state.plugins.find((p) => p.id === id);
    if (!plugin) {
      return {
        status: "blocked",
        message: tClient(WEBUI.storeErrors.pluginMissing),
      };
    }

    await get().validateCurrentConfig();
    assertBackendOperationCurrent(operation);
    const latest = get();
    const references = incomingReferences(latest, plugin.name);
    return {
      status: "ready",
      plugin,
      references,
      canRemoveReferences:
        references.length > 0 && references.every((edge) => edge.removable),
      replacementCandidates: replacementCandidates(latest, plugin, references),
    };
  },

  confirmDeletePlugin: async (id) => {
    const operation = captureBackendOperation();
    await get().validateCurrentConfig();
    assertBackendOperationCurrent(operation);
    const state = get();
    const plugin = state.plugins.find((p) => p.id === id);
    if (!plugin) throw new Error(tClient(WEBUI.storeErrors.pluginMissing));
    const references = incomingReferences(state, plugin.name);
    if (references.length > 0) {
      throw new Error(tClient(WEBUI.storeErrors.pluginStillReferenced));
    }
    set((current) => deletePluginFromState(current, id));
    await get().saveConfig();
  },

  replaceAndDeletePlugin: async (id, replacementTag) => {
    const operation = captureBackendOperation();
    await get().validateCurrentConfig();
    assertBackendOperationCurrent(operation);
    const state = get();
    const plugin = state.plugins.find((p) => p.id === id);
    const replacement = state.plugins.find((p) => p.name === replacementTag);
    if (!plugin) throw new Error(tClient(WEBUI.storeErrors.pluginMissing));
    if (!replacement)
      throw new Error(tClient(WEBUI.storeErrors.replacementMissing));
    const references = incomingReferences(state, plugin.name);
    if (
      !replacementCandidates(state, plugin, references).some(
        (candidate) => candidate.name === replacementTag,
      )
    ) {
      throw new Error(tClient(WEBUI.storeErrors.replacementIncompatible));
    }

    const replaced = replacePluginReferences(
      state.configModel,
      references,
      plugin.name,
      replacementTag,
    );
    set((current) => {
      const applied = applyConfigModelToState(current, replaced.config, [
        ...replaced.changedTags,
        plugin.name,
      ]);
      return deletePluginFromState({ ...current, ...applied }, id);
    });
    await get().saveConfig();
  },

  removeReferencesAndDeletePlugin: async (id) => {
    const operation = captureBackendOperation();
    await get().validateCurrentConfig();
    assertBackendOperationCurrent(operation);
    const state = get();
    const plugin = state.plugins.find((p) => p.id === id);
    if (!plugin) throw new Error(tClient(WEBUI.storeErrors.pluginMissing));
    const references = incomingReferences(state, plugin.name);
    if (references.length === 0) {
      set((current) => deletePluginFromState(current, id));
      await get().saveConfig();
      return;
    }
    if (!references.every((edge) => edge.removable)) {
      throw new Error(tClient(WEBUI.storeErrors.unsafeReferences));
    }

    const removed = removeSafePluginReferences(state.configModel, references);
    set((current) => {
      const applied = applyConfigModelToState(current, removed.config, [
        ...removed.changedTags,
        plugin.name,
      ]);
      return deletePluginFromState({ ...current, ...applied }, id);
    });
    await get().saveConfig();
  },

  enterEditorForPluginReferences: () => set({ editorMode: true }),

  addPlugin: (plugin) =>
    set((state) =>
      syncPluginsToConfig(state, (plugins) => [
        ...plugins,
        {
          ...plugin,
          id: plugin.name,
          createdAt: new Date().toISOString(),
          updatedAt: new Date().toISOString(),
          metrics: { calls: 0, avgLatency: 0, errorRate: 0, qps: 0 },
        },
      ]),
    ),

  renamePlugin: async (id, name, options) => {
    const operation = captureBackendOperation();
    const nextName = name.trim();
    const state = get();
    const plugin = state.plugins.find((p) => p.id === id);
    if (!plugin) {
      return {
        status: "invalid",
        message: tClient(WEBUI.storeErrors.pluginMissing),
      };
    }
    if (!nextName) {
      return {
        status: "invalid",
        message: tClient(WEBUI.storeErrors.pluginNameRequired),
      };
    }
    if (nextName === plugin.name) {
      return {
        status: "invalid",
        message: tClient(WEBUI.storeErrors.pluginNameUnchanged),
      };
    }
    if (state.plugins.some((p) => p.id !== id && p.name === nextName)) {
      return {
        status: "invalid",
        message: tClient(WEBUI.storeErrors.pluginNameExists),
      };
    }
    if (state.configError) {
      return {
        status: "invalid",
        message: tClient(WEBUI.storeErrors.configHasErrorsBeforeRename),
      };
    }

    await get().validateCurrentConfig();
    assertBackendOperationCurrent(operation);
    const latest = get();
    const references = incomingReferences(latest, plugin.name);
    if (references.length > 0 && !options?.confirmed) {
      return { status: "needs-confirmation", references };
    }

    const replaced = replacePluginReferences(
      latest.configModel,
      references,
      plugin.name,
      nextName,
    );
    const renamed = renamePluginConfigTag(
      replaced.config,
      plugin.name,
      nextName,
    );
    set((current) =>
      applyConfigModelToState(
        current,
        renamed.config,
        [...replaced.changedTags, ...renamed.changedTags],
        nextName,
      ),
    );
    await get().saveConfig();
    return { status: "renamed" };
  },
}));

function applyConfigFileResponse(response: ConfigFileResponse, set: StoreSet) {
  const parsed = parseOxiDnsNextYaml(response.content);
  if (!parsed.config) {
    set({
      configText: response.content,
      yamlConfig: response.content,
      configVersion: response.version,
      configPath: response.path,
      configError:
        parsed.diagnostics[0] ?? tClient(WEBUI.storeErrors.configParseFailed),
      configDiagnostics: parsed.diagnostics,
    });
    return;
  }

  set({
    configModel: parsed.config,
    configText: response.content,
    yamlConfig: response.content,
    configVersion: response.version,
    configPath: response.path,
    plugins: pluginsFromConfig(parsed.config),
    configError: parsed.diagnostics[0] ?? null,
    configDiagnostics: parsed.diagnostics,
  });
}

function applyConfigValidationResponse(
  response: ConfigValidateResponse,
  set: StoreSet,
) {
  set({
    dependencyGraph: response.dependency_graph,
    configDiagnostics: [],
    configError: null,
  });
}

function syncPluginsToConfig(
  state: AppState,
  update: (plugins: PluginInstance[]) => PluginInstance[],
  changedTags: string[] = [],
) {
  const plugins = update(state.plugins);
  const configModel = configFromPlugins(state.configModel, plugins);
  // Preserve comments/blank lines: only the explicitly changed tags are
  // regenerated; every other plugin keeps its original YAML node verbatim.
  const configText = serializePluginsPreserving(
    state.configText,
    configModel,
    new Set(changedTags),
  );
  return {
    plugins,
    configModel,
    configText,
    yamlConfig: configText,
    selectedPlugin: syncSelectedPlugin(state.selectedPlugin, plugins),
    configError: null,
    configDiagnostics: [],
  };
}

function applyConfigModelToState(
  state: AppState,
  configModel: OxiDnsNextConfig,
  changedTags: string[],
  selectedTag?: string | null,
) {
  const plugins = pluginsFromConfig(configModel);
  const configText = serializePluginsPreserving(
    state.configText,
    configModel,
    new Set(changedTags),
  );
  return {
    plugins,
    configModel,
    configText,
    yamlConfig: configText,
    selectedPlugin:
      selectedTag === null
        ? null
        : selectedTag
          ? (plugins.find((plugin) => plugin.name === selectedTag) ?? null)
          : syncSelectedPlugin(state.selectedPlugin, plugins),
    configError: null,
    configDiagnostics: [],
  };
}

function deletePluginFromState(state: AppState, id: string) {
  const plugin = state.plugins.find((p) => p.id === id);
  if (!plugin) return {};
  const configModel: OxiDnsNextConfig = {
    ...state.configModel,
    plugins: state.configModel.plugins.filter((p) => p.tag !== plugin.name),
  };
  const selectedWasDeleted = state.selectedPlugin?.id === id;
  return {
    ...applyConfigModelToState(
      state,
      configModel,
      [plugin.name],
      selectedWasDeleted ? null : undefined,
    ),
    detailOpen: selectedWasDeleted ? false : state.detailOpen,
  };
}

function incomingReferences(state: AppState, tag: string) {
  return getIncomingPluginReferences(
    state.plugins,
    state.dependencyGraph?.edges,
    tag,
  );
}

function replacementCandidates(
  state: AppState,
  plugin: PluginInstance,
  references: PluginReferenceImpact[],
) {
  return getReplacementCandidates(state.plugins, plugin.id, references);
}

function syncSelectedPlugin(
  selectedPlugin: PluginInstance | null,
  plugins: PluginInstance[],
) {
  if (!selectedPlugin) return null;
  return plugins.find((plugin) => plugin.id === selectedPlugin.id) ?? null;
}

function pluginCountOf(text: string): number {
  return parseOxiDnsNextYaml(text).config?.plugins.length ?? 0;
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// Wait for positive evidence that the backend instance changed after a restart
// request. Unix restarts use exec(), so the API may never be observably down.
async function pollReconnect(
  baseline: ProcessInstanceBaseline,
  onPhase?: (phase: "waiting_down" | "waiting_up") => void,
  assertCurrent?: () => void,
): Promise<void> {
  let sawDown = false;

  onPhase?.("waiting_down");
  const deadline = Date.now() + 90_000;
  while (Date.now() < deadline) {
    await delay(sawDown ? 1500 : 800);
    assertCurrent?.();
    try {
      const health = await fetchHealth();
      assertCurrent?.();
      const fresh =
        processInstanceChanged(health, baseline) ||
        (sawDown && !hasProcessIdentityBaseline(baseline));
      if (fresh) {
        return;
      }
    } catch (error) {
      assertCurrent?.();
      if (isSupersededApiRequest(error)) throw error;
      sawDown = true;
      onPhase?.("waiting_up");
    }
  }

  throw new Error(
    tClient(
      sawDown
        ? WEBUI.storeErrors.restartTimeout
        : WEBUI.storeErrors.restartNotObserved,
    ),
  );
}

// Poll the reload status until the backend settles on a new completion.
// During reassembly the API hub is briefly torn down, so transient fetch
// errors are expected and ignored. We treat the reload as done once it is
// no longer pending/in-progress AND a new completion timestamp appeared
// (distinct from the pre-reload baseline), or it explicitly failed.
async function pollReload(
  baselineCompleted?: number,
  assertCurrent?: () => void,
): Promise<ReloadSnapshot> {
  const maxAttempts = 40; // ~30s at 750ms intervals
  let last: ReloadSnapshot | null = null;
  for (let i = 0; i < maxAttempts; i += 1) {
    await delay(750);
    assertCurrent?.();
    try {
      last = await fetchReloadStatus();
      assertCurrent?.();
    } catch (error) {
      assertCurrent?.();
      if (isSupersededApiRequest(error)) throw error;
      continue;
    }
    const settled = !last.pending && !last.in_progress;
    const advanced =
      last.last_completed_ms !== undefined &&
      last.last_completed_ms !== baselineCompleted;
    if (settled && (advanced || last.status === "failed")) return last;
  }
  return last ?? { status: "unknown", pending: false, in_progress: false };
}
