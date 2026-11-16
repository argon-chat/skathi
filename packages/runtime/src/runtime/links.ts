import type { MirrorElement, MirrorNode } from "./types";
import { isMirrorNode } from "./listeners";
import { warnCompat } from "../compat";
import { fsHost, openContentUrl } from "./activation";
import { activateFormButton, isResetButton, isSubmitButton } from "./forms";
import { activateLabel } from "./labels";
import { fileInputOps, isColorInput, isFileInput, isLabel, showColorPicker, showFilePicker } from "./file-input";

// There is no document to navigate, so a link hands its URL to the system. Links are reachable by
// CONTENT, so the scheme allowlist (http, https, mailto) is the security line; other schemes are
// refused with an advisory naming `appShell.openExternal`, the call for URLs the app trusts.

export const CONTENT_SCHEMES = new Set(["http", "https", "mailto"]);
export const SCHEME_RE = /^([a-zA-Z][a-zA-Z0-9+.-]*):/;

export function isLink(n: MirrorNode): n is MirrorElement {
  return n.kind === 1 && (n.tag === "a" || n.tag === "area") && n.attrs != null && n.attrs.href != null;
}

const ACTIVATABLE = new Set(["button", "input", "label", "summary"]);
const hasActivationBehaviour = (n: MirrorNode): n is MirrorElement =>
  n.kind === 1 && (ACTIVATABLE.has(n.tag) || isLink(n));

/** DOM "dispatch" step 5.5: only a `MouseEvent` click activates; an engine without MouseEvent (unit tests) counts any click. */
export function isActivationEvent(ev: Event): boolean {
  return typeof MouseEvent !== "function" || ev instanceof MouseEvent;
}

/** A synthetic click on a `--window-control` caption button acts as a real press, whose action the OS performs itself. */
export function activateWindowControl(target: MirrorNode): void {
  if (isMirrorNode(target) && typeof __sk_window_control_click === "function") __sk_window_control_click(target.__skId);
}

export function activate(target: MirrorNode, bubbles: boolean): void {
  if (!isMirrorNode(target)) return;
  for (let n: MirrorNode | null = target; n; n = n.parentNode) {
    if (hasActivationBehaviour(n)) {
      if (isFileInput(n)) showFilePicker(fileInputOps(n), null);
      else if (isColorInput(n)) showColorPicker(n, null);
      else if (isLabel(n)) activateLabel(n, target);
      else if (isLink(n)) followLink(n);
      else if (isSubmitButton(n) || isResetButton(n)) activateFormButton(n);
      return;
    }
    if (!bubbles) return;
  }
}

function followLink(a: MirrorElement): void {
  followHref(String(a.attrs.href ?? ""), a.attrs.download ?? null);
}

export function followHref(href: string, download: string | null): void {
  const raw = href.trim();
  if (raw === "" || raw.startsWith("#")) return;
  const m = SCHEME_RE.exec(raw);
  if (!m) {
    warnCompat(
      "link:relative",
      `<a href="${raw}"> — a relative link has no document to navigate in Skathi, so ` +
        `activating it does nothing. In-app links belong to vue-router's <RouterLink> (which ` +
        `handles the click itself); an external one needs an absolute https: URL`,
    );
    return;
  }
  const scheme = m[1]!.toLowerCase();
  if (scheme === "javascript") {
    // `javascript:void(0)` is the "link with only a click handler" idiom; anything else asked for code to run.
    if (!/^javascript:\s*(void\s*\(?\s*0\s*\)?\s*;?\s*|;?)$/i.test(raw)) {
      warnCompat(
        "link:javascript",
        `<a href="javascript:…"> — javascript: URLs are not executed (CSP-style); put the code ` +
          `in a @click handler`,
      );
    }
    return;
  }
  if (download != null && (scheme === "blob" || scheme === "data")) {
    // Resolved now, at the click, so `a.click(); URL.revokeObjectURL(url)` still saves.
    if (fsHost) fsHost.download(raw, download);
    else warnCompat("download:nohost", "<a download>: this host has no sk-desktop; nothing was saved");
    return;
  }
  if (!CONTENT_SCHEMES.has(scheme)) {
    adviseScheme(scheme, "<a href>");
    return;
  }
  openContentUrl(raw, "<a href>");
}

export function adviseScheme(scheme: string, via: string): void {
  warnCompat(
    `link-scheme:${scheme}`,
    `${via} with a ${scheme}: URL was refused: links and window.open() only hand http, https ` +
      `and mailto to the system, because content can reach them. If the app trusts this URL, ` +
      `open it from your own code with appShell.openExternal(url)`,
  );
}
