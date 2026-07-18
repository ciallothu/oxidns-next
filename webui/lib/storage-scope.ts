"use client";

import { useAuthStore } from "./auth-store";

/**
 * Keep browser-only presentation state isolated per API instance and account.
 * The encoded value is safe to append to a localStorage key and contains no
 * credential or session material.
 */
export function useAuthenticatedStorageScope(): string {
  return useAuthStore((state) =>
    encodeURIComponent(
      `${state.serverConfig.url.trim()}|${state.user?.id ?? "anonymous"}`,
    ),
  );
}

export function removeLegacyStorageKey(key: string): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.removeItem(key);
  } catch {
    // Storage may be disabled. Presentation state can safely remain in memory.
  }
}
