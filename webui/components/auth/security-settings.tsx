"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import {
  Check,
  Copy,
  Fingerprint,
  KeyRound,
  Loader2,
  Plus,
  RefreshCw,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import {
  beginPasskeyRegistration,
  beginTotpSetup,
  changeLocalPassword,
  confirmTotpSetup,
  deletePasskey,
  disableTotp,
  fetchSecuritySummary,
  finishPasskeyRegistration,
  renamePasskey,
  type SecuritySummary,
  type TotpSetupResponse,
} from "@/lib/auth-api";
import { createPasskeyCredential } from "@/lib/webauthn";
import {
  assertApiSessionCurrent,
  captureApiSession,
} from "@/lib/api-client";
import { useAuthStore } from "@/lib/auth-store";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Field, FieldLabel } from "@/components/ui/field";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { WEBUI } from "@/lib/i18n";
import { useI18n } from "@/lib/i18n/provider";

export function SecuritySettings() {
  const { t, formatDateTime } = useI18n();
  const user = useAuthStore((state) => state.user);
  const methods = useAuthStore((state) => state.methods);
  const sessionGeneration = useAuthStore(
    (state) => state.sessionGeneration,
  );
  const [summary, setSummary] = useState<SecuritySummary | null>(null);
  const [totpSetup, setTotpSetup] = useState<TotpSetupResponse | null>(null);
  const [totpSetupExpiresAt, setTotpSetupExpiresAt] = useState<number | null>(
    null,
  );
  const [totpCode, setTotpCode] = useState("");
  const [recoveryCodes, setRecoveryCodes] = useState<string[] | null>(null);
  const [disablePassword, setDisablePassword] = useState("");
  const [disableCode, setDisableCode] = useState("");
  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [passkeyName, setPasskeyName] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [isLoading, setIsLoading] = useState(true);
  const refreshSerial = useRef(0);
  const scopeSerial = useRef(0);

  const refresh = useCallback(async () => {
    const serial = ++refreshSerial.current;
    setIsLoading(true);
    setError(null);
    try {
      const nextSummary = await fetchSecuritySummary();
      if (refreshSerial.current === serial) setSummary(nextSummary);
    } catch (reason) {
      if (refreshSerial.current === serial) setError(errorMessage(reason));
    } finally {
      if (refreshSerial.current === serial) setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    // This component remains mounted while the settings route is visible.
    // Clear all account-derived and one-time secret state whenever the auth
    // session changes so a shared browser cannot briefly expose the previous
    // account's TOTP setup, recovery codes, passkeys, or OIDC identities.
    scopeSerial.current += 1;
    refreshSerial.current += 1;
    setSummary(null);
    setTotpSetup(null);
    setTotpSetupExpiresAt(null);
    setTotpCode("");
    setRecoveryCodes(null);
    setDisablePassword("");
    setDisableCode("");
    setCurrentPassword("");
    setNewPassword("");
    setConfirmPassword("");
    setPasskeyName("");
    setBusy(null);
    setError(null);
    setCopied(false);
    setIsLoading(Boolean(user));
    if (user) void refresh();
  }, [sessionGeneration, user, refresh]);

  useEffect(() => {
    if (!totpSetupExpiresAt) return;
    const timeout = window.setTimeout(() => {
      setTotpSetup(null);
      setTotpSetupExpiresAt(null);
      setTotpCode("");
    }, Math.max(0, totpSetupExpiresAt - Date.now()));
    return () => window.clearTimeout(timeout);
  }, [totpSetupExpiresAt]);

  const run = async (name: string, task: () => Promise<void>) => {
    const scope = scopeSerial.current;
    setBusy(name);
    setError(null);
    try {
      await task();
    } catch (reason) {
      if (scopeSerial.current === scope) setError(errorMessage(reason));
    } finally {
      if (scopeSerial.current === scope) setBusy(null);
    }
  };

  if (!user) return null;

  return (
    <Card>
      <CardHeader>
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <CardTitle className="flex items-center gap-2">
              <ShieldCheck className="h-5 w-5" />
              {t(WEBUI.accountSecurity.title)}
            </CardTitle>
            <CardDescription>
              {t(WEBUI.accountSecurity.description, {
                username: user.username,
              })}
            </CardDescription>
          </div>
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => void refresh()}
            disabled={busy !== null || isLoading}
          >
            <RefreshCw className={isLoading ? "animate-spin" : ""} />
            <span className="sr-only">{t(WEBUI.common.refresh)}</span>
          </Button>
        </div>
      </CardHeader>
      <CardContent className="space-y-6">
        {error && (
          <div className="rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
            {error}
          </div>
        )}
        {isLoading && !summary && (
          <p className="text-sm text-muted-foreground">
            {t(WEBUI.accountSecurity.loading)}
          </p>
        )}

        {methods.password && (
          <section className="space-y-3">
            <div>
              <h3 className="flex items-center gap-2 text-sm font-semibold">
                <KeyRound className="h-4 w-4" />
                {t(WEBUI.accountSecurity.passwordTitle)}
              </h3>
              <p className="text-xs text-muted-foreground">
                {t(WEBUI.accountSecurity.passwordDescription)}
              </p>
            </div>
            <div className="grid gap-2 sm:grid-cols-3">
              <Field>
                <FieldLabel>
                  {t(WEBUI.accountSecurity.currentPassword)}
                </FieldLabel>
                <Input
                  type="password"
                  value={currentPassword}
                  onChange={(event) => setCurrentPassword(event.target.value)}
                  autoComplete="current-password"
                />
              </Field>
              <Field>
                <FieldLabel>{t(WEBUI.accountSecurity.newPassword)}</FieldLabel>
                <Input
                  type="password"
                  value={newPassword}
                  onChange={(event) => setNewPassword(event.target.value)}
                  autoComplete="new-password"
                />
              </Field>
              <Field>
                <FieldLabel>
                  {t(WEBUI.accountSecurity.confirmPassword)}
                </FieldLabel>
                <Input
                  type="password"
                  value={confirmPassword}
                  onChange={(event) => setConfirmPassword(event.target.value)}
                  autoComplete="new-password"
                />
              </Field>
            </div>
            <Button
              variant="outline"
              onClick={() =>
                void run("password-change", async () => {
                  if (newPassword !== confirmPassword) {
                    throw new Error(
                      t(WEBUI.accountSecurity.passwordMismatch),
                    );
                  }
                  await changeLocalPassword(currentPassword, newPassword);
                  setCurrentPassword("");
                  setNewPassword("");
                  setConfirmPassword("");
                })
              }
              disabled={
                busy !== null ||
                isLoading ||
                !summary ||
                !currentPassword ||
                !newPassword ||
                !confirmPassword
              }
            >
              {busy === "password-change" && (
                <Loader2 className="animate-spin" />
              )}
              {t(WEBUI.accountSecurity.changePassword)}
            </Button>
          </section>
        )}

        <section className="space-y-3 border-t pt-5">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div>
              <h3 className="flex items-center gap-2 text-sm font-semibold">
                <KeyRound className="h-4 w-4" />
                {t(WEBUI.accountSecurity.totpTitle)}
              </h3>
              <p className="text-xs text-muted-foreground">
                {t(WEBUI.accountSecurity.totpDescription)}
              </p>
            </div>
            <Badge variant={summary?.totp_enabled ? "default" : "secondary"}>
              {!summary
                ? t(WEBUI.accountSecurity.loading)
                : summary.totp_enabled
                  ? t(WEBUI.accountSecurity.enabled)
                  : t(WEBUI.accountSecurity.disabled)}
            </Badge>
          </div>

          {summary && !summary.totp_enabled && !totpSetup && (
            <Button
              variant="outline"
              onClick={() =>
                void run("totp-begin", async () => {
                  const setup = await beginTotpSetup();
                  setTotpSetup(setup);
                  setTotpSetupExpiresAt(
                    Date.now() + setup.expires_in * 1_000,
                  );
                  setRecoveryCodes(null);
                })
              }
              disabled={busy !== null}
            >
              {busy === "totp-begin" ? (
                <Loader2 className="animate-spin" />
              ) : (
                <Plus />
              )}
              {t(WEBUI.accountSecurity.enableTotp)}
            </Button>
          )}

          {totpSetup && !recoveryCodes && (
            <div className="space-y-3 rounded-lg border bg-muted/20 p-3">
              <p className="text-sm">
                {t(WEBUI.accountSecurity.totpSetupInstructions)}
              </p>
              <div className="rounded-md border bg-background p-2 font-mono text-sm break-all">
                {totpSetup.secret}
              </div>
              <a
                href={totpSetup.otpauth_uri}
                className="block truncate text-xs text-primary underline underline-offset-2"
              >
                {t(WEBUI.accountSecurity.openAuthenticator)}
              </a>
              <div className="flex gap-2">
                <Input
                  value={totpCode}
                  onChange={(event) => setTotpCode(event.target.value)}
                  inputMode="numeric"
                  autoComplete="one-time-code"
                  placeholder={t(WEBUI.accountSecurity.verificationCode)}
                />
                <Button
                  onClick={() =>
                    void run("totp-confirm", async () => {
                      const result = await confirmTotpSetup(totpCode);
                      setRecoveryCodes(result.recovery_codes);
                      setTotpSetup(null);
                      setTotpSetupExpiresAt(null);
                      setTotpCode("");
                      await refresh();
                    })
                  }
                  disabled={busy !== null || !totpCode.trim()}
                >
                  {busy === "totp-confirm" && (
                    <Loader2 className="animate-spin" />
                  )}
                  {t(WEBUI.accountSecurity.confirmTotp)}
                </Button>
              </div>
            </div>
          )}

          {recoveryCodes && (
            <div className="space-y-3 rounded-lg border border-amber-500/30 bg-amber-500/10 p-3">
              <div>
                <p className="text-sm font-semibold">
                  {t(WEBUI.accountSecurity.recoveryCodesTitle)}
                </p>
                <p className="text-xs text-muted-foreground">
                  {t(WEBUI.accountSecurity.recoveryCodesDescription)}
                </p>
              </div>
              <pre className="overflow-auto rounded-md border bg-background p-3 font-mono text-sm">
                {recoveryCodes.join("\n")}
              </pre>
              <Button
                variant="outline"
                onClick={() => {
                  const scope = scopeSerial.current;
                  void navigator.clipboard
                    .writeText(recoveryCodes.join("\n"))
                    .then(() => {
                      if (scopeSerial.current === scope) setCopied(true);
                    });
                }}
              >
                {copied ? <Check /> : <Copy />}
                {copied
                  ? t(WEBUI.accountSecurity.copied)
                  : t(WEBUI.accountSecurity.copyCodes)}
              </Button>
              <Button
                variant="ghost"
                onClick={() => {
                  setRecoveryCodes(null);
                  setCopied(false);
                }}
              >
                {t(WEBUI.accountSecurity.done)}
              </Button>
            </div>
          )}

          {summary?.totp_enabled && (
            <div className="grid gap-2 rounded-lg border p-3 sm:grid-cols-[1fr_1fr_auto]">
              <Input
                type="password"
                value={disablePassword}
                onChange={(event) => setDisablePassword(event.target.value)}
                autoComplete="current-password"
                placeholder={t(WEBUI.accountSecurity.currentPassword)}
              />
              <Input
                value={disableCode}
                onChange={(event) => setDisableCode(event.target.value)}
                inputMode="numeric"
                autoComplete="one-time-code"
                placeholder={t(WEBUI.accountSecurity.verificationCode)}
              />
              <Button
                variant="destructive"
                onClick={() =>
                  void run("totp-disable", async () => {
                    await disableTotp(disablePassword, disableCode);
                    setDisablePassword("");
                    setDisableCode("");
                    await refresh();
                  })
                }
                disabled={
                  busy !== null || !disablePassword || !disableCode.trim()
                }
              >
                {t(WEBUI.accountSecurity.disableTotp)}
              </Button>
            </div>
          )}
        </section>

        <section className="space-y-3 border-t pt-5">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div>
              <h3 className="flex items-center gap-2 text-sm font-semibold">
                <Fingerprint className="h-4 w-4" />
                {t(WEBUI.accountSecurity.passkeysTitle)}
              </h3>
              <p className="text-xs text-muted-foreground">
                {t(WEBUI.accountSecurity.passkeysDescription)}
              </p>
            </div>
            {!methods.passkey && (
              <Badge variant="secondary">
                {t(WEBUI.accountSecurity.notConfigured)}
              </Badge>
            )}
          </div>
          {methods.passkey && (
            <>
              <div className="flex gap-2">
                <Input
                  value={passkeyName}
                  onChange={(event) => setPasskeyName(event.target.value)}
                  placeholder={t(WEBUI.accountSecurity.passkeyNamePlaceholder)}
                />
                <Button
                  onClick={() =>
                    void run("passkey-add", async () => {
                      const session = captureApiSession();
                      const flow = await beginPasskeyRegistration();
                      assertApiSessionCurrent(session);
                      const credential = await createPasskeyCredential(
                        flow.options,
                      );
                      assertApiSessionCurrent(session);
                      await finishPasskeyRegistration(
                        flow.flow_id,
                        credential,
                        passkeyName,
                      );
                      setPasskeyName("");
                      await refresh();
                    })
                  }
                  disabled={busy !== null || isLoading || !summary}
                >
                  {busy === "passkey-add" ? (
                    <Loader2 className="animate-spin" />
                  ) : (
                    <Plus />
                  )}
                  {t(WEBUI.accountSecurity.addPasskey)}
                </Button>
              </div>

              <div className="space-y-2">
                {summary?.passkeys.length ? (
              summary.passkeys.map((passkey) => (
                <div
                  key={passkey.id}
                  className="flex flex-wrap items-center gap-2 rounded-lg border p-3"
                >
                  <Input
                    className="min-w-44 flex-1"
                    defaultValue={passkey.name}
                    aria-label={t(WEBUI.accountSecurity.passkeyName)}
                    onBlur={(event) => {
                      const name = event.target.value.trim();
                      if (!name || name === passkey.name) return;
                      void run(`passkey-rename-${passkey.id}`, async () => {
                        await renamePasskey(passkey.id, name);
                        await refresh();
                      });
                    }}
                  />
                  <span className="text-xs text-muted-foreground">
                    {t(WEBUI.accountSecurity.addedAt, {
                      date: formatDateTime(passkey.created_at_ms),
                    })}
                  </span>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={() =>
                      window.confirm(
                        t(WEBUI.accountSecurity.deletePasskeyConfirm, {
                          name: passkey.name,
                        }),
                      ) &&
                      void run(`passkey-delete-${passkey.id}`, async () => {
                          await deletePasskey(passkey.id);
                          await refresh();
                        })
                    }
                    disabled={busy !== null}
                  >
                    <Trash2 />
                    <span className="sr-only">
                      {t(WEBUI.accountSecurity.deletePasskey)}
                    </span>
                  </Button>
                </div>
              ))
                ) : summary ? (
                  <p className="rounded-lg border border-dashed p-3 text-sm text-muted-foreground">
                    {t(WEBUI.accountSecurity.noPasskeys)}
                  </p>
                ) : null}
              </div>
            </>
          )}
        </section>

        <section className="space-y-3 border-t pt-5">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div>
              <h3 className="text-sm font-semibold">
                {t(WEBUI.accountSecurity.oidcTitle)}
              </h3>
              <p className="text-xs text-muted-foreground">
                {t(WEBUI.accountSecurity.oidcDescription)}
              </p>
            </div>
            <Badge variant={methods.oidc ? "default" : "secondary"}>
              {methods.oidc
                ? t(WEBUI.accountSecurity.available)
                : t(WEBUI.accountSecurity.notConfigured)}
            </Badge>
          </div>
          {summary?.oidc_identities.map((identity) => (
            <div
              key={`${identity.issuer}:${identity.subject}`}
              className="rounded-lg border p-3 text-sm"
            >
              <p className="font-medium">
                {identity.display_name || identity.subject}
              </p>
              <p className="truncate font-mono text-xs text-muted-foreground">
                {identity.issuer}
              </p>
            </div>
          ))}
        </section>
      </CardContent>
    </Card>
  );
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
