import { launch } from "../src/index";

const keepAlive = process.argv.includes("keep");
const app = await launch({ app: "hello", window: [640, 480], label: "orphan", keepAlive });
console.log(`RUNNER ${process.pid} HOST ${app.pid} SLOT ${app.slotDir}`);
await new Promise(() => {});
