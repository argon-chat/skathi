import { mkdirSync, writeFileSync, existsSync } from "fs";
import { dirname, join, relative } from "path";

const REF = "v2.2.0";
const STYLE = "default";
const RAW = `https://raw.githubusercontent.com/unovue/shadcn-vue/${REF}/apps/www/src`;
const UI = `${RAW}/registry/${STYLE}/ui`;

const APP = new URL("..", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");
const OUT = join(APP, "src", "components", "ui");

const COMPONENTS: Record<string, string[]> = {
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

const EXTRAS: Record<string, string> = {
  "src/lib/utils.ts": `${RAW}/lib/utils.ts`,
};

const force = process.argv.includes("--force");
const edits: { file: string; from: string; to: string }[] = [];

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
