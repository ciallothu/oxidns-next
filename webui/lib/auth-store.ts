"use client";

import { create } from "zustand";
import { persist } from "zustand/middleware";
import { WEBUI, tClient } from "./i18n";

export interface ServerConfig {
  url: string;
}

export interface AuthMethods {
  password: boolean;
  totp: boolean;
  oidc: boolean;
  passkey: boolean;
}

export interface AuthUser {
  id: string;
  username: string;
}

export interface AuthSessionResponse {
  ok: boolean;
  authenticated: boolean;
  setup_required: boolean;
  user?: AuthUser;
  auth_method?: string;
  csrf_token?: string;
  methods: AuthMethods;
}

export interface TotpChallenge {
  challengeId: string;
  expiresIn: number;
  expiresAt: number;
}

interface LoginResponse extends Partial<AuthSessionResponse> {
  ok: boolean;
  requires_totp?: boolean;
  challenge_id?: string;
  expires_in?: number;
  message?: string;
}

export interface AuthState {
  serverConfig: ServerConfig;
  isAuthenticated: boolean;
  isConnected: boolean;
  isConnecting: boolean;
  isHydrated: boolean;
  hasAttemptedAutoConnect: boolean;
  connectionError: string | null;
  needsCredentials: boolean;
  setupRequired: boolean;
  methods: AuthMethods;
  user: AuthUser | null;
  authMethod: string | null;
  csrfToken: string | null;
  totpChallenge: TotpChallenge | null;
  /** Monotonic guard against stale requests invalidating a newer session. */
  sessionGeneration: number;

  setServerConfig: (config: ServerConfig) => void;
  connect: (config?: ServerConfig) => Promise<boolean>;
  attemptAutoConnect: () => Promise<void>;
  refreshSession: () => Promise<boolean>;
  bootstrap: (
    username: string,
    password: string,
    token?: string,
  ) => Promise<boolean>;
  login: (
    username: string,
    password: string,
  ) => Promise<"authenticated" | "totp" | "failed">;
  verifyTotp: (code: string) => Promise<boolean>;
  finishPasskeyLogin: (payload: unknown) => Promise<boolean>;
  cancelTotpChallenge: () => void;
  markHydrated: () => void;
  markSessionExpired: () => void;
  logout: () => Promise<boolean>;
}

const EMPTY_METHODS: AuthMethods = {
  password: false,
  totp: false,
  oidc: false,
  passkey: false,
};

export const useAuthStore = create<AuthState>()(
  persist(
    (set, get) => {
      const applySession = (session: AuthSessionResponse) => {
        set({
          isAuthenticated: session.authenticated,
          isConnected: session.authenticated,
          needsCredentials:
            !session.authenticated && !session.setup_required,
          setupRequired: session.setup_required,
          methods: session.methods ?? EMPTY_METHODS,
          user: session.user ?? null,
          authMethod: session.auth_method ?? null,
          csrfToken: session.csrf_token ?? null,
          totpChallenge: null,
          connectionError: null,
          sessionGeneration: get().sessionGeneration + 1,
        });
      };

      const requestSession = async (config: ServerConfig) => {
        const url = normalizeApiUrl(config.url);
        const response = await fetch(`${url}/auth/session`, {
          method: "GET",
          headers: { Accept: "application/json" },
          credentials: "include",
          cache: "no-store",
        });
        if (!response.ok) {
          throw new Error(
            tClient(WEBUI.storeErrors.connectionHttpFailed, {
              status: response.status,
            }),
          );
        }
        return readAuthJson<AuthSessionResponse>(response);
      };

      return {
        serverConfig: { url: "/api" },
        isAuthenticated: false,
        isConnected: false,
        isConnecting: false,
        isHydrated: false,
        hasAttemptedAutoConnect: false,
        connectionError: null,
        needsCredentials: false,
        setupRequired: false,
        methods: EMPTY_METHODS,
        user: null,
        authMethod: null,
        csrfToken: null,
        totpChallenge: null,
        sessionGeneration: 0,

        setServerConfig: (config) =>
          set((state) => ({
            serverConfig: config,
            ...(state.serverConfig.url === config.url
              ? {}
              : {
                  ...loggedOutState(false, false),
                  sessionGeneration: state.sessionGeneration + 1,
                }),
          })),

        connect: async (config) => {
          const serverConfig = config ?? get().serverConfig;
          const requestGeneration = get().sessionGeneration;
          // Manual reconnects count as connection attempts too. Without this,
          // a failed attempt after changing the backend leaves every route
          // except Settings stuck in the initial "connecting" placeholder.
          set({
            isConnecting: true,
            connectionError: null,
            hasAttemptedAutoConnect: true,
          });
          try {
            const session = await requestSession(serverConfig);
            if (get().sessionGeneration !== requestGeneration) return false;
            set({ serverConfig });
            applySession(session);
            set({ isConnecting: false });
            return session.authenticated;
          } catch (error) {
            if (get().sessionGeneration !== requestGeneration) return false;
            set({
              isConnecting: false,
              connectionError:
                error instanceof Error
                  ? error.message
                  : tClient(WEBUI.storeErrors.connectionFailed),
            });
            return false;
          }
        },

        refreshSession: async () => get().connect(),

        attemptAutoConnect: async () => {
          if (get().hasAttemptedAutoConnect || get().isConnecting) return;
          set({ hasAttemptedAutoConnect: true });
          await get().connect();
        },

        bootstrap: async (username, password, token) => {
          set({ isConnecting: true, connectionError: null });
          try {
            const request = await authFetch("/auth/bootstrap", {
              method: "POST",
              body: JSON.stringify({
                username,
                password,
                ...(token ? { token } : {}),
              }),
            });
            const session = await readAuthJson<AuthSessionResponse>(
              request.response,
            );
            assertAuthRequestCurrent(request);
            assertAuthenticatedSession(session);
            applySession(session);
            set({ isConnecting: false });
            return true;
          } catch (error) {
            if (isSupersededAuthRequest(error)) return false;
            set({
              isConnecting: false,
              connectionError: authErrorMessage(error),
            });
            return false;
          }
        },

        login: async (username, password) => {
          set({ isConnecting: true, connectionError: null });
          try {
            const request = await authFetch("/auth/login", {
              method: "POST",
              body: JSON.stringify({ username, password }),
            });
            const body = await readAuthJson<LoginResponse>(request.response);
            assertAuthRequestCurrent(request);
            if (request.response.status === 202 && body.requires_totp) {
              if (!body.challenge_id) {
                throw new Error(tClient(WEBUI.storeErrors.invalidJsonResponse, {
                  preview: "missing challenge_id",
                }));
              }
              set({
                isConnecting: false,
                totpChallenge: {
                  challengeId: body.challenge_id,
                  expiresIn: body.expires_in ?? 300,
                  expiresAt: Date.now() + (body.expires_in ?? 300) * 1_000,
                },
              });
              return "totp";
            }
            const session = body as AuthSessionResponse;
            assertAuthenticatedSession(session);
            applySession(session);
            set({ isConnecting: false });
            return "authenticated";
          } catch (error) {
            if (isSupersededAuthRequest(error)) return "failed";
            set({
              isConnecting: false,
              connectionError: authErrorMessage(error),
            });
            return "failed";
          }
        },

        verifyTotp: async (code) => {
          const challenge = get().totpChallenge;
          if (!challenge) return false;
          set({ isConnecting: true, connectionError: null });
          try {
            const request = await authFetch("/auth/login/totp", {
              method: "POST",
              body: JSON.stringify({
                challenge_id: challenge.challengeId,
                code,
              }),
            });
            const session = await readAuthJson<AuthSessionResponse>(
              request.response,
            );
            assertAuthRequestCurrent(request);
            assertAuthenticatedSession(session);
            applySession(session);
            set({ isConnecting: false });
            return true;
          } catch (error) {
            if (isSupersededAuthRequest(error)) return false;
            set({
              isConnecting: false,
              connectionError: authErrorMessage(error),
            });
            return false;
          }
        },

        finishPasskeyLogin: async (payload) => {
          set({ isConnecting: true, connectionError: null });
          try {
            const request = await authFetch(
              "/auth/passkeys/login/finish",
              { method: "POST", body: JSON.stringify(payload) },
            );
            const session = await readAuthJson<AuthSessionResponse>(
              request.response,
            );
            assertAuthRequestCurrent(request);
            assertAuthenticatedSession(session);
            applySession(session);
            set({ isConnecting: false });
            return true;
          } catch (error) {
            if (isSupersededAuthRequest(error)) return false;
            set({
              isConnecting: false,
              connectionError: authErrorMessage(error),
            });
            return false;
          }
        },

        cancelTotpChallenge: () =>
          set({ totpChallenge: null, connectionError: null }),

        markHydrated: () => set({ isHydrated: true }),

        markSessionExpired: () =>
          set((state) => ({
            ...loggedOutState(true, true),
            connectionError: tClient(WEBUI.storeErrors.loginExpired),
            sessionGeneration: state.sessionGeneration + 1,
          })),

        logout: async () => {
          const csrfToken = get().csrfToken;
          try {
            await authFetch("/auth/logout", {
              method: "POST",
              headers: csrfToken ? { "X-CSRF-Token": csrfToken } : {},
            });
          } catch (error) {
            if (isSupersededAuthRequest(error)) return false;
            // A network failure means the HttpOnly cookie may still represent a
            // valid server-side session. Keep the local session visible so the
            // operator can retry instead of presenting a false logout.
            set({ connectionError: authErrorMessage(error) });
            return false;
          }
          set((state) => ({
            ...loggedOutState(true, true),
            sessionGeneration: state.sessionGeneration + 1,
          }));
          return true;
        },
      };
    },
    {
      name: "oxidns-next-auth",
      // Only the API address is persisted. Passwords, passkey responses,
      // TOTP challenges, recovery codes and session material never enter
      // browser storage; the server session lives in an HttpOnly cookie.
      partialize: (state) => ({ serverConfig: state.serverConfig }),
      onRehydrateStorage: () => {
        clearLegacyCredentialStorage();
        return (state) => state?.markHydrated();
      },
    },
  ),
);

function loggedOutState(
  needsCredentials: boolean,
  hasAttemptedAutoConnect: boolean,
) {
  return {
    isAuthenticated: false,
    isConnected: false,
    isConnecting: false,
    needsCredentials,
    setupRequired: false,
    user: null,
    authMethod: null,
    csrfToken: null,
    totpChallenge: null,
    methods: EMPTY_METHODS,
    hasAttemptedAutoConnect,
    connectionError: null,
  };
}

function normalizeApiUrl(url: string) {
  const trimmed = url.trim();
  if (!trimmed) {
    throw new Error(tClient(WEBUI.storeErrors.serviceUrlRequired));
  }
  return trimmed.replace(/\/$/, "");
}

interface AuthFetchResult {
  response: Response;
  generation: number;
  base: string;
}

class SupersededAuthRequestError extends Error {
  constructor() {
    super("Authentication request was superseded");
    this.name = "SupersededAuthRequestError";
  }
}

async function authFetch(
  path: string,
  init: RequestInit,
): Promise<AuthFetchResult> {
  const state = useAuthStore.getState();
  const base = normalizeApiUrl(state.serverConfig.url);
  const generation = state.sessionGeneration;
  const headers = new Headers(init.headers);
  headers.set("Accept", "application/json");
  if (init.body !== undefined) headers.set("Content-Type", "application/json");
  const response = await fetch(`${base}${path}`, {
    ...init,
    headers,
    credentials: "include",
    cache: "no-store",
  });
  const request = { response, generation, base };
  assertAuthRequestCurrent(request);
  if (!response.ok && response.status !== 202) {
    const message = await readErrorMessage(response);
    assertAuthRequestCurrent(request);
    throw new Error(message);
  }
  return request;
}

function assertAuthRequestCurrent(request: AuthFetchResult) {
  const state = useAuthStore.getState();
  if (
    state.sessionGeneration !== request.generation ||
    normalizeApiUrl(state.serverConfig.url) !== request.base
  ) {
    throw new SupersededAuthRequestError();
  }
}

function isSupersededAuthRequest(error: unknown) {
  return error instanceof SupersededAuthRequestError;
}

function clearLegacyCredentialStorage() {
  if (typeof window === "undefined") return;

  // The original console persisted HTTP Basic credentials and raw YAML (which
  // could contain passwords or tokens). Remove those legacy stores before the
  // new server-address-only state is hydrated.
  window.localStorage.removeItem("oxidns-auth");
  window.localStorage.removeItem("oxidns:upgrade-config");
  window.localStorage.removeItem("oxidns-next:upgrade-config");
  window.localStorage.removeItem("oxidns:pinned-plugins");
  window.localStorage.removeItem("oxidns-next:pinned-plugins");
  window.localStorage.removeItem("oxidns_topo_positions_v2");
  window.localStorage.removeItem("oxidns_qrf_positions");
  for (let index = window.localStorage.length - 1; index >= 0; index -= 1) {
    const key = window.localStorage.key(index);
    if (
      key?.startsWith("oxidns:config-history:") ||
      key?.startsWith("oxidns_seq_positions_")
    ) {
      window.localStorage.removeItem(key);
    }
  }
}

async function readAuthJson<T>(response: Response): Promise<T> {
  const text = await response.text();
  try {
    return JSON.parse(text) as T;
  } catch {
    throw new Error(
      tClient(WEBUI.storeErrors.invalidJsonResponse, {
        preview: text.slice(0, 160),
      }),
    );
  }
}

async function readErrorMessage(response: Response) {
  const text = await response.text();
  try {
    const body = JSON.parse(text) as { message?: string };
    if (body.message) return body.message;
  } catch {
    // Fall through to a compact HTTP error.
  }
  return `HTTP ${response.status}${response.statusText ? ` ${response.statusText}` : ""}`;
}

function authErrorMessage(error: unknown) {
  return error instanceof Error
    ? error.message
    : tClient(WEBUI.storeErrors.connectionFailed);
}

function assertAuthenticatedSession(
  session: AuthSessionResponse,
): asserts session is AuthSessionResponse & {
  authenticated: true;
  user: AuthUser;
  csrf_token: string;
} {
  if (
    session.authenticated !== true ||
    !session.user ||
    typeof session.user.id !== "string" ||
    typeof session.user.username !== "string" ||
    typeof session.csrf_token !== "string" ||
    !session.csrf_token
  ) {
    throw new Error(
      tClient(WEBUI.storeErrors.invalidJsonResponse, {
        preview: "missing authenticated session fields",
      }),
    );
  }
}
