"use client";

import { apiFetch, readApiResponseBody } from "@/lib/api-client";

export interface PasskeySummary {
  id: string;
  name: string;
  created_at_ms: number;
  last_used_at_ms?: number;
}

export interface OidcIdentitySummary {
  issuer: string;
  subject: string;
  display_name?: string;
}

export interface SecuritySummary {
  ok: boolean;
  totp_enabled: boolean;
  passkeys: PasskeySummary[];
  oidc_identities: OidcIdentitySummary[];
}

export interface TotpSetupResponse {
  ok: boolean;
  secret: string;
  otpauth_uri: string;
  expires_in: number;
}

export interface PasskeyFlowResponse {
  ok: boolean;
  flow_id: string;
  options: unknown;
}

export async function fetchSecuritySummary(): Promise<SecuritySummary> {
  return authJson<SecuritySummary>(await apiFetch("/auth/security"));
}

export async function beginTotpSetup(): Promise<TotpSetupResponse> {
  return authJson<TotpSetupResponse>(
    await apiFetch("/auth/totp/begin", { method: "POST" }),
  );
}

export async function confirmTotpSetup(code: string) {
  return authJson<{ ok: boolean; recovery_codes: string[] }>(
    await apiFetch("/auth/totp/confirm", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ code }),
    }),
  );
}

export async function disableTotp(password: string, code: string) {
  return authJson<{ ok: boolean }>(
    await apiFetch("/auth/totp", {
      method: "DELETE",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ password, code }),
    }),
  );
}

export async function changeLocalPassword(
  currentPassword: string,
  newPassword: string,
) {
  return authJson<{ ok: boolean }>(
    await apiFetch("/auth/password", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        current_password: currentPassword,
        new_password: newPassword,
      }),
    }),
  );
}

export async function beginPasskeyRegistration(): Promise<PasskeyFlowResponse> {
  return authJson<PasskeyFlowResponse>(
    await apiFetch("/auth/passkeys/register/begin", { method: "POST" }),
  );
}

export async function finishPasskeyRegistration(
  flowId: string,
  credential: unknown,
  name?: string,
) {
  return authJson<{ ok: boolean; passkey: PasskeySummary }>(
    await apiFetch("/auth/passkeys/register/finish", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        flow_id: flowId,
        credential,
        ...(name?.trim() ? { name: name.trim() } : {}),
      }),
    }),
  );
}

export async function renamePasskey(id: string, name: string) {
  return authJson<{ ok: boolean; passkey: PasskeySummary }>(
    await apiFetch(`/auth/passkeys/${encodeURIComponent(id)}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name: name.trim() }),
    }),
  );
}

export async function deletePasskey(id: string) {
  return authJson<{ ok: boolean }>(
    await apiFetch(`/auth/passkeys/${encodeURIComponent(id)}`, {
      method: "DELETE",
    }),
  );
}

export async function beginPasskeyLogin(
  username: string,
): Promise<PasskeyFlowResponse> {
  return authJson<PasskeyFlowResponse>(
    await apiFetch("/auth/passkeys/login/begin", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ username }),
    }),
  );
}

export async function fetchOidcStart(returnTo = "/") {
  return authJson<{ ok: boolean; url: string }>(
    await apiFetch("/auth/oidc/start", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ return_to: returnTo }),
    }),
  );
}

async function authJson<T>(response: Response): Promise<T> {
  const text = await readApiResponseBody(response, (current) => current.text());
  let body: unknown;
  try {
    body = text ? (JSON.parse(text) as unknown) : {};
  } catch {
    throw new Error(`HTTP ${response.status}: invalid JSON response`);
  }
  if (!response.ok) {
    const message =
      body &&
      typeof body === "object" &&
      "message" in body &&
      typeof body.message === "string"
        ? body.message
        : `HTTP ${response.status}`;
    throw new Error(message);
  }
  return body as T;
}
