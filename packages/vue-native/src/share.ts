// Web Share: `navigator.share` and `navigator.canShare` through the system share sheet.

export interface ShareHost {
  /** `{ title, text?, url?, files: names }` and the files' bytes in the same order; answers `{ token }` or a refusal. */
  share(requestJson: string, buffers: ArrayBuffer[]): string;
}

interface FileLike {
  readonly name: string;
  arrayBuffer(): Promise<ArrayBuffer>;
}

export interface ShareDeps {
  host: ShareHost;
  call(json: string): Promise<unknown>;
  resolveUrl(url: string): string;
  isFile(v: unknown): v is FileLike;
  activation: { isActive(): boolean; consume(): void };
  documentTitle(): string;
  DOMException: new (message?: string, name?: string) => Error;
}

interface ShareData {
  title?: unknown;
  text?: unknown;
  url?: unknown;
  files?: unknown;
}

interface Checked {
  title?: string;
  text?: string;
  url?: string;
  files: FileLike[];
}

export function installShare(nav: object, deps: ShareDeps): void {
  const { DOMException } = deps;
  let busy = false;

  const check = (data: ShareData | undefined, method: string): Checked => {
    const fail = (m: string): never => {
      throw new TypeError(`Failed to execute '${method}' on 'Navigator': ${m}`);
    };
    const d = data ?? {};
    if (typeof d !== "object") fail("The provided value is not of type 'ShareData'.");
    const out: Checked = { files: [] };
    if (d.title !== undefined) out.title = String(d.title);
    if (d.text !== undefined) out.text = String(d.text);
    if (d.url !== undefined) {
      try {
        out.url = deps.resolveUrl(String(d.url));
      } catch {
        fail(`Invalid URL`);
      }
    }
    if (d.files !== undefined) {
      if (d.files === null || typeof d.files !== "object" || !(Symbol.iterator in d.files)) {
        fail("The provided value cannot be converted to a sequence.");
      }
      for (const f of d.files as Iterable<unknown>) {
        if (!deps.isFile(f)) return fail("Failed to convert value to 'File'.");
        out.files.push(f);
      }
    }
    if (out.title === undefined && out.text === undefined && out.url === undefined && out.files.length === 0) {
      fail(
        "No known share data fields supplied. If using only new fields (other than title, text and url), you must feature-detect them first.",
      );
    }
    return out;
  };

  const run = async (data?: ShareData): Promise<void> => {
    if (busy) throw new DOMException("An earlier share has not yet completed.", "InvalidStateError");
    if (!deps.activation.isActive()) {
      throw new DOMException("Must be handling a user gesture to perform a share request.", "NotAllowedError");
    }
    const c = check(data, "share");
    deps.activation.consume();
    busy = true;
    try {
      const buffers = await Promise.all(c.files.map((f) => f.arrayBuffer()));
      const request = {
        title: c.title || deps.documentTitle() || c.url || "",
        ...(c.text !== undefined ? { text: c.text } : {}),
        ...(c.url !== undefined ? { url: c.url } : {}),
        files: c.files.map((f) => f.name),
      };
      await deps.call(deps.host.share(JSON.stringify(request), buffers)).catch((f: unknown) => {
        const failure = (f ?? {}) as { name?: string; message?: string; error?: string };
        throw new DOMException(failure.message ?? failure.error ?? "Share failed", failure.name ?? "AbortError");
      });
    } finally {
      busy = false;
    }
  };

  Object.defineProperties(nav, {
    share: {
      value: function share(data?: ShareData): Promise<void> {
        return run(data);
      },
      configurable: true,
      writable: true,
    },
    canShare: {
      value: function canShare(data?: ShareData): boolean {
        try {
          check(data, "canShare");
          return true;
        } catch {
          return false;
        }
      },
      configurable: true,
      writable: true,
    },
  });
}
