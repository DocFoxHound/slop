// Passkeys in the browser (crates/slop-api/src/passkeys.rs). The server sends WebAuthn options as
// JSON with base64url binary fields; the browser API wants ArrayBuffers, and its answer goes back
// as JSON the same way.

import { isDesktop } from "./platform";

const b64urlToBuf = (s: string): ArrayBuffer => {
  const b64 = s.replace(/-/g, "+").replace(/_/g, "/").padEnd(Math.ceil(s.length / 4) * 4, "=");
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out.buffer;
};

const bufToB64url = (b: ArrayBuffer | null | undefined): string | undefined => {
  if (!b) return undefined;
  let bin = "";
  for (const byte of new Uint8Array(b)) bin += String.fromCharCode(byte);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
};

type Json = Record<string, unknown>;

/** Passkeys belong to the website's address, so the desktop app (another origin) can't use them. */
export const passkeysSupported = (): boolean =>
  !isDesktop && typeof window !== "undefined" && "PublicKeyCredential" in window;

/** Makes a passkey from the server's `CreationChallengeResponse`; returns what to send back. */
export async function createPasskey(options: { publicKey: Json }): Promise<Json> {
  const pk = options.publicKey as Json & { user: Json; excludeCredentials?: Json[] };
  const publicKey = {
    ...pk,
    challenge: b64urlToBuf(pk.challenge as string),
    user: { ...pk.user, id: b64urlToBuf(pk.user.id as string) },
    excludeCredentials: (pk.excludeCredentials ?? []).map((c) => ({ ...c, id: b64urlToBuf(c.id as string) })),
  } as unknown as PublicKeyCredentialCreationOptions;
  const cred = (await navigator.credentials.create({ publicKey })) as PublicKeyCredential | null;
  if (!cred) throw new Error("No passkey was made.");
  const r = cred.response as AuthenticatorAttestationResponse;
  return {
    id: cred.id,
    rawId: bufToB64url(cred.rawId),
    type: cred.type,
    response: {
      attestationObject: bufToB64url(r.attestationObject),
      clientDataJSON: bufToB64url(r.clientDataJSON),
      transports: typeof r.getTransports === "function" ? r.getTransports() : undefined,
    },
    extensions: cred.getClientExtensionResults(),
  };
}

/** Signs the server's `RequestChallengeResponse` with a passkey; returns what to send back. */
export async function signWithPasskey(options: { publicKey: Json }): Promise<Json> {
  const pk = options.publicKey as Json & { allowCredentials?: Json[] };
  const publicKey = {
    ...pk,
    challenge: b64urlToBuf(pk.challenge as string),
    allowCredentials: (pk.allowCredentials ?? []).map((c) => ({ ...c, id: b64urlToBuf(c.id as string) })),
  } as unknown as PublicKeyCredentialRequestOptions;
  // A button press, not autofill: the browser's normal passkey prompt.
  const cred = (await navigator.credentials.get({ publicKey })) as PublicKeyCredential | null;
  if (!cred) throw new Error("No passkey was used.");
  const r = cred.response as AuthenticatorAssertionResponse;
  return {
    id: cred.id,
    rawId: bufToB64url(cred.rawId),
    type: cred.type,
    response: {
      authenticatorData: bufToB64url(r.authenticatorData),
      clientDataJSON: bufToB64url(r.clientDataJSON),
      signature: bufToB64url(r.signature),
      userHandle: bufToB64url(r.userHandle),
    },
    extensions: cred.getClientExtensionResults(),
  };
}
