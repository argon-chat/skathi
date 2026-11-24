// Windows MSIX: AppxManifest.xml from the package config, visual assets from the icon, the build folder as the
// payload, and an optional .appinstaller feed. The package is written unsigned (appx.ts); `ctx.sign` signs it.

import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { iconAt, iconOnCanvas, iconSources } from "./icons";
import { reservedPath, writeAppx } from "./appx";
import type {
  AppInstallerOptions,
  Artifact,
  MsixOptions,
  OptionsContext,
  PackageCheckContext,
  PackageConfig,
  PackageContext,
  PackageFormat,
  Problem,
} from "./types";

export const MSIX_KEYS = [
  "publisher",
  "identityName",
  "architecture",
  "minVersion",
  "maxVersionTested",
  "capabilities",
  "background",
  "appInstaller",
] as const;
export const APP_INSTALLER_KEYS = ["url", "hoursBetweenUpdateChecks", "showPrompt", "updateBlocksActivation"] as const;

/** Windows 10 1809: the oldest release whose installer takes uap3 protocols and file types from a full-trust app. */
export const MIN_VERSION = "10.0.17763.0";
export const MAX_VERSION_TESTED = "10.0.26100.0";
const ARCHITECTURES: readonly MsixOptions["architecture"][] = ["x64", "arm64", "x86", "neutral"];

const CAPABILITIES: Readonly<Record<string, "foundation" | "uap" | "rescap" | "device">> = {
  internetClient: "foundation",
  internetClientServer: "foundation",
  privateNetworkClientServer: "foundation",
  documentsLibrary: "uap",
  picturesLibrary: "uap",
  videosLibrary: "uap",
  musicLibrary: "uap",
  removableStorage: "uap",
  enterpriseAuthentication: "uap",
  sharedUserCertificates: "uap",
  userAccountInformation: "uap",
  contacts: "uap",
  appointments: "uap",
  broadFileSystemAccess: "rescap",
  unvirtualizedResources: "rescap",
  allowElevation: "rescap",
  microphone: "device",
  webcam: "device",
  location: "device",
  bluetooth: "device",
  proximity: "device",
  radios: "device",
  serialcommunication: "device",
  usb: "device",
  humaninterfacedevice: "device",
  gazeInput: "device",
};

const RDN_TYPES = new Set([
  "CN",
  "L",
  "O",
  "OU",
  "E",
  "C",
  "S",
  "STREET",
  "T",
  "G",
  "I",
  "SN",
  "DC",
  "SERIALNUMBER",
  "Description",
  "PostalCode",
  "POBox",
  "Phone",
  "X21Address",
  "dnQualifier",
]);
const OID = /^OID\.(0|[1-9]\d*)(\.(0|[1-9]\d*))+$/;
const UNQUOTED = /^[^,+="<>#;]+/;

export interface Rdn {
  type: string;
  /** Unquoted, with `""` read as `"`. */
  value: string;
}

/**
 * The attributes of a certificate subject as Windows prints it and the manifest schema takes it:
 * `TYPE=value` joined by ", ", a value either free of `,+="<>#;` or quoted (`""` inside is a quote).
 */
export function parsePublisher(dn: string): { rdns: Rdn[] } | { error: string } {
  if (!dn) return { error: "is empty" };
  if (dn.length > 8192) return { error: "is longer than 8192 characters" };
  const rdns: Rdn[] = [];
  let i = 0;
  while (i < dn.length) {
    if (rdns.length) {
      if (dn.slice(i, i + 2) !== ", ") {
        return {
          error: `needs ", " (a comma and one space) between attributes at character ${i + 1}, as Windows prints the subject`,
        };
      }
      i += 2;
    }
    const eq = dn.indexOf("=", i);
    if (eq < 0) return { error: `has no "=" after ${JSON.stringify(dn.slice(i))}` };
    const type = dn.slice(i, eq);
    if (!RDN_TYPES.has(type) && !OID.test(type)) {
      return {
        error: `has the attribute type ${JSON.stringify(type)}; Windows takes ${[...RDN_TYPES].join(", ")} or OID.<n>.<n>…, spelled as here`,
      };
    }
    i = eq + 1;
    if (dn[i] === '"') {
      let j = i + 1;
      let value = "";
      for (;;) {
        const q = dn.indexOf('"', j);
        if (q < 0) return { error: `has an unclosed quote in the value of ${type}` };
        value += dn.slice(j, q);
        if (dn[q + 1] === '"') {
          value += '"';
          j = q + 2;
          continue;
        }
        i = q + 1;
        break;
      }
      rdns.push({ type, value });
    } else {
      const m = UNQUOTED.exec(dn.slice(i));
      if (!m) {
        return {
          error: `has a value of ${type} that is empty or contains one of , + = " < > # ; without quotes; quote it as the certificate does: ${type}="…"`,
        };
      }
      i += m[0].length;
      rdns.push({ type, value: m[0] });
    }
  }
  return { rdns };
}

/** `Identity/@Name`: letters, digits, "." and "-", 3 to 50 characters. */
const IDENTITY_NAME = /^[A-Za-z0-9.-]{3,50}$/;
const WINDOWS_VERSION = /^\d{1,5}\.\d{1,5}\.\d{1,5}\.\d{1,5}$/;
const COLOR = /^(transparent|#[0-9a-fA-F]{6})$/;

const isObject = (v: unknown): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v);

function parseOptions(raw: Record<string, unknown>, ctx: OptionsContext, problems: Problem[]): MsixOptions {
  const error = (key: string, message: string) =>
    problems.push({
      code: "SK8001",
      severity: "error",
      key: `${ctx.at}.${key}`,
      message: `"${ctx.at}.${key}" ${message}`,
    });
  const string = (key: string): string | undefined => {
    const v = raw[key];
    if (v === undefined) return undefined;
    if (typeof v === "string" && v.trim()) return v.trim();
    error(key, "must be a non-empty string");
    return undefined;
  };

  const publisher = string("publisher") ?? "";
  if (publisher) {
    const parsed = parsePublisher(publisher);
    if ("error" in parsed)
      error("publisher", `${parsed.error}. Copy the subject from the signing certificate (see the packaging guide)`);
  }
  const identityName = string("identityName") ?? ctx.identity.id;
  let architecture: MsixOptions["architecture"] = process.arch === "arm64" ? "arm64" : "x64";
  const arch = string("architecture");
  if (arch !== undefined) {
    const known = ARCHITECTURES.find((a) => a === arch);
    if (known) architecture = known;
    else error("architecture", `must be ${ARCHITECTURES.map((a) => JSON.stringify(a)).join(", ")}`);
  }
  const version = (key: string, fallback: string) => {
    const v = string(key);
    if (v === undefined) return fallback;
    if (!WINDOWS_VERSION.test(v)) error(key, `must be a Windows version like "${fallback}"`);
    return v;
  };
  const capabilities: string[] = [];
  if (raw.capabilities !== undefined) {
    if (!Array.isArray(raw.capabilities)) error("capabilities", 'must be a list, e.g. ["microphone", "webcam"]');
    else
      for (const c of raw.capabilities) {
        if (c === "runFullTrust") continue;
        if (typeof c !== "string" || !(c in CAPABILITIES)) {
          error(
            "capabilities",
            `has ${JSON.stringify(c)}, which is not a capability sk knows; known: ${Object.keys(CAPABILITIES).join(", ")}`,
          );
        } else if (!capabilities.includes(c)) capabilities.push(c);
      }
  }
  const background = string("background") ?? "transparent";
  if (!COLOR.test(background)) error("background", 'must be "transparent" or a colour like "#1e1e2e"');

  let appInstaller: AppInstallerOptions | null = null;
  const ai = raw.appInstaller;
  if (ai !== undefined && ai !== null) {
    const at = "appInstaller";
    if (!isObject(ai)) error(at, 'must be an object, e.g. { "url": "https://example.com/downloads/notes" }');
    else {
      for (const key of Object.keys(ai))
        if (!key.startsWith("$") && !(APP_INSTALLER_KEYS as readonly string[]).includes(key))
          error(`${at}.${key}`, `is not a key of appInstaller; the keys are ${APP_INSTALLER_KEYS.join(", ")}`);
      const url = typeof ai.url === "string" ? ai.url.trim().replace(/\/+$/, "") : "";
      if (!/^https:\/\/[^\s/]+/.test(url)) {
        error(`${at}.url`, "must be the https URL of the folder the .msix and .appinstaller are uploaded to");
      }
      const hours = ai.hoursBetweenUpdateChecks ?? 24;
      if (typeof hours !== "number" || !Number.isInteger(hours) || hours < 0 || hours > 255) {
        error(`${at}.hoursBetweenUpdateChecks`, "must be a whole number of hours from 0 to 255");
      }
      const bool = (key: string, dflt: boolean) => {
        const v = ai[key];
        if (v === undefined) return dflt;
        if (typeof v !== "boolean") error(`${at}.${key}`, "must be true or false");
        return v === true;
      };
      appInstaller = {
        url,
        hoursBetweenUpdateChecks: typeof hours === "number" ? hours : 24,
        showPrompt: bool("showPrompt", false),
        updateBlocksActivation: bool("updateBlocksActivation", false),
      };
    }
  }
  return {
    publisher,
    identityName,
    architecture,
    minVersion: version("minVersion", MIN_VERSION),
    maxVersionTested: version("maxVersionTested", MAX_VERSION_TESTED),
    capabilities,
    background,
    appInstaller,
  };
}

function validate(ctx: PackageCheckContext): Problem[] {
  const problems: Problem[] = [];
  const o = ctx.config.windows.msix;
  const at = "package.windows.msix";
  const error = (key: string, message: string) =>
    problems.push({ code: "SK8001", severity: "error", key: `${at}.${key}`, message });
  const warn = (key: string, message: string) =>
    problems.push({ code: "SK8002", severity: "warning", key: `${at}.${key}`, message });
  if (!o.publisher) {
    error(
      "publisher",
      `MSIX needs "${at}.publisher": the subject of the certificate the package is signed with, e.g. "CN=Example Ltd, O=Example Ltd, C=US"`,
    );
  }
  if (!IDENTITY_NAME.test(o.identityName)) {
    error(
      "identityName",
      `the package name ${JSON.stringify(o.identityName)} must be 3 to 50 letters, digits, "." or "-"; set "${at}.identityName"`,
    );
  }
  const parts = ctx.config.identity.version.split(".").map(Number);
  if (parts.some((p) => p > 65535)) {
    problems.push({
      code: "SK8001",
      severity: "error",
      key: "package.version",
      message: `MSIX takes version parts up to 65535; ${ctx.config.identity.version} has a larger one`,
    });
  }
  if (!publisherDisplayName(ctx.config)) {
    error("publisher", 'set "package.publisher": the publisher\'s name as Windows shows it to users');
  }
  const asks = [
    [ctx.app.macos.microphoneUsageDescription, "microphone"],
    [ctx.app.macos.cameraUsageDescription, "webcam"],
  ] as const;
  for (const [reason, cap] of asks) {
    if (reason && !o.capabilities.includes(cap)) {
      warn(
        "capabilities",
        `the app asks for the ${cap === "webcam" ? "camera" : cap} on macOS, but "${at}.capabilities" lacks "${cap}": Windows does not list the packaged app under that privacy setting and denies it`,
      );
    }
  }
  return problems;
}

/** `PublisherDisplayName`: `package.publisher`, else the subject's O, else its CN. */
export function publisherDisplayName(config: PackageConfig): string {
  if (config.identity.publisher) return config.identity.publisher;
  const parsed = parsePublisher(config.windows.msix.publisher);
  if ("error" in parsed) return "";
  return (parsed.rdns.find((r) => r.type === "O") ?? parsed.rdns.find((r) => r.type === "CN"))?.value ?? "";
}

export const xml = (s: string) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

/** `1.2.3` as `1.2.3.0`. */
export const msixVersion = (v: string) => [...v.split("."), "0", "0", "0"].slice(0, 4).join(".");

export interface ManifestInput {
  config: PackageConfig;
  /** Inside the package, `\` separated. */
  executable: string;
  assets: { storeLogo: string; square150: string; square44: string; wide310: string | null };
  /** Per association, its logo inside the package, or null. */
  fileTypeLogos: readonly (string | null)[];
}

const fileTypeName = (name: string, i: number, taken: Set<string>) => {
  let base =
    name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 90) || `files-${i}`;
  if (!/^[a-z0-9]/.test(base)) base = `t${base}`;
  let n = base;
  for (let k = 2; taken.has(n); k++) n = `${base}-${k}`;
  taken.add(n);
  return n;
};

export function appxManifest({ config, executable, assets, fileTypeLogos }: ManifestInput): string {
  const o = config.windows.msix;
  const id = config.identity;
  const extensions: string[] = [];
  for (const p of config.protocols) {
    extensions.push(
      '        <uap3:Extension Category="windows.protocol">',
      `          <uap3:Protocol Name="${xml(p.scheme)}" Parameters="--sk-protocol-launch -- &quot;%1&quot;">`,
      `            <uap:DisplayName>${xml(p.name)}</uap:DisplayName>`,
      "          </uap3:Protocol>",
      "        </uap3:Extension>",
    );
  }
  const taken = new Set<string>();
  config.fileAssociations.forEach((f, i) => {
    const logo = fileTypeLogos[i];
    extensions.push(
      '        <uap3:Extension Category="windows.fileTypeAssociation">',
      `          <uap3:FileTypeAssociation Name="${fileTypeName(f.name, i, taken)}" Parameters="&quot;%1&quot;">`,
      `            <uap:DisplayName>${xml(f.name)}</uap:DisplayName>`,
      ...(logo ? [`            <uap:Logo>${xml(logo)}</uap:Logo>`] : []),
      "            <uap:SupportedFileTypes>",
      ...f.extensions.map(
        (e) =>
          `              <uap:FileType${f.mimeType ? ` ContentType="${xml(f.mimeType)}"` : ""}>.${xml(e)}</uap:FileType>`,
      ),
      "            </uap:SupportedFileTypes>",
      "          </uap3:FileTypeAssociation>",
      "        </uap3:Extension>",
    );
  });
  const caps = (kind: string) => o.capabilities.filter((c) => CAPABILITIES[c] === kind);
  const lines = [
    '<?xml version="1.0" encoding="utf-8"?>',
    '<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"',
    '  xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"',
    '  xmlns:uap3="http://schemas.microsoft.com/appx/manifest/uap/windows10/3"',
    '  xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"',
    '  IgnorableNamespaces="uap uap3 rescap">',
    `  <Identity Name="${xml(o.identityName)}" Publisher="${xml(o.publisher)}" Version="${msixVersion(id.version)}" ProcessorArchitecture="${o.architecture}"/>`,
    "  <Properties>",
    `    <DisplayName>${xml(id.displayName)}</DisplayName>`,
    `    <PublisherDisplayName>${xml(publisherDisplayName(config))}</PublisherDisplayName>`,
    `    <Logo>${xml(assets.storeLogo)}</Logo>`,
    `    <Description>${xml(id.description)}</Description>`,
    "  </Properties>",
    "  <Dependencies>",
    `    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="${o.minVersion}" MaxVersionTested="${o.maxVersionTested}"/>`,
    "  </Dependencies>",
    "  <Resources>",
    '    <Resource Language="en-us"/>',
    "  </Resources>",
    "  <Applications>",
    `    <Application Id="App" Executable="${xml(executable)}" EntryPoint="Windows.FullTrustApplication">`,
    `      <uap:VisualElements DisplayName="${xml(id.displayName)}" Description="${xml(id.description)}" BackgroundColor="${o.background}" Square150x150Logo="${xml(assets.square150)}" Square44x44Logo="${xml(assets.square44)}"${assets.wide310 ? ">" : "/>"}`,
    ...(assets.wide310
      ? [`        <uap:DefaultTile Wide310x150Logo="${xml(assets.wide310)}"/>`, "      </uap:VisualElements>"]
      : []),
    ...(extensions.length ? ["      <Extensions>", ...extensions, "      </Extensions>"] : []),
    "    </Application>",
    "  </Applications>",
    "  <Capabilities>",
    ...caps("foundation").map((c) => `    <Capability Name="${c}"/>`),
    ...caps("uap").map((c) => `    <uap:Capability Name="${c}"/>`),
    '    <rescap:Capability Name="runFullTrust"/>',
    ...caps("rescap").map((c) => `    <rescap:Capability Name="${c}"/>`),
    ...caps("device").map((c) => `    <DeviceCapability Name="${c}"/>`),
    "  </Capabilities>",
    "</Package>",
    "",
  ];
  return lines.join("\r\n");
}

export function appInstallerXml(config: PackageConfig, packageFile: string, feedFile: string): string | null {
  const o = config.windows.msix;
  const ai = o.appInstaller;
  if (!ai) return null;
  const version = msixVersion(config.identity.version);
  return [
    '<?xml version="1.0" encoding="utf-8"?>',
    `<AppInstaller xmlns="http://schemas.microsoft.com/appx/appinstaller/2018" Version="${version}" Uri="${xml(`${ai.url}/${encodeURIComponent(feedFile)}`)}">`,
    `  <MainPackage Name="${xml(o.identityName)}" Publisher="${xml(o.publisher)}" Version="${version}" ProcessorArchitecture="${o.architecture}" Uri="${xml(`${ai.url}/${encodeURIComponent(packageFile)}`)}"/>`,
    "  <UpdateSettings>",
    `    <OnLaunch HoursBetweenUpdateChecks="${ai.hoursBetweenUpdateChecks}" ShowPrompt="${ai.showPrompt}" UpdateBlocksActivation="${ai.updateBlocksActivation}"/>`,
    "    <AutomaticBackgroundTask/>",
    "  </UpdateSettings>",
    "</AppInstaller>",
    "",
  ].join("\r\n");
}

/** Every file under `dir`, `/` separated, sorted; dot-folders (`.sk`) and debug symbols stay out. */
export function payloadFiles(dir: string): string[] {
  const out: string[] = [];
  const walk = (abs: string) => {
    for (const entry of readdirSync(abs, { withFileTypes: true })) {
      if (entry.name.startsWith(".")) continue;
      const path = join(abs, entry.name);
      if (entry.isDirectory()) walk(path);
      else if (entry.isFile() && !/\.pdb$/i.test(entry.name)) out.push(relative(dir, path).split(sep).join("/"));
    }
  };
  walk(dir);
  return out.sort();
}

const ASSETS = {
  storeLogo: ["PackageAssets/StoreLogo.png", 100],
  square150: ["PackageAssets/Square150x150Logo.png", 300],
  square44: ["PackageAssets/Square44x44Logo.png", 88],
} as const;

async function write(ctx: PackageContext): Promise<Artifact[]> {
  const { config } = ctx;
  const o = config.windows.msix;
  const exeRel = relative(ctx.buildDir, ctx.exe).split(sep).join("/");
  if (exeRel.startsWith("..")) throw new Error(`${ctx.exe} is not inside ${ctx.buildDir}`);
  const payload = payloadFiles(ctx.buildDir);
  const clash = payload.find(reservedPath);
  if (clash)
    throw new Error(
      `${join(ctx.buildDir, clash)}: the name is reserved for the package's own files; rename or remove it`,
    );
  if (payload.some((p) => p.toLowerCase().startsWith("packageassets/"))) {
    throw new Error(
      `${join(ctx.buildDir, "PackageAssets")}: the name is reserved for the package's tile images; rename the folder`,
    );
  }

  const generated = new Map<string, Uint8Array>();
  const sources = iconSources(config.icons);
  const asset = (path: string, png: Uint8Array | null) => {
    if (png) generated.set(path, png);
  };
  for (const [path, size] of Object.values(ASSETS)) asset(path, iconAt(sources, size) ?? blankPng(size));
  const wide = sources.length ? "PackageAssets/Wide310x150Logo.png" : null;
  if (wide) asset(wide, iconOnCanvas(sources, 240, 620, 300));
  const fileTypeLogos = config.fileAssociations.map((f, i) => {
    if (!f.icon) return null;
    const path = `PackageAssets/FileType${i}.png`;
    asset(path, iconAt(iconSources([f.icon]), 96));
    return path.replace(/\//g, "\\");
  });

  const win = (p: string) => p.replace(/\//g, "\\");
  const manifest = appxManifest({
    config,
    executable: win(exeRel),
    assets: {
      storeLogo: win(ASSETS.storeLogo[0]),
      square150: win(ASSETS.square150[0]),
      square44: win(ASSETS.square44[0]),
      wide310: wide && win(wide),
    },
    fileTypeLogos,
  });
  const version = msixVersion(config.identity.version);
  const fileName = `${o.identityName}_${version}_${o.architecture}.msix`;
  const out = join(ctx.outDir, fileName);
  const files = [...payload, ...generated.keys()].sort();
  ctx.log.info(`Publisher: ${o.publisher}`);
  const { bytes } = writeAppx(
    out,
    files,
    (p) => generated.get(p) ?? new Uint8Array(readFileSync(join(ctx.buildDir, p))),
    manifest,
  );
  writeFileSync(join(ctx.workDir, "AppxManifest.xml"), manifest);
  ctx.log.info(`${fileName}: ${files.length} files, ${(bytes / 2 ** 20).toFixed(1)} MiB`);

  let signed = false;
  if (ctx.sign) {
    await ctx.sign(out, "msix");
    signed = true;
  } else {
    ctx.log.warn(
      `${fileName} is unsigned and Windows installs only signed packages: sign it with signtool sign /fd SHA256 and a certificate whose subject is the Publisher above`,
    );
  }
  const artifacts: Artifact[] = [{ path: out, kind: "installer", format: "msix", signed }];
  const feedFile = `${o.identityName}.appinstaller`;
  const feed = appInstallerXml(config, fileName, feedFile);
  if (feed) {
    const feedPath = join(ctx.outDir, feedFile);
    writeFileSync(feedPath, feed);
    artifacts.push({ path: feedPath, kind: "update", format: "msix", signed: false });
  }
  return artifacts;
}

function blankPng(size: number): Uint8Array {
  return iconAt([{ width: 1, height: 1, data: new Uint8Array(4) }], size) ?? new Uint8Array(0);
}

export const format: PackageFormat<"msix"> = {
  name: "msix",
  platform: "windows",
  hosts: ["windows", "macos", "linux"],
  optionKeys: MSIX_KEYS,
  parseOptions,
  validate,
  write,
};
