import { expect, test } from "bun:test";
import { fromBase64Url, installCredentials, toBase64Url } from "../src/credentials";

class FakeDOMException extends Error {
  constructor(message?: string, name?: string) {
    super(message);
    this.name = name ?? "Error";
  }
}

type Json = Record<string, unknown>;
interface Cred {
  id: string;
  rawId: ArrayBuffer;
  type: string;
  authenticatorAttachment: string | null;
  response: Json & {
    clientDataJSON: ArrayBuffer;
    getPublicKey?(): ArrayBuffer | null;
    getTransports?(): string[];
  };
  getClientExtensionResults(): Json;
  toJSON(): Json;
}
interface Container {
  create(o?: Json): Promise<Cred>;
  get(o?: Json): Promise<Cred>;
}

const enc = (s: string) => toBase64Url(new TextEncoder().encode(s));

function setup(pageUrl = "app://redjs/index.html", answer?: (req: Json) => Promise<Json>) {
  const requests: Json[] = [];
  const cancelled: number[] = [];
  const nav = {} as { credentials: Container };
  installCredentials(nav, {
    host: {
      create: (json) => {
        requests.push({ op: "create", ...JSON.parse(json) });
        return '{"token":7}';
      },
      get: (json) => {
        requests.push({ op: "get", ...JSON.parse(json) });
        return '{"token":8}';
      },
      cancel: (t) => void cancelled.push(t),
      available: () => true,
    },
    call: () =>
      answer
        ? answer(requests[requests.length - 1]!)
        : Promise.resolve(
            requests[requests.length - 1]!.op === "create"
              ? {
                  id: enc("cred"),
                  attestationObject: enc("att"),
                  authenticatorData: enc("auth"),
                  transports: ["internal", "hybrid"],
                  attachment: "platform",
                  publicKey: enc("spki"),
                  publicKeyAlgorithm: -7,
                  residentKey: true,
                }
              : {
                  id: enc("cred"),
                  authenticatorData: enc("auth"),
                  signature: enc("sig"),
                  userHandle: enc("u1"),
                  attachment: "platform",
                },
          ),
    pageUrl: () => pageUrl,
    DOMException: FakeDOMException,
  });
  return { c: nav.credentials, requests, cancelled };
}

const createOptions = (rp: Json = { id: "example.com", name: "Example" }): Json => ({
  publicKey: {
    rp,
    user: { id: new TextEncoder().encode("u1"), name: "ann", displayName: "Ann" },
    challenge: new Uint8Array([1, 2, 3]),
    pubKeyCredParams: [{ type: "public-key", alg: -7 }],
    authenticatorSelection: { residentKey: "required", userVerification: "required" },
    extensions: { credProps: true },
  },
});

test("base64url round-trips and refuses garbage", () => {
  for (const n of [0, 1, 2, 3, 4, 31, 32, 33]) {
    const b = new Uint8Array(n).map((_, i) => (i * 37 + 11) & 255);
    expect([...fromBase64Url(toBase64Url(b))!]).toEqual([...b]);
  }
  expect(fromBase64Url("a")).toBeNull();
  expect(fromBase64Url("ab$c")).toBeNull();
});

test("create sends the request and builds the credential", async () => {
  const { c, requests } = setup();
  const cred = await c.create(createOptions());
  const req = requests[0]!;
  expect(req).toMatchObject({
    rpId: "example.com",
    rpName: "Example",
    userId: enc("u1"),
    algs: [-7],
    residentKey: "required",
    userVerification: "required",
    attestation: "none",
  });
  expect(JSON.parse(req.clientDataJSON as string)).toEqual({
    type: "webauthn.create",
    challenge: "AQID",
    origin: "https://example.com",
    crossOrigin: false,
  });
  expect(new TextDecoder().decode(cred.response.clientDataJSON)).toBe(req.clientDataJSON as string);
  expect([cred.id, cred.type, cred.authenticatorAttachment]).toEqual([enc("cred"), "public-key", "platform"]);
  expect(new TextDecoder().decode(cred.rawId)).toBe("cred");
  expect(cred.response.getTransports!()).toEqual(["hybrid", "internal"]);
  expect(new TextDecoder().decode(cred.response.getPublicKey!()!)).toBe("spki");
  expect(cred.getClientExtensionResults()).toEqual({ credProps: { rk: true } });
  expect(cred.toJSON()).toMatchObject({
    id: enc("cred"),
    rawId: enc("cred"),
    response: { attestationObject: enc("att"), publicKeyAlgorithm: -7, transports: ["hybrid", "internal"] },
  });
});

test("get uses the page's own origin when it is the relying party", async () => {
  const { c, requests } = setup("https://login.example.com/app");
  const cred = await c.get({
    publicKey: { challenge: new Uint8Array([9]), allowCredentials: [{ type: "public-key", id: new Uint8Array([5]) }] },
  });
  expect(requests[0]).toMatchObject({ rpId: "login.example.com", allow: [{ id: "BQ", transports: [] }] });
  expect(JSON.parse(requests[0]!.clientDataJSON as string).origin).toBe("https://login.example.com");
  expect(cred.toJSON()).toMatchObject({ response: { signature: enc("sig"), userHandle: enc("u1") } });

  await c.get({ publicKey: { challenge: new Uint8Array([9]), rpId: "example.com" } });
  expect(JSON.parse(requests[1]!.clientDataJSON as string).origin).toBe("https://login.example.com");
});

test("without a web origin the relying party must be named", async () => {
  const { c } = setup();
  await expect(c.create(createOptions({ name: "Example" }))).rejects.toMatchObject({ name: "SecurityError" });
  await expect(c.get({ publicKey: { challenge: new Uint8Array([1]) } })).rejects.toMatchObject({
    name: "SecurityError",
  });
  await expect(c.get({ publicKey: { challenge: new Uint8Array([1]), rpId: "bad domain" } })).rejects.toMatchObject({
    name: "SecurityError",
  });
  await expect(c.get({})).rejects.toMatchObject({ name: "NotSupportedError" });
});

test("an abort cancels the host request and rejects with the reason", async () => {
  const { c, cancelled } = setup("app://x", () => new Promise(() => {}));
  const ac = new AbortController();
  const p = c.get({ publicKey: { challenge: new Uint8Array([1]), rpId: "example.com" }, signal: ac.signal });
  ac.abort(new Error("stop"));
  await expect(p).rejects.toThrow("stop");
  expect(cancelled).toEqual([8]);
});

test("host failures keep their WebAuthn name", async () => {
  const { c } = setup("app://x", () => Promise.reject({ name: "InvalidStateError", message: "already registered" }));
  await expect(c.create(createOptions())).rejects.toMatchObject({ name: "InvalidStateError" });
});

test("the JSON parsers decode the binary members", () => {
  const PKC = (
    globalThis as unknown as {
      PublicKeyCredential: { parseCreationOptionsFromJSON(o: Json): Json; parseRequestOptionsFromJSON(o: Json): Json };
    }
  ).PublicKeyCredential;
  const o = PKC.parseCreationOptionsFromJSON({
    challenge: "AQID",
    rp: { name: "x" },
    user: { id: "dTE", name: "a", displayName: "A" },
    excludeCredentials: [{ type: "public-key", id: "BQ" }],
  });
  expect([...new Uint8Array(o.challenge as ArrayBuffer)]).toEqual([1, 2, 3]);
  expect(new TextDecoder().decode((o.user as Json).id as ArrayBuffer)).toBe("u1");
  expect(() => PKC.parseRequestOptionsFromJSON({ challenge: "!!" })).toThrow(TypeError);
});
