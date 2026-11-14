// A RELATIVE content glob, as every Vite app writes it: Tailwind v3 resolves it
// against process.cwd(), and prod.ts runs from packages/cli — so the
// utilities below only appear if tailwind.ts moves to this directory first.
export default {
  content: ["./src/**/*.{vue,ts}"],
  corePlugins: { preflight: false },
};
