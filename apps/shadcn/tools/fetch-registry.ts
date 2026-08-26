// Pulls shadcn-vue component sources straight out of the upstream repo.
//
//   bun tools/fetch-registry.ts [--force]
//
// WHY NOT `npx shadcn-vue add`: the CLI assumes a vite/nuxt project (it reads
// components.json, patches tailwind.config, rewrites tsconfig paths and runs a
// package-manager install). Copying the registry files verbatim is the same
// bytes with none of the assumptions — and it is what makes the compat claim
// checkable: `git diff` against this script's output shows EXACTLY what we
// changed, which for the trial must be import-path rewrites and nothing else.
//
// PIN: v2.2.0 is the LAST shadcn-vue release on Tailwind v3 (apps/www pins
// tailwindcss ^3.4.17; from v2.3 the docs app moved to the Tailwind v4 tree in
// apps/v4). vue-native pins Tailwind 3.4 for cascade reasons — see
// packages/vue-native/tailwind.ts — so v2.2.0 is the honest pairing. The
// components themselves are the reka-ui generation, i.e. current shadcn-vue.
//
// THE ONLY EDITS (both mechanical, both logged to tools/registry-edits.json):
//   1. `@/lib/utils`  -> a relative path to src/lib/utils.js
//   2. `@/registry/...` (blocks/examples only) -> relative
// Everything else — templates, class strings, reka-ui usage — is byte-identical
// to upstream. Any further change would invalidate the compat measurement.

import { mkdirSync, writeFileSync, existsSync } from "fs";
import { dirname, join, relative } from "path";

const REF = "v2.2.0";
const STYLE = "default";
const RAW = `https://raw.githubusercontent.com/unovue/shadcn-vue/${REF}/apps/www/src`;
const UI = `${RAW}/registry/${STYLE}/ui`;

const APP = new URL("..", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");
const OUT = join(APP, "src", "components", "ui");

/** component dir -> files, as listed by the upstream git tree at REF. */
const COMPONENTS: Record<string, string[]> = {
  // tier 1 — static / low-DOM
  button: ["Button.vue", "index.ts"],
  badge: ["Badge.vue", "index.ts"],
  card: [
    "Card.vue", "CardContent.vue", "CardDescription.vue",
    "CardFooter.vue", "CardHeader.vue", "CardTitle.vue", "index.ts",
  ],
  alert: ["Alert.vue", "AlertDescription.vue", "AlertTitle.vue", "index.ts"],
  separator: ["Separator.vue", "index.ts"],
  skeleton: ["Skeleton.vue", "index.ts"],
  label: ["Label.vue", "index.ts"],
  progress: ["Progress.vue", "index.ts"],

  // tier 2 — form + state
  input: ["Input.vue", "index.ts"],
  textarea: ["Textarea.vue", "index.ts"],
  checkbox: ["Checkbox.vue", "index.ts"],
  switch: ["Switch.vue", "index.ts"],
  toggle: ["Toggle.vue", "index.ts"],
  "radio-group": ["RadioGroup.vue", "RadioGroupItem.vue", "index.ts"],
  tabs: ["Tabs.vue", "TabsContent.vue", "TabsList.vue", "TabsTrigger.vue", "index.ts"],
  accordion: [
    "Accordion.vue", "AccordionContent.vue", "AccordionItem.vue",
    "AccordionTrigger.vue", "index.ts",
  ],

  // tier 3 — portal / floating
  dialog: [
    "Dialog.vue", "DialogClose.vue", "DialogContent.vue", "DialogDescription.vue",
    "DialogFooter.vue", "DialogHeader.vue", "DialogScrollContent.vue",
    "DialogTitle.vue", "DialogTrigger.vue", "index.ts",
  ],
  tooltip: ["Tooltip.vue", "TooltipContent.vue", "TooltipProvider.vue", "TooltipTrigger.vue", "index.ts"],
  popover: ["Popover.vue", "PopoverContent.vue", "PopoverTrigger.vue", "index.ts"],
  select: [
    "Select.vue", "SelectContent.vue", "SelectGroup.vue", "SelectItem.vue",
    "SelectItemText.vue", "SelectLabel.vue", "SelectScrollDownButton.vue",
    "SelectScrollUpButton.vue", "SelectSeparator.vue", "SelectTrigger.vue",
    "SelectValue.vue", "index.ts",
  ],
  "dropdown-menu": [
    "DropdownMenu.vue", "DropdownMenuCheckboxItem.vue", "DropdownMenuContent.vue",
    "DropdownMenuGroup.vue", "DropdownMenuItem.vue", "DropdownMenuLabel.vue",
    "DropdownMenuRadioGroup.vue", "DropdownMenuRadioItem.vue",
    "DropdownMenuSeparator.vue", "DropdownMenuShortcut.vue", "DropdownMenuSub.vue",
    "DropdownMenuSubContent.vue", "DropdownMenuSubTrigger.vue",
    "DropdownMenuTrigger.vue", "index.ts",
  ],
};

/** Non-`ui/` files the components import, fetched verbatim too. */
const EXTRAS: Record<string, string> = {
  // `cn()` — clsx + tailwind-merge. Kept byte-identical (its `@tanstack/vue-table`
  // and `vue` imports are `import type`, which the bundler erases).
  "src/lib/utils.ts": `${RAW}/lib/utils.ts`,
};

const force = process.argv.includes("--force");
const edits: { file: string; from: string; to: string }[] = [];

/** `@/lib/utils` -> relative path from the file to src/lib/utils. */
function rewrite(fileAbs: string, text: string): string {
  const fromDir = dirname(fileAbs);
  return text.replace(/(['"])@\/([^'"]+)\1/g, (_m, q, spec: string) => {
    const targetAbs = join(APP, "src", spec.replace(/^registry\/[^/]+\//, "components/"));
    let rel = relative(fromDir, targetAbs).split("\\").join("/");
    if (!rel.startsWith(".")) rel = "./" + rel;
    edits.push({ file: relative(APP, fileAbs).split("\\").join("/"), from: `@/${spec}`, to: rel });
    return `${q}${rel}${q}`;
  });
}

async function get(url: string): Promise<string> {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`${r.status} ${r.statusText} for ${url}`);
  return r.text();
}

let written = 0;
let skipped = 0;

for (const [dir, files] of Object.entries(COMPONENTS)) {
  for (const name of files) {
    const abs = join(OUT, dir, name);
    if (existsSync(abs) && !force) {
      skipped++;
      continue;
    }
    const text = rewrite(abs, await get(`${UI}/${dir}/${name}`));
    mkdirSync(dirname(abs), { recursive: true });
    writeFileSync(abs, text);
    written++;
  }
}

for (const [rel, url] of Object.entries(EXTRAS)) {
  const abs = join(APP, rel);
  if (existsSync(abs) && !force) {
    skipped++;
    continue;
  }
  mkdirSync(dirname(abs), { recursive: true });
  writeFileSync(abs, rewrite(abs, await get(url)));
  written++;
}

writeFileSync(
  join(APP, "tools", "registry-edits.json"),
  JSON.stringify(
    { ref: REF, style: STYLE, source: `${UI}`, componentCount: Object.keys(COMPONENTS).length, edits },
    null,
    2,
  ) + "\n",
);

console.log(
  `shadcn-vue ${REF} (${STYLE}): ${written} file(s) written, ${skipped} kept` +
    ` — ${edits.length} import-path rewrites (see tools/registry-edits.json)`,
);
