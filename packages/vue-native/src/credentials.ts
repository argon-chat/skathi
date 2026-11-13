// Passkeys: `navigator.credentials` for `publicKey` (WebAuthn Level 3) over the system authenticator.
// An app has no web origin, so the relying party ID must be named (`rp.id`, `rpId`) unless the page
// itself is served from it over https; the client data's origin is then `https://<rpId>`.

export interface WebAuthnHost {
  create(requestJson: string): string;
  get(requestJson: string): string;
  cancel(token: number): void;
  available(): boolean;
}

export interface CredentialsDeps {
  host: WebAuthnHost;
  /** The token-settled call: resolves the host's payload, rejects with `{ name, message }`. */
  call(json: string): Promise<Record<string, unknown>>;
  /** The page's own URL, or "" when there is none. */
  pageUrl(): string;
  DOMException: new (message?: string, name?: string) => Error;
}

type Bytes = Uint8Array;
type Json = Record<string, unknown>;

interface SignalLike {
  readonly aborted: boolean;
  readonly reason: unknown;
  addEventListener(type: "abort", fn: () => void, opts?: { once?: boolean }): void;
  removeEventListener(type: "abort", fn: () => void): void;
}

const B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

export function toBase64Url(b: Bytes): string {
  let out = "";
  for (let i = 0; i < b.length; i += 3) {
    const n = (b[i]! << 16) | ((b[i + 1] ?? 0) << 8) | (b[i + 2] ?? 0);
    out += B64[(n >> 18) & 63]! + B64[(n >> 12) & 63]!;
    if (i + 1 < b.length) out += B64[(n >> 6) & 63]!;
    if (i + 2 < b.length) out += B64[n & 63]!;
  }
  return out;
}

export function fromBase64Url(s: string): Bytes | null {
  const clean = s.replace(/=+$/, "").replace(/\+/g, "-").replace(/\//g, "_");
  if (/[^A-Za-z0-9_-]/.test(clean) || clean.length % 4 === 1) return null;
  const out = new Uint8Array(Math.floor((clean.length * 3) / 4));
  let bits = 0;
  let acc = 0;
  let j = 0;
  for (const ch of clean) {
    acc = (acc << 6) | B64.indexOf(ch);
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      out[j++] = (acc >> bits) & 0xff;
    }
  }
  return out;
}

const buffer = (b: Bytes): ArrayBuffer => b.slice().buffer as ArrayBuffer;

function utf8(s: string): Bytes {
  const out: number[] = [];
  for (const ch of s) {
    const c = ch.codePointAt(0)!;
    if (c < 0x80) out.push(c);
    else if (c < 0x800) out.push(0xc0 | (c >> 6), 0x80 | (c & 63));
    else if (c < 0x10000) out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
    else out.push(0xf0 | (c >> 18), 0x80 | ((c >> 12) & 63), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
  }
  return new Uint8Array(out);
}

function bytesOf(v: unknown, what: string): Bytes {
  if (v instanceof ArrayBuffer) return new Uint8Array(v.slice(0));
  if (ArrayBuffer.isView(v))
    return new Uint8Array(v.buffer.slice(v.byteOffset, v.byteOffset + v.byteLength) as ArrayBuffer);
  throw new TypeError(
    `Failed to read the '${what}' property: The provided value is not of type '(ArrayBuffer or ArrayBufferView)'.`,
  );
}

function decoded(v: unknown, what: string): ArrayBuffer {
  const b = typeof v === "string" ? fromBase64Url(v) : null;
  if (!b) throw new TypeError(`Failed to read the '${what}' property: it is not valid base64url.`);
  return buffer(b);
}

const DOMAIN = /^(?=.{1,253}$)([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?)(\.[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?)*$/;

export function installCredentials(nav: object, deps: CredentialsDeps): void {
  const { host, DOMException } = deps;
  const INTERNAL = Symbol("vn.credentials");

  const relyingParty = (requested: unknown): { rpId: string; origin: string } => {
    const page = /^https:\/\/([^/?#:@]+)(:\d+)?(?:[/?#]|$)/i.exec(deps.pageUrl());
    const pageHost = page ? page[1]!.toLowerCase() : "";
    const rpId = requested === undefined ? pageHost : String(requested).toLowerCase();
    if (!rpId) {
      throw new DOMException(
        "This app has no web origin: name the relying party with 'rp.id' (create) or 'rpId' (get).",
        "SecurityError",
      );
    }
    if (!DOMAIN.test(rpId) || /^\d+(\.\d+){3}$/.test(rpId)) {
      throw new DOMException(`'${rpId}' is an invalid domain.`, "SecurityError");
    }
    const onPage = pageHost !== "" && (pageHost === rpId || pageHost.endsWith(`.${rpId}`));
    return { rpId, origin: onPage ? `https://${pageHost}${page![2] ?? ""}` : `https://${rpId}` };
  };

  const structuredCloneJson = (v: Json): Json => JSON.parse(JSON.stringify(v)) as Json;

  const clientData = (type: string, challenge: Bytes, origin: string): string =>
    JSON.stringify({ type, challenge: toBase64Url(challenge), origin, crossOrigin: false });

  const timeoutOf = (v: unknown): number => {
    const n = Number(v);
    return Number.isFinite(n) && n > 0 ? Math.min(Math.max(n, 10_000), 600_000) : 300_000;
  };

  const descriptors = (list: unknown, what: string): Json[] => {
    if (list === undefined) return [];
    if (!Array.isArray(list))
      throw new TypeError(
        `Failed to read the '${what}' property: The provided value cannot be converted to a sequence.`,
      );
    return list
      .filter((d: Json) => d && d.type === "public-key")
      .map((d: Json) => ({
        id: toBase64Url(bytesOf(d.id, `${what}[].id`)),
        transports: Array.isArray(d.transports) ? d.transports.map(String) : [],
      }));
  };

  const run = (json: string, signal: SignalLike | undefined): Promise<Json> => {
    const answer = JSON.parse(json) as { token?: number };
    const settled = deps.call(json).catch((f: unknown) => {
      const failure = (f ?? {}) as { name?: string; message?: string; error?: string };
      throw new DOMException(
        failure.message ?? failure.error ?? "The operation failed.",
        failure.name ?? "NotAllowedError",
      );
    });
    if (!signal || answer.token === undefined) return settled;
    const token = answer.token;
    return new Promise((resolve, reject) => {
      const onAbort = (): void => {
        host.cancel(token);
        reject(signal.reason);
      };
      signal.addEventListener("abort", onAbort, { once: true });
      settled.then(
        (v) => {
          signal.removeEventListener("abort", onAbort);
          resolve(v);
        },
        (e: unknown) => {
          signal.removeEventListener("abort", onAbort);
          reject(e);
        },
      );
    });
  };

  class Credential {
    readonly #id: string;
    constructor(key?: symbol, id?: string) {
      if (key !== INTERNAL) throw new TypeError("Illegal constructor");
      this.#id = id!;
    }
    get id(): string {
      return this.#id;
    }
    get type(): string {
      return "public-key";
    }
  }

  class AuthenticatorResponse {
    readonly #clientData: Bytes;
    constructor(key?: symbol, clientData?: Bytes) {
      if (key !== INTERNAL) throw new TypeError("Illegal constructor");
      this.#clientData = clientData!;
    }
    get clientDataJSON(): ArrayBuffer {
      return buffer(this.#clientData);
    }
  }

  class AuthenticatorAttestationResponse extends AuthenticatorResponse {
    readonly #r: Json;
    constructor(key?: symbol, clientData?: Bytes, r?: Json) {
      super(key, clientData);
      this.#r = r!;
    }
    get attestationObject(): ArrayBuffer {
      return decoded(this.#r.attestationObject, "attestationObject");
    }
    getTransports(): string[] {
      return [...((this.#r.transports as string[]) ?? [])].sort();
    }
    getAuthenticatorData(): ArrayBuffer {
      return decoded(this.#r.authenticatorData, "authenticatorData");
    }
    getPublicKey(): ArrayBuffer | null {
      return typeof this.#r.publicKey === "string" ? decoded(this.#r.publicKey, "publicKey") : null;
    }
    getPublicKeyAlgorithm(): number {
      return Number(this.#r.publicKeyAlgorithm ?? 0);
    }
  }

  class AuthenticatorAssertionResponse extends AuthenticatorResponse {
    readonly #r: Json;
    constructor(key?: symbol, clientData?: Bytes, r?: Json) {
      super(key, clientData);
      this.#r = r!;
    }
    get authenticatorData(): ArrayBuffer {
      return decoded(this.#r.authenticatorData, "authenticatorData");
    }
    get signature(): ArrayBuffer {
      return decoded(this.#r.signature, "signature");
    }
    get userHandle(): ArrayBuffer | null {
      return typeof this.#r.userHandle === "string" ? decoded(this.#r.userHandle, "userHandle") : null;
    }
  }

  class PublicKeyCredential extends Credential {
    readonly #raw: Bytes;
    readonly #response: AuthenticatorResponse;
    readonly #attachment: string | null;
    readonly #extensions: Json;
    constructor(
      key?: symbol,
      raw?: Bytes,
      response?: AuthenticatorResponse,
      attachment?: string | null,
      extensions?: Json,
    ) {
      super(key, toBase64Url(raw ?? new Uint8Array()));
      this.#raw = raw!;
      this.#response = response!;
      this.#attachment = attachment ?? null;
      this.#extensions = extensions ?? {};
    }
    get rawId(): ArrayBuffer {
      return buffer(this.#raw);
    }
    get response(): AuthenticatorResponse {
      return this.#response;
    }
    get authenticatorAttachment(): string | null {
      return this.#attachment;
    }
    getClientExtensionResults(): Json {
      return structuredCloneJson(this.#extensions);
    }
    toJSON(): Json {
      const r = this.#response;
      const b = (a: ArrayBuffer): string => toBase64Url(new Uint8Array(a));
      const response: Json = { clientDataJSON: b(r.clientDataJSON) };
      if (r instanceof AuthenticatorAttestationResponse) {
        response.attestationObject = b(r.attestationObject);
        response.authenticatorData = b(r.getAuthenticatorData());
        response.transports = r.getTransports();
        const pk = r.getPublicKey();
        if (pk) response.publicKey = b(pk);
        response.publicKeyAlgorithm = r.getPublicKeyAlgorithm();
      } else if (r instanceof AuthenticatorAssertionResponse) {
        response.authenticatorData = b(r.authenticatorData);
        response.signature = b(r.signature);
        const uh = r.userHandle;
        if (uh) response.userHandle = b(uh);
      }
      return {
        id: this.id,
        rawId: this.id,
        type: "public-key",
        authenticatorAttachment: this.#attachment,
        clientExtensionResults: this.getClientExtensionResults(),
        response,
      };
    }

    static isUserVerifyingPlatformAuthenticatorAvailable(): Promise<boolean> {
      return Promise.resolve(host.available());
    }
    static isConditionalMediationAvailable(): Promise<boolean> {
      return Promise.resolve(false);
    }
    static getClientCapabilities(): Promise<Record<string, boolean>> {
      const uvpa = host.available();
      return Promise.resolve({
        conditionalCreate: false,
        conditionalGet: false,
        hybridTransport: true,
        passkeyPlatformAuthenticator: uvpa,
        userVerifyingPlatformAuthenticator: uvpa,
        relatedOrigins: false,
        signalAllAcceptedCredentials: false,
        signalCurrentUserDetails: false,
        signalUnknownCredential: false,
        "extension:credProps": true,
      });
    }
    static parseCreationOptionsFromJSON(options: Json): Json {
      const user = (options.user ?? {}) as Json;
      return {
        ...options,
        challenge: decoded(options.challenge, "challenge"),
        user: { ...user, id: decoded(user.id, "user.id") },
        ...(Array.isArray(options.excludeCredentials)
          ? {
              excludeCredentials: options.excludeCredentials.map((d: Json) => ({
                ...d,
                id: decoded(d.id, "excludeCredentials[].id"),
              })),
            }
          : {}),
      };
    }
    static parseRequestOptionsFromJSON(options: Json): Json {
      return {
        ...options,
        challenge: decoded(options.challenge, "challenge"),
        ...(Array.isArray(options.allowCredentials)
          ? {
              allowCredentials: options.allowCredentials.map((d: Json) => ({
                ...d,
                id: decoded(d.id, "allowCredentials[].id"),
              })),
            }
          : {}),
      };
    }
  }

  const create = async (options: Json | undefined): Promise<PublicKeyCredential> => {
    const signal = options?.signal as SignalLike | undefined;
    if (signal?.aborted) throw signal.reason;
    const pk = options?.publicKey as Json | undefined;
    if (!pk) throw new DOMException("Only 'publicKey' credentials are supported.", "NotSupportedError");
    const rp = (pk.rp ?? {}) as Json;
    const user = (pk.user ?? {}) as Json;
    if (rp.name === undefined)
      throw new TypeError(
        "Failed to execute 'create' on 'CredentialsContainer': Required member is undefined: rp.name",
      );
    for (const k of ["name", "displayName", "id"]) {
      if (user[k] === undefined) {
        throw new TypeError(
          `Failed to execute 'create' on 'CredentialsContainer': Required member is undefined: user.${k}`,
        );
      }
    }
    const userId = bytesOf(user.id, "user.id");
    if (userId.length < 1 || userId.length > 64) {
      throw new TypeError(
        "Failed to execute 'create' on 'CredentialsContainer': user.id must be between 1 and 64 bytes.",
      );
    }
    const challenge = bytesOf(pk.challenge, "challenge");
    let algs = Array.isArray(pk.pubKeyCredParams)
      ? pk.pubKeyCredParams.filter((p: Json) => p && p.type === "public-key").map((p: Json) => Number(p.alg))
      : [];
    if (Array.isArray(pk.pubKeyCredParams) && pk.pubKeyCredParams.length > 0 && algs.length === 0) {
      throw new DOMException("None of the requested credential types are supported.", "NotSupportedError");
    }
    if (algs.length === 0) algs = [-7, -257];
    const { rpId, origin } = relyingParty(rp.id);
    const sel = (pk.authenticatorSelection ?? {}) as Json;
    const residentKey = sel.residentKey ?? (sel.requireResidentKey === true ? "required" : "discouraged");
    const cd = clientData("webauthn.create", challenge, origin);
    const r = await run(
      host.create(
        JSON.stringify({
          rpId,
          rpName: String(rp.name),
          userId: toBase64Url(userId),
          userName: String(user.name),
          userDisplayName: String(user.displayName),
          clientDataJSON: cd,
          algs,
          timeout: timeoutOf(pk.timeout),
          exclude: descriptors(pk.excludeCredentials, "excludeCredentials"),
          attachment: sel.authenticatorAttachment ?? "",
          residentKey,
          userVerification: sel.userVerification ?? "preferred",
          attestation: pk.attestation ?? "none",
        }),
      ),
      signal,
    );
    const ext = (pk.extensions ?? {}) as Json;
    const extensions: Json = {};
    if (ext.credProps === true) {
      extensions.credProps = typeof r.residentKey === "boolean" ? { rk: r.residentKey } : {};
    }
    const raw = fromBase64Url(String(r.id)) ?? new Uint8Array();
    const response = new AuthenticatorAttestationResponse(INTERNAL, utf8(cd), r);
    return new PublicKeyCredential(INTERNAL, raw, response, (r.attachment as string | null) ?? null, extensions);
  };

  const get = async (options: Json | undefined): Promise<PublicKeyCredential> => {
    const signal = options?.signal as SignalLike | undefined;
    if (signal?.aborted) throw signal.reason;
    const pk = options?.publicKey as Json | undefined;
    if (!pk) throw new DOMException("Only 'publicKey' credentials are supported.", "NotSupportedError");
    if (options?.mediation === "conditional") {
      throw new DOMException("Conditional mediation is not supported.", "NotSupportedError");
    }
    const challenge = bytesOf(pk.challenge, "challenge");
    const { rpId, origin } = relyingParty(pk.rpId);
    const cd = clientData("webauthn.get", challenge, origin);
    const r = await run(
      host.get(
        JSON.stringify({
          rpId,
          clientDataJSON: cd,
          timeout: timeoutOf(pk.timeout),
          allow: descriptors(pk.allowCredentials, "allowCredentials"),
          userVerification: pk.userVerification ?? "preferred",
        }),
      ),
      signal,
    );
    const raw = fromBase64Url(String(r.id)) ?? new Uint8Array();
    const response = new AuthenticatorAssertionResponse(INTERNAL, utf8(cd), r);
    return new PublicKeyCredential(INTERNAL, raw, response, (r.attachment as string | null) ?? null, {});
  };

  class CredentialsContainer {
    constructor(key?: symbol) {
      if (key !== INTERNAL) throw new TypeError("Illegal constructor");
    }
    create(options?: Json): Promise<PublicKeyCredential> {
      return create(options);
    }
    get(options?: Json): Promise<PublicKeyCredential> {
      return get(options);
    }
    store(): Promise<never> {
      return Promise.reject(new DOMException("Only 'publicKey' credentials are supported.", "NotSupportedError"));
    }
    preventSilentAccess(): Promise<void> {
      return Promise.resolve();
    }
  }

  const g = globalThis as Record<string, unknown>;
  Object.assign(g, {
    Credential,
    PublicKeyCredential,
    AuthenticatorResponse,
    AuthenticatorAttestationResponse,
    AuthenticatorAssertionResponse,
    CredentialsContainer,
  });
  Object.defineProperty(nav, "credentials", {
    value: new CredentialsContainer(INTERNAL),
    configurable: true,
    enumerable: true,
  });
}
