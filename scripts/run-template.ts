import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";
import { create, parseTemplate, prepareTemplate } from "../packages/cli/cli/create";
import { main } from "../packages/cli/cli/sk";

const [name, ...args] = process.argv.slice(2);
const template = parseTemplate(name ?? "");
await prepareTemplate(template);

// Scaffold the same assets and config as sk create, within reach of the workspace dependencies.
const parent = resolve(import.meta.dir, "../target/templates");
mkdirSync(parent, { recursive: true });
const app = mkdtempSync(join(parent, template + "-"));
process.once("exit", () => rmSync(app, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 }));

create(app, { template, name: template[0]!.toUpperCase() + template.slice(1) });
process.exitCode = await main(["dev", app, ...args]);
