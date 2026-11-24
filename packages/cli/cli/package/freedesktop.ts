// The freedesktop.org side of a Linux package: the desktop entry, hicolor icons, AppStream metainfo
// and the shared-mime-info package for the app's own file types, laid out under `usr/` the same way
// whichever format carries it.

import type { FileAssociation, PackageContext, UrlProtocol } from "./types";

export const ICON_SIZES = [16, 32, 48, 64, 128, 256, 512] as const;

/** Main categories of the Desktop Menu spec; an entry should name at least one. */
export const MAIN_CATEGORIES: ReadonlySet<string> = new Set([
  "AudioVideo",
  "Audio",
  "Video",
  "Development",
  "Education",
  "Game",
  "Graphics",
  "Network",
  "Office",
  "Science",
  "Settings",
  "System",
  "Utility",
]);

export const mimeTypeOf = (a: FileAssociation): string => a.mimeType ?? `application/x-${a.extensions[0]}`;

/** The `Exec` arguments. A link launch goes through the host's protocol marker so the URL cannot carry host flags. */
export function execArgs(files: boolean, urls: boolean): string {
  if (files && urls) return " %U";
  if (urls) return " --sk-protocol-launch -- %u";
  if (files) return " %F";
  return "";
}

const escapeValue = (s: string) =>
  s.replace(/\\/g, "\\\\").replace(/\n/g, "\\n").replace(/\t/g, "\\t").replace(/\r/g, "\\r");
const list = (items: readonly string[]) => items.map((i) => escapeValue(i).replace(/;/g, "\\;") + ";").join("");

/** An `Exec` program argument, quoted when the spec's reserved characters are in it. */
export function execQuote(arg: string): string {
  if (!/[\s"'\\><~|&;$*?#()`]/.test(arg)) return arg;
  return `"${arg.replace(/(["`$\\])/g, "\\$1")}"`;
}

export interface DesktopEntryInput {
  name: string;
  comment: string;
  /** What `Exec` starts: a program on PATH or an absolute path. */
  exec: string;
  icon: string | null;
  categories: readonly string[];
  fileAssociations: readonly FileAssociation[];
  protocols: readonly UrlProtocol[];
  /** The window's app id (X11 class), so the shell matches windows to this entry. */
  wmClass: string;
  extra?: Readonly<Record<string, string>>;
}

export function desktopEntry(d: DesktopEntryInput): string {
  const mime = [
    ...new Set(d.fileAssociations.map(mimeTypeOf)),
    ...d.protocols.map((p) => `x-scheme-handler/${p.scheme}`),
  ];
  const lines = [
    "[Desktop Entry]",
    "Type=Application",
    `Name=${escapeValue(d.name)}`,
    `Comment=${escapeValue(d.comment)}`,
    `Exec=${execQuote(d.exec)}${execArgs(d.fileAssociations.length > 0, d.protocols.length > 0)}`,
    ...(d.icon ? [`Icon=${escapeValue(d.icon)}`] : []),
    "Terminal=false",
    `Categories=${list(d.categories)}`,
    ...(mime.length ? [`MimeType=${list(mime)}`] : []),
    `StartupWMClass=${escapeValue(d.wmClass)}`,
    ...Object.entries(d.extra ?? {}).map(([k, v]) => `${k}=${escapeValue(v)}`),
  ];
  return lines.join("\n") + "\n";
}

const xml = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

export interface MetainfoInput {
  id: string;
  name: string;
  summary: string;
  developer: string;
  version: string;
  /** SPDX expression; null leaves `project_license` out. */
  license: string | null;
  homepage: string | null;
  binary: string;
  mimeTypes: readonly string[];
  /** `YYYY-MM-DD`. */
  date: string;
}

export function metainfo(m: MetainfoInput): string {
  const summary = m.summary.replace(/[.\s]+$/, "");
  return [
    '<?xml version="1.0" encoding="UTF-8"?>',
    '<component type="desktop-application">',
    `  <id>${xml(m.id)}</id>`,
    "  <metadata_license>CC0-1.0</metadata_license>",
    ...(m.license ? [`  <project_license>${xml(m.license)}</project_license>`] : []),
    `  <name>${xml(m.name)}</name>`,
    `  <summary>${xml(summary)}</summary>`,
    "  <description>",
    `    <p>${xml(m.summary)}</p>`,
    "  </description>",
    `  <developer id="${xml(m.id.split(".").slice(0, 2).join("."))}">`,
    `    <name>${xml(m.developer)}</name>`,
    "  </developer>",
    ...(m.homepage ? [`  <url type="homepage">${xml(m.homepage)}</url>`] : []),
    `  <launchable type="desktop-id">${xml(m.id)}.desktop</launchable>`,
    "  <provides>",
    `    <binary>${xml(m.binary)}</binary>`,
    ...m.mimeTypes.map((t) => `    <mediatype>${xml(t)}</mediatype>`),
    "  </provides>",
    "  <releases>",
    `    <release version="${xml(m.version)}" date="${m.date}"/>`,
    "  </releases>",
    '  <content_rating type="oars-1.1"/>',
    "</component>",
    "",
  ].join("\n");
}

/** A shared-mime-info package for the app's own types (none given, `x-` or `vnd.`); null when there are none. */
export function mimePackage(associations: readonly FileAssociation[]): string | null {
  const own = associations.filter((a) => a.extensions.length && (!a.mimeType || /\/(x-|vnd\.)/.test(a.mimeType)));
  if (!own.length) return null;
  const seen = new Set<string>();
  const types = own.flatMap((a) => {
    const type = mimeTypeOf(a);
    if (seen.has(type)) return [];
    seen.add(type);
    return [
      `  <mime-type type="${xml(type)}">`,
      `    <comment>${xml(a.name)}</comment>`,
      ...a.extensions.map((e) => `    <glob pattern="*.${xml(e)}"/>`),
      "  </mime-type>",
    ];
  });
  return [
    '<?xml version="1.0" encoding="UTF-8"?>',
    '<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">',
    ...types,
    "</mime-info>",
    "",
  ].join("\n");
}

/** Every file a format puts under `usr/share`, keyed by its path below `usr/share`. */
export function shareFiles(
  ctx: Pick<PackageContext, "icon" | "config">,
  entry: string,
  meta: string,
): Map<string, Uint8Array> {
  const id = ctx.config.identity.id;
  const enc = new TextEncoder();
  const files = new Map<string, Uint8Array>();
  files.set(`applications/${id}.desktop`, enc.encode(entry));
  files.set(`metainfo/${id}.metainfo.xml`, enc.encode(meta));
  for (const size of ICON_SIZES) {
    const png = ctx.icon(size);
    if (png) files.set(`icons/hicolor/${size}x${size}/apps/${id}.png`, png);
  }
  const mime = mimePackage(ctx.config.fileAssociations);
  if (mime) files.set(`mime/packages/${id}.xml`, enc.encode(mime));
  return files;
}
