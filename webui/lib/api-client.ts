"use client";

import { useAuthStore } from "@/lib/auth-store";

export interface ApiSessionSnapshot {
  generation: number;
  serverUrl: string;
}

const responseSessions = new WeakMap<Response, ApiSessionSnapshot>();

class SupersededApiRequestError extends Error {
  constructor() {
    super("API request was superseded");
    this.name = "AbortError";
  }
}

/**
 * Resolve a management API path against the user-selected backend.
 *
 * Authentication is intentionally absent from this function: the opaque
 * session token is an HttpOnly cookie and is attached by the browser.
 */
export function apiUrl(path: string) {
  const baseUrl = useAuthStore.getState().serverConfig.url.trim();
  return `${baseUrl.replace(/\/$/, "")}${path}`;
}

export function apiHeaders(initial?: HeadersInit) {
  const headers = new Headers(initial);
  if (!headers.has("Accept")) headers.set("Accept", "application/json");
  return Object.fromEntries(headers.entries());
}

/**
 * Shared authenticated fetch wrapper.
 *
 * Cross-origin console deployments need `credentials: include`; mutating
 * requests also carry the per-session CSRF token returned by the backend.
 * No reusable credential is exposed to JavaScript or persisted in storage.
 */
export async function apiFetch(path: string, init: RequestInit = {}) {
  return apiRequest(apiUrl(path), init);
}

/** Attach session and CSRF semantics to an already-resolved API URL. */
export async function apiRequest(
  input: RequestInfo | URL,
  init: RequestInit = {},
) {
  const session = captureApiSession();
  const method = (init.method ?? "GET").toUpperCase();
  const headers = new Headers(init.headers);
  if (!headers.has("Accept")) headers.set("Accept", "application/json");

  if (!isReadOnlyMethod(method)) {
    const csrfToken = useAuthStore.getState().csrfToken;
    if (csrfToken && !headers.has("X-CSRF-Token")) {
      headers.set("X-CSRF-Token", csrfToken);
    }
  }

  const response = await fetch(input, {
    ...init,
    method,
    headers,
    credentials: "include",
    cache: init.cache ?? "no-store",
  });
  responseSessions.set(response, session);
  assertApiSessionCurrent(session);
  if (response.status === 401) {
    const sessionFailure = await isSessionFailure(response);
    // Parsing the response body is asynchronous. Re-check after it completes
    // so an old 401 cannot expire a session established in the meantime.
    assertApiSessionCurrent(session);
    if (sessionFailure) {
      useAuthStore.getState().markSessionExpired();
    }
  }
  return response;
}

export function captureApiSession(): ApiSessionSnapshot {
  const state = useAuthStore.getState();
  return {
    generation: state.sessionGeneration,
    serverUrl: state.serverConfig.url,
  };
}

export function isApiSessionCurrent(session: ApiSessionSnapshot) {
  const current = useAuthStore.getState();
  return (
    current.sessionGeneration === session.generation &&
    current.serverConfig.url === session.serverUrl
  );
}

export function assertApiSessionCurrent(session: ApiSessionSnapshot) {
  if (!isApiSessionCurrent(session)) throw new SupersededApiRequestError();
}

export function assertApiResponseCurrent(response: Response) {
  const session = responseSessions.get(response);
  if (session) assertApiSessionCurrent(session);
}

/**
 * Read a response body and verify that it still belongs to the active session.
 *
 * Fetch completion is not enough: parsing text, JSON, blobs, or stream chunks
 * yields back to the event loop, where a logout or backend switch may occur.
 */
export async function readApiResponseBody<T>(
  response: Response,
  reader: (response: Response) => Promise<T>,
): Promise<T> {
  try {
    const value = await reader(response);
    assertApiResponseCurrent(response);
    return value;
  } catch (error) {
    // Prefer the superseded marker over a parse/read error from an obsolete
    // response so callers never surface it in the next session.
    assertApiResponseCurrent(response);
    throw error;
  }
}

export function isSupersededApiRequest(error: unknown) {
  return error instanceof SupersededApiRequestError;
}

function isReadOnlyMethod(method: string) {
  return method === "GET" || method === "HEAD" || method === "OPTIONS";
}

async function isSessionFailure(response: Response) {
  try {
    const body = (await response.clone().json()) as { code?: unknown };
    return body.code === "unauthorized" || body.code === "session_expired";
  } catch {
    // Preserve compatibility with older management endpoints that returned a
    // plain 401 body for an invalid session.
    return true;
  }
}
