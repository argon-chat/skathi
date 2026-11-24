// What the docs add to packages/runtime/sk.config.schema.json: grouping, computed defaults, platform notes and
// example values. Types, constraints, allowed values and descriptions come from the schema itself. Keys are option
// paths: "window.width", "packs[].name". The window keys that have a Window API entry also take their default,
// platform support, value notes and limitations from it (docs/src/data/window/entries.ts).

export interface ConfigGroup {
  title: string;
  intro: string;
  keys: readonly string[];
}

export interface ConfigNote {
  /** Markdown, for a default the schema cannot state as a value. */
  default?: string;
  /** Notes on enum values, by value. */
  values?: Readonly<Record<string, string>>;
  platforms?: string;
  notes?: readonly string[];
  /** The key's value in its example; checked against the schema at build time. */
  example?: unknown;
  /** The release the key arrived in, for a key without a Window API entry to say so. */
  since?: string;
}

export const groups: readonly ConfigGroup[] = [
  {
    title: "Editor support",
    intro: "`sk create` writes the `$schema` line, so editors complete and check every key below.",
    keys: ["$schema"],
  },
  {
    title: "Identity",
    intro:
      "Who the app is: the name its data is stored under, the name people see, its version and its launch identity.",
    keys: ["name", "productName", "version", "id"],
  },
  {
    title: "Main window",
    intro:
      "Read by the host once, before the main window opens; a change needs a restart. Each key also has a page in the [Window API](/window-api/#startup-configuration) with its run-time counterpart.",
    keys: ["window"],
  },
  {
    title: "Runtime",
    intro: "Settings the host reads at start. Each also works inside `window`, where it wins over the top-level key.",
    keys: ["devtools", "httpCache", "colorPicker"],
  },
  {
    title: "Text",
    intro: "How the host draws text, for the whole app. Read once at start; a change needs a restart.",
    keys: ["textRendering"],
  },
  {
    title: "Launch",
    intro: "How a second launch and a link to the app's URL scheme reach the running app.",
    keys: ["singleInstance", "raiseOnSecondInstance", "protocols"],
  },
  {
    title: "Build and packaging",
    intro:
      "Read by `sk build`, `sk publish` and `sk check`: what goes into the package, how the bundle is built, and for which OSes.",
    keys: [
      "platforms",
      "assets",
      "publicDir",
      "packs",
      "define",
      "aliases",
      "aot",
      "intl",
      "publishDevtools",
      "publishJit",
      "publishSingleFile",
      "check",
    ],
  },
  {
    title: "Windows exe",
    intro: "Stamped into the Windows exe by `sk build`. Read on every OS, so a typo fails the build everywhere.",
    keys: ["windows"],
  },
  {
    title: "macOS bundle",
    intro: "Written into the `.app` bundle's `Info.plist`. Read on every OS, so a typo fails the build everywhere.",
    keys: ["macos"],
  },
];

const BUILD_TIME = "Stamped at build time; it cannot change while the app runs.";

export const notes: Readonly<Record<string, ConfigNote>> = {
  $schema: { example: "./node_modules/@skathi-native/runtime/sk.config.schema.json" },
  name: {
    default: "the app directory's name",
    platforms:
      "The storage directory is `%LOCALAPPDATA%\\<name>\\storage` on Windows, `~/Library/Application Support/<name>/storage` on macOS and `$XDG_DATA_HOME/<name>/storage` (`~/.local/share`) on Linux.",
    notes: [
      "Renaming the exe does not move the app's data: the packaged app keeps the `name` it was built with.",
      "`SK_APP_NAME` in the environment overrides it, so two dev instances can keep separate storage.",
    ],
    example: "notes",
  },
  productName: {
    default: "`window.title`, else `name`",
    notes: ["It is the default of `windows.exeName` and `macos.exeName`."],
    example: "Notes",
  },
  version: {
    default: '`package.json` `"version"`, else `0.0.0` with a warning',
    notes: ["A leading `v` is accepted. The numeric parts become the exe's file version; a missing part is 0."],
    example: "1.4.0",
  },
  id: {
    default: "`name`",
    notes: ['It is also the default `macos.bundleId`, with `"_"` written as `"-"`.'],
    example: "com.example.notes",
  },
  window: {
    notes: [
      "A key the host does not know is ignored without a warning, and a value of the wrong type keeps the default.",
      "`SK_WINDOW=1280x800` in the environment overrides `width` and `height`.",
    ],
    example: { title: "Notes", width: 960, height: 640, minWidth: 480, minHeight: 320, backdrop: "mica" },
  },
  "window.title": { example: "Notes" },
  "window.width": { example: 1280 },
  "window.height": { example: 800 },
  "window.minWidth": { example: 480 },
  "window.minHeight": { example: 320 },
  "window.x": { example: 24 },
  "window.y": { example: 24 },
  "window.center": { example: true },
  "window.monitor": { example: "cursor" },
  "window.frame": { example: false },
  "window.titleBarStyle": {
    default: '`"hidden"`',
    example: "default",
  },
  "window.titleBarHeight": { example: 44 },
  "window.titleBarFallback": { example: false },
  "window.titleBarOverlay": {
    default: "`false`",
    example: { height: 40, color: "#00000000", symbolColor: "#1f1f1f" },
  },
  "window.titleBarOverlay.height": { example: 40 },
  "window.titleBarOverlay.color": { example: "#00000000" },
  "window.titleBarOverlay.symbolColor": { example: "#1f1f1f" },
  "window.backdrop": {
    default: '`"mica"`',
    values: {
      tabbed: 'A synonym of `"mica-alt"`.',
      blur: 'A synonym of `"acrylic"`, with an advisory.',
      opaque: 'A synonym of `"none"`.',
    },
    example: "acrylic",
  },
  "window.resizable": { example: false },
  "window.minimizable": { default: "`true`", example: false },
  "window.maximizable": { default: "`true`", example: false },
  "window.alwaysOnTop": { example: true },
  "window.focus": { example: "raise" },
  "window.devtools": { default: "the top-level `devtools`, else `true`", example: false },
  "window.httpCache": { default: "the top-level `httpCache`", example: { maxMB: 64 } },
  "window.httpCache.maxMB": { example: 64 },
  "window.colorPicker": { default: "the top-level `colorPicker`", example: "system", since: "0.5.0" },
  devtools: {
    default: "`true`",
    notes: [
      "`false` starts no devtools listener, writes no handshake file and declines F12; `appDevtools.enable()` starts them later.",
      "`SK_DEVTOOLS=1` in the environment forces them on and locks them; `SK_DEVTOOLS=0` only changes the start state.",
      "A published app has devtools only with `publishDevtools`.",
    ],
    example: false,
  },
  httpCache: {
    default: '`{ "maxMB": 256 }`',
    notes: ["`SK_HTTP_CACHE` in the environment wins: `0` or `off` turns the cache off, a number is the cap in MB."],
    example: { maxMB: 64 },
  },
  "httpCache.maxMB": { example: 64 },
  colorPicker: {
    default: '`"builtin"`',
    notes: [
      "An app can also show its own picker per input by cancelling the input's `showpicker` event ([Form controls](/elements/form-controls/#show-your-own-picker)).",
      "With `\"system\"` the input's `alpha`, `colorspace` and `hdr` do not reach the OS dialog; its sRGB pick is converted to the input's space.",
    ],
    example: "system",
    since: "0.5.0",
  },
  textRendering: {
    default: '`"skathi-mode"` on Windows and Linux, `"optimized"` on macOS',
    values: {
      "skathi-mode":
        "Text looks as it does on a Mac at every size and scale. `font-smoothing: antialiased` turns the emboldening off; `subpixel-antialiased` and `font-dilation` change nothing.",
      optimized:
        "ClearType and the font's hinting on Windows: sharper stems at 100 % scale, the look of other Windows apps.",
    },
    platforms:
      'macOS: `"skathi-mode"` draws the same as `"optimized"`, with CoreText. Linux: `"optimized"` is swash\'s own rasterizer.',
    notes: [
      "`SK_TEXT_RENDERING=optimized` (or `skathi-mode`) in the environment wins over the config and over `sk_app::embed::Builder::text_rendering`.",
      "[Text rendering](/guides/text-rendering/#two-rendering-modes) compares the two modes.",
    ],
    example: "optimized",
  },
  singleInstance: { example: true },
  raiseOnSecondInstance: { example: false },
  protocols: {
    platforms:
      "Windows: the packaged exe registers each scheme for the current user at start (an MSIX-packaged app declares them in its AppxManifest instead). macOS: written into the bundle's `CFBundleURLTypes`. Linux: not registered yet. A dev host never registers them.",
    notes: ["Schemes are lowercased, and a trailing `:` or `://` is dropped."],
    example: ["notes"],
  },
  platforms: {
    default: "the OS `sk publish` runs on",
    example: ["windows", "macos"],
  },
  assets: {
    default: '`["assets"]`',
    notes: [
      "In a packaged app, `fetch()` and `<img>` resolve relative URLs from the package only; a file left out is a 404 with an advisory.",
    ],
    example: ["assets", "fonts/**/*.woff2"],
  },
  publicDir: {
    default: "`false`",
    notes: ["A path that is not a directory is a warning, and nothing is packaged from it."],
    example: "public",
  },
  packs: {
    notes: ["Pack names must be unique."],
    example: [{ name: "fonts", match: ["**/*.woff2"] }, { name: "media" }],
  },
  "packs[].name": { example: "fonts" },
  "packs[].match": { example: ["**/*.woff2"] },
  define: {
    notes: [
      'As in Vite, a string value is inserted as code, so a string literal carries its own quotes: `"\\"1.4.0\\""`. Other values are inserted as JSON.',
      "`import.meta.env.*` keys also show up on `import.meta.env`. `--define KEY=VALUE` on the command line adds more.",
    ],
    example: { __APP_FLAVOR__: '"beta"', "import.meta.env.API_URL": '"https://api.example.com"' },
  },
  aliases: {
    notes: [
      "The name must be a bare module name, and the file must exist. The engine's own modules cannot be aliased.",
    ],
    example: { "native-sensors": "src/native/sensors.ts" },
  },
  aot: {
    platforms: "Windows x64 only: building for another OS, or on another machine, needs `--no-aot`.",
    example: true,
  },
  intl: {
    platforms: "Windows and Linux. macOS apps use the system's ICU and always have every locale.",
    notes: [
      "`sk dev` always has every locale.",
      'With `"full"`, `icu.skpk` ships beside the exe and the app reads it at start. Without it, the app runs as if `"invariant"` and says so on stderr.',
      "[Best practices](/guides/best-practices/#choose-how-much-locale-data-to-ship) shows what changes in invariant mode.",
    ],
    example: "invariant",
    since: "0.4.0",
  },
  publishDevtools: { example: true },
  publishJit: {
    platforms: "Windows x64, with `aot`.",
    notes: [
      "Read by `sk publish --aot`; debug builds and builds without AOT keep the JIT. `false` with `publishDevtools` is an error.",
      "The workload has no WebAssembly either: `sk publish` refuses an app whose image names `WebAssembly`, and lists where.",
      "[AOT compilation](/guides/aot/#ship-without-the-jit) gives the size and speed measured.",
    ],
    example: false,
    since: "0.6.0",
  },
  publishSingleFile: {
    platforms: "Windows and Linux. A macOS app is a bundle: `sk publish` refuses the option for macOS.",
    notes: [
      "Read by `sk publish` only; `sk build` keeps writing the files beside the exe.",
      "Without it, `sk publish` writes beside the exe `icu.skpk`, the `packs`, and with AOT `app.image.dll` (and `inspector.image.dll` when devtools are in). With it, all of them go inside the exe and nothing is written beside it.",
      "The native images stay uncompressed in the exe, so the exe grows by their size; nothing is expanded at start. [AOT compilation](/guides/aot/#one-file-publishsinglefile) has the measurements.",
    ],
    example: true,
    since: "0.6.0",
  },
  check: { example: { allow: { SK2001: ["some-package", "src/legacy/**"] } } },
  "check.allow": {
    notes: ["An unknown code stops `sk check` with exit code 2. `sk explain --list` lists the codes."],
    example: { SK2001: ["some-package", "src/legacy/**"] },
  },
  windows: {
    platforms: "Windows. `icon` and `copyright` also serve macOS when `macos` does not set them.",
    example: { exeName: "Notes", icon: "assets/icon.png", company: "Example Ltd", console: false },
  },
  "windows.exeName": {
    default: "`productName`",
    notes: ["Characters Windows does not allow in a file name are dropped."],
    example: "Notes",
  },
  "windows.icon": {
    notes: [BUILD_TIME, "Without it, the build warns and the exe gets the default Windows icon."],
    example: "assets/icon.png",
  },
  "windows.company": { example: "Example Ltd" },
  "windows.copyright": { example: "© 2026 Example Ltd" },
  "windows.description": { example: "Notes for teams" },
  "windows.console": { notes: ["`--console` on the command line does the same."], example: true },
  macos: {
    platforms: "macOS.",
    example: { bundleId: "com.example.notes", category: "productivity", minimumSystemVersion: "14.0" },
  },
  "macos.bundleId": { default: '`id`, with `"_"` as `"-"`', example: "com.example.notes" },
  "macos.exeName": { default: "`productName`", example: "Notes" },
  "macos.icon": { default: "`windows.icon`", example: "assets/icon-mac.png" },
  "macos.category": {
    notes: ['A short name gets the `"public.app-category."` prefix.'],
    example: "productivity",
  },
  "macos.minimumSystemVersion": {
    notes: ["A version below 13.0 is a warning: the app would install on that macOS and then fail to load."],
    example: "14.0",
  },
  "macos.copyright": { default: "`windows.copyright`", example: "© 2026 Example Ltd" },
  "macos.microphoneUsageDescription": {
    notes: [
      "Without it, a microphone request is denied without a prompt. With it, the bundle also gets the audio input entitlement.",
      "`sk dev` runs the app from a dev `.app` with these descriptions and the bundle id `<bundleId>.dev`, so macOS asks there too. `sk create` writes them for the showcase.",
    ],
    example: "Notes records voice memos.",
  },
  "macos.cameraUsageDescription": {
    notes: [
      "Without it, a camera request is denied without a prompt. With it, the bundle also gets the camera entitlement.",
      "`sk dev` runs the app from a dev `.app` with these descriptions, so macOS asks there too.",
    ],
    example: "Notes scans documents with the camera.",
  },
  "macos.sign": { default: "ad hoc where `codesign` exists", example: "Developer ID Application: Example Ltd" },
};
