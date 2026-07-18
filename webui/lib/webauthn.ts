/**
 * Browser WebAuthn adapters.
 *
 * webauthn-rs serializes binary fields as base64url strings while the browser
 * API expects ArrayBuffer values. Keep the conversion at this boundary so
 * credentials never pass through application storage.
 */

import { WEBUI, tClient } from "@/lib/i18n";

type JsonRecord = Record<string, unknown>;

export async function createPasskeyCredential(
  serializedOptions: unknown,
): Promise<JsonRecord> {
  ensureWebAuthnAvailable();
  const publicKey = unwrapPublicKey(serializedOptions);
  const options: PublicKeyCredentialCreationOptions = {
    ...(publicKey as unknown as PublicKeyCredentialCreationOptions),
    challenge: decodeBase64Url(requiredString(publicKey.challenge, "challenge")),
    user: {
      ...(asRecord(publicKey.user) as unknown as PublicKeyCredentialUserEntity),
      id: decodeBase64Url(
        requiredString(asRecord(publicKey.user).id, "user.id"),
      ),
    },
    excludeCredentials: decodeCredentialDescriptors(
      publicKey.excludeCredentials,
    ),
  };

  const credential = await navigator.credentials.create({ publicKey: options });
  if (!(credential instanceof PublicKeyCredential)) {
    throw new Error(tClient(WEBUI.storeErrors.passkeyUnexpectedRegistration));
  }
  const response = credential.response;
  if (!(response instanceof AuthenticatorAttestationResponse)) {
    throw new Error(tClient(WEBUI.storeErrors.passkeyUnexpectedRegistration));
  }

  return {
    id: credential.id,
    rawId: encodeBase64Url(credential.rawId),
    type: credential.type,
    authenticatorAttachment: credential.authenticatorAttachment,
    clientExtensionResults: credential.getClientExtensionResults(),
    response: {
      attestationObject: encodeBase64Url(response.attestationObject),
      clientDataJSON: encodeBase64Url(response.clientDataJSON),
      transports: response.getTransports?.() ?? [],
    },
  };
}

export async function getPasskeyCredential(
  serializedOptions: unknown,
): Promise<JsonRecord> {
  ensureWebAuthnAvailable();
  const publicKey = unwrapPublicKey(serializedOptions);
  const options: PublicKeyCredentialRequestOptions = {
    ...(publicKey as unknown as PublicKeyCredentialRequestOptions),
    challenge: decodeBase64Url(requiredString(publicKey.challenge, "challenge")),
    allowCredentials: decodeCredentialDescriptors(publicKey.allowCredentials),
  };

  const credential = await navigator.credentials.get({ publicKey: options });
  if (!(credential instanceof PublicKeyCredential)) {
    throw new Error(
      tClient(WEBUI.storeErrors.passkeyUnexpectedAuthentication),
    );
  }
  const response = credential.response;
  if (!(response instanceof AuthenticatorAssertionResponse)) {
    throw new Error(
      tClient(WEBUI.storeErrors.passkeyUnexpectedAuthentication),
    );
  }

  return {
    id: credential.id,
    rawId: encodeBase64Url(credential.rawId),
    type: credential.type,
    authenticatorAttachment: credential.authenticatorAttachment,
    clientExtensionResults: credential.getClientExtensionResults(),
    response: {
      authenticatorData: encodeBase64Url(response.authenticatorData),
      clientDataJSON: encodeBase64Url(response.clientDataJSON),
      signature: encodeBase64Url(response.signature),
      userHandle: response.userHandle
        ? encodeBase64Url(response.userHandle)
        : null,
    },
  };
}

function decodeCredentialDescriptors(
  value: unknown,
): PublicKeyCredentialDescriptor[] | undefined {
  if (!Array.isArray(value)) return undefined;
  return value.map((item) => {
    const descriptor = asRecord(item);
    return {
      ...(descriptor as unknown as PublicKeyCredentialDescriptor),
      id: decodeBase64Url(requiredString(descriptor.id, "credential.id")),
      type: "public-key",
    };
  });
}

function unwrapPublicKey(value: unknown): JsonRecord {
  const wrapper = asRecord(value);
  const publicKey = asRecord(wrapper.publicKey);
  return Object.keys(publicKey).length > 0 ? publicKey : wrapper;
}

function decodeBase64Url(value: string): Uint8Array<ArrayBuffer> {
  const normalized = value.replace(/-/g, "+").replace(/_/g, "/");
  const padded = normalized.padEnd(Math.ceil(normalized.length / 4) * 4, "=");
  const bytes = Uint8Array.from(atob(padded), (character) =>
    character.charCodeAt(0),
  );
  return new Uint8Array(bytes);
}

function encodeBase64Url(value: ArrayBuffer): string {
  const bytes = new Uint8Array(value);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary)
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/g, "");
}

function requiredString(value: unknown, field: string): string {
  if (typeof value !== "string" || !value) {
    throw new Error(
      tClient(WEBUI.storeErrors.passkeyInvalidOptions, { field }),
    );
  }
  return value;
}

function asRecord(value: unknown): JsonRecord {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as JsonRecord)
    : {};
}

function ensureWebAuthnAvailable() {
  if (
    typeof window === "undefined" ||
    !window.isSecureContext ||
    !("PublicKeyCredential" in window)
  ) {
    throw new Error(tClient(WEBUI.storeErrors.passkeyUnsupported));
  }
}
