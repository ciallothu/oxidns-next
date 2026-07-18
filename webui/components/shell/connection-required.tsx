"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import {
  CircleAlert,
  FileCode2,
  Fingerprint,
  KeyRound,
  Loader2,
  LogIn,
  PlugZap,
  ShieldCheck,
  UserPlus,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Field, FieldLabel } from "@/components/ui/field";
import { useAppStore } from "@/lib/store";
import { useAuthStore } from "@/lib/auth-store";
import { beginPasskeyLogin, fetchOidcStart } from "@/lib/auth-api";
import {
  assertApiSessionCurrent,
  captureApiSession,
} from "@/lib/api-client";
import { getPasskeyCredential } from "@/lib/webauthn";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { WEBUI } from "@/lib/i18n";
import { useI18n } from "@/lib/i18n/provider";

export function LoginRequired() {
  const { t } = useI18n();
  const serverConfig = useAuthStore((state) => state.serverConfig);
  const methods = useAuthStore((state) => state.methods);
  const setupRequired = useAuthStore((state) => state.setupRequired);
  const totpChallenge = useAuthStore((state) => state.totpChallenge);
  const bootstrap = useAuthStore((state) => state.bootstrap);
  const login = useAuthStore((state) => state.login);
  const verifyTotp = useAuthStore((state) => state.verifyTotp);
  const finishPasskeyLogin = useAuthStore(
    (state) => state.finishPasskeyLogin,
  );
  const cancelTotpChallenge = useAuthStore(
    (state) => state.cancelTotpChallenge,
  );
  const isConnecting = useAuthStore((state) => state.isConnecting);
  const connectionError = useAuthStore((state) => state.connectionError);
  const setEditorMode = useAppStore((state) => state.setEditorMode);

  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [bootstrapToken, setBootstrapToken] = useState("");
  const [totpCode, setTotpCode] = useState("");
  const [localError, setLocalError] = useState<string | null>(null);
  const [flowBusy, setFlowBusy] = useState(false);

  useEffect(() => {
    if (!totpChallenge) return;
    const delay = Math.max(0, totpChallenge.expiresAt - Date.now());
    const timeout = window.setTimeout(cancelTotpChallenge, delay);
    return () => window.clearTimeout(timeout);
  }, [totpChallenge, cancelTotpChallenge]);

  const handlePasswordSubmit = async (event: React.FormEvent) => {
    event.preventDefault();
    setLocalError(null);
    if (setupRequired) {
      if (password !== confirmPassword) {
        setLocalError(t(WEBUI.connection.passwordMismatch));
        return;
      }
      await bootstrap(username, password, bootstrapToken || undefined);
      return;
    }
    await login(username, password);
  };

  const handleTotpSubmit = async (event: React.FormEvent) => {
    event.preventDefault();
    setLocalError(null);
    await verifyTotp(totpCode);
  };

  const handlePasskey = async () => {
    const session = captureApiSession();
    setLocalError(null);
    setFlowBusy(true);
    try {
      const flow = await beginPasskeyLogin(username);
      assertApiSessionCurrent(session);
      const credential = await getPasskeyCredential(flow.options);
      assertApiSessionCurrent(session);
      await finishPasskeyLogin({
        flow_id: flow.flow_id,
        credential,
      });
    } catch (error) {
      setLocalError(
        error instanceof Error
          ? error.message
          : t(WEBUI.connection.passkeyFailed),
      );
    } finally {
      setFlowBusy(false);
    }
  };

  const handleOidc = async () => {
    setLocalError(null);
    setFlowBusy(true);
    try {
      // The API may live on a different explicitly allowed origin. Preserve
      // the complete console URL so the callback returns to the WebUI rather
      // than attempting to render a page on the API origin.
      const response = await fetchOidcStart(window.location.href);
      window.location.assign(response.url);
    } catch (error) {
      setLocalError(
        error instanceof Error
          ? error.message
          : t(WEBUI.connection.oidcFailed),
      );
      setFlowBusy(false);
    }
  };

  const error = localError ?? connectionError;

  return (
    <main className="oxidns-next-dialog-scrollbar min-h-0 flex-1 overflow-auto p-6">
      <Card className="mx-auto max-w-md">
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            {setupRequired ? (
              <UserPlus className="h-5 w-5" />
            ) : (
              <KeyRound className="h-5 w-5" />
            )}
            {setupRequired
              ? t(WEBUI.connection.setupTitle)
              : t(WEBUI.connection.loginTitle)}
          </CardTitle>
          <CardDescription>
            {setupRequired
              ? t(WEBUI.connection.setupDescription)
              : t(WEBUI.connection.authRequired)}
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="truncate rounded-md border bg-muted/40 px-3 py-2 font-mono text-xs text-muted-foreground">
            {serverConfig.url || "/api"}
          </div>

          {totpChallenge ? (
            <form onSubmit={handleTotpSubmit} className="space-y-4">
              <div className="rounded-lg border bg-muted/30 p-3 text-sm text-muted-foreground">
                <div className="mb-1 flex items-center gap-2 font-medium text-foreground">
                  <ShieldCheck className="h-4 w-4" />
                  {t(WEBUI.connection.totpTitle)}
                </div>
                {t(WEBUI.connection.totpDescription)}
              </div>
              <Field>
                <FieldLabel>{t(WEBUI.connection.totpCode)}</FieldLabel>
                <Input
                  value={totpCode}
                  onChange={(event) => setTotpCode(event.target.value)}
                  inputMode="numeric"
                  autoComplete="one-time-code"
                  autoFocus
                />
              </Field>
              <Button
                type="submit"
                className="w-full"
                disabled={isConnecting || !totpCode.trim()}
              >
                {isConnecting
                  ? t(WEBUI.connection.verifying)
                  : t(WEBUI.connection.verifyAndLogin)}
              </Button>
              <Button
                type="button"
                variant="ghost"
                className="w-full"
                onClick={() => {
                  setTotpCode("");
                  setLocalError(null);
                  cancelTotpChallenge();
                }}
                disabled={isConnecting}
              >
                {t(WEBUI.connection.backToPassword)}
              </Button>
            </form>
          ) : (
            <form onSubmit={handlePasswordSubmit} className="space-y-4">
              {(setupRequired || methods.password || methods.passkey) && (
                <Field>
                  <FieldLabel>{t(WEBUI.connection.username)}</FieldLabel>
                  <Input
                    value={username}
                    onChange={(event) => setUsername(event.target.value)}
                    autoComplete="username"
                    autoFocus
                  />
                </Field>
              )}
              {(setupRequired || methods.password) && (
                <Field>
                  <FieldLabel>{t(WEBUI.connection.password)}</FieldLabel>
                  <Input
                    type="password"
                    value={password}
                    onChange={(event) => setPassword(event.target.value)}
                    autoComplete={setupRequired ? "new-password" : "current-password"}
                  />
                </Field>
              )}
              {setupRequired && (
                <>
                  <Field>
                    <FieldLabel>
                      {t(WEBUI.connection.confirmPassword)}
                    </FieldLabel>
                    <Input
                      type="password"
                      value={confirmPassword}
                      onChange={(event) =>
                        setConfirmPassword(event.target.value)
                      }
                      autoComplete="new-password"
                    />
                  </Field>
                  <Field>
                    <FieldLabel>{t(WEBUI.connection.bootstrapToken)}</FieldLabel>
                    <Input
                      type="password"
                      value={bootstrapToken}
                      onChange={(event) => setBootstrapToken(event.target.value)}
                      autoComplete="off"
                      placeholder={t(WEBUI.connection.bootstrapTokenPlaceholder)}
                    />
                  </Field>
                </>
              )}

              {(setupRequired || methods.password) && (
                <Button
                  type="submit"
                  className="w-full"
                  disabled={
                    isConnecting ||
                    flowBusy ||
                    !username.trim() ||
                    !password ||
                    (setupRequired && !confirmPassword)
                  }
                >
                  {isConnecting ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : setupRequired ? (
                    <UserPlus className="h-4 w-4" />
                  ) : (
                    <LogIn className="h-4 w-4" />
                  )}
                  {isConnecting
                    ? t(WEBUI.connection.loggingIn)
                    : setupRequired
                      ? t(WEBUI.connection.createAdmin)
                      : t(WEBUI.connection.loginTitle)}
                </Button>
              )}
            </form>
          )}

          {!setupRequired && !totpChallenge &&
            (methods.passkey || methods.oidc) && (
              <div className="space-y-2 border-t pt-4">
                <p className="text-center text-xs text-muted-foreground">
                  {t(WEBUI.connection.otherMethods)}
                </p>
                <div className="grid gap-2 sm:grid-cols-2">
                  {methods.passkey && (
                    <Button
                      type="button"
                      variant="outline"
                      onClick={() => void handlePasskey()}
                      disabled={isConnecting || flowBusy || !username.trim()}
                    >
                      <Fingerprint className="h-4 w-4" />
                      {t(WEBUI.connection.usePasskey)}
                    </Button>
                  )}
                  {methods.oidc && (
                    <Button
                      type="button"
                      variant="outline"
                      onClick={() => void handleOidc()}
                      disabled={isConnecting || flowBusy}
                    >
                      <ShieldCheck className="h-4 w-4" />
                      {t(WEBUI.connection.useOidc)}
                    </Button>
                  )}
                </div>
              </div>
            )}

          {error && (
            <div className="flex items-center gap-2 rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              <CircleAlert className="h-4 w-4 shrink-0" />
              {error}
            </div>
          )}

          <div className="flex flex-wrap items-center gap-2 border-t pt-4">
            <Button variant="outline" size="sm" asChild>
              <Link href="/settings">
                {t(WEBUI.connection.editConnection)}
              </Link>
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setEditorMode(true)}
            >
              <FileCode2 className="mr-1.5 h-3.5 w-3.5" />
              {t(WEBUI.connection.offlineEditConfig)}
            </Button>
          </div>
        </CardContent>
      </Card>
    </main>
  );
}

export function ConnectionPending() {
  const { t } = useI18n();
  return (
    <main className="oxidns-next-dialog-scrollbar min-h-0 flex-1 overflow-auto p-6">
      <Card className="max-w-xl">
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Loader2 className="h-5 w-5 animate-spin" />
            {t(WEBUI.connection.pendingTitle)}
          </CardTitle>
          <CardDescription>{t(WEBUI.connection.pendingDesc)}</CardDescription>
        </CardHeader>
      </Card>
    </main>
  );
}

export function ConnectionRequired() {
  const { t } = useI18n();
  const setEditorMode = useAppStore((state) => state.setEditorMode);
  return (
    <main className="oxidns-next-dialog-scrollbar min-h-0 flex-1 overflow-auto p-6">
      <Card className="max-w-xl">
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <PlugZap className="h-5 w-5" />
            {t(WEBUI.connection.requiredTitle)}
          </CardTitle>
          <CardDescription>{t(WEBUI.connection.requiredDesc)}</CardDescription>
        </CardHeader>
        <CardContent className="flex flex-wrap gap-2">
          <Button asChild>
            <Link href="/settings">{t(WEBUI.connection.goSettings)}</Link>
          </Button>
          <Button variant="outline" onClick={() => setEditorMode(true)}>
            <FileCode2 className="mr-1.5 h-4 w-4" />
            {t(WEBUI.connection.offlineEditConfigFile)}
          </Button>
        </CardContent>
      </Card>
    </main>
  );
}
