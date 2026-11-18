import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { initWasm, Resvg } from "@resvg/resvg-wasm";
import satori from "satori";
import wolf from "../../../logos/logo_single.png?inline";

const require = createRequire(import.meta.url);
const read = (specifier: string) => readFile(require.resolve(specifier));

let ready: Promise<{ fonts: Parameters<typeof satori>[1]["fonts"]; wolf: string }> | undefined;

function load() {
  ready ??= (async () => {
    await initWasm(read("@resvg/resvg-wasm/index_bg.wasm"));
    const [lora, sans, sansBold, mono] = await Promise.all([
      read("@fontsource/lora/files/lora-latin-600-normal.woff"),
      read("@fontsource/dm-sans/files/dm-sans-latin-400-normal.woff"),
      read("@fontsource/dm-sans/files/dm-sans-latin-600-normal.woff"),
      read("@fontsource/jetbrains-mono/files/jetbrains-mono-latin-400-normal.woff"),
    ]);
    return {
      fonts: [
        { name: "Lora", data: lora, weight: 600, style: "normal" },
        { name: "DM Sans", data: sans, weight: 400, style: "normal" },
        { name: "DM Sans", data: sansBold, weight: 600, style: "normal" },
        { name: "JetBrains Mono", data: mono, weight: 400, style: "normal" },
      ],
      wolf,
    };
  })();
  return ready;
}

type Node = { type: string; props: { style?: Record<string, unknown>; children?: unknown; [key: string]: unknown } };
const h = (
  type: string,
  style: Record<string, unknown>,
  children?: unknown,
  props: Record<string, unknown> = {},
): Node => ({
  type,
  props: { ...props, style, children },
});

export interface Card {
  kicker: string;
  title: string;
  description: string;
  label: string;
  meta: string;
}

/** A 1200×630 link-preview card in the site's dark palette, as PNG bytes. */
export async function renderCard(card: Card): Promise<Uint8Array> {
  const { fonts, wolf } = await load();
  const glow = (color: string, place: Record<string, unknown>) =>
    h("div", {
      position: "absolute",
      width: 640,
      height: 520,
      backgroundImage: `radial-gradient(closest-side, ${color}, transparent)`,
      opacity: 0.22,
      ...place,
    });
  const tree = h(
    "div",
    {
      position: "relative",
      display: "flex",
      flexDirection: "column",
      // Satori sizes the content box: 1200×630 less the padding.
      width: 1200 - 72 * 2,
      height: 630 - 56 - 48,
      padding: "56px 72px 48px",
      backgroundColor: "#17161e",
      color: "#edeaf3",
      fontFamily: "DM Sans",
    },
    [
      glow("#f7a8d0", { left: -220, bottom: -260 }),
      glow("#9fcdf5", { right: -200, top: -240 }),
      h("img", { position: "absolute", right: -90, top: -40, width: 420, height: 420, opacity: 0.12 }, undefined, {
        src: wolf,
        width: 420,
        height: 420,
      }),
      h("div", { display: "flex", alignItems: "center", gap: 14, fontSize: 26, fontWeight: 600 }, [
        h("img", { width: 44, height: 44 }, undefined, { src: wolf, width: 44, height: 44 }),
        card.kicker,
      ]),
      h("div", { display: "flex", flexDirection: "column", marginTop: "auto", maxWidth: 1056 }, [
        h("div", { fontFamily: "Lora", fontSize: 60, fontWeight: 600, lineHeight: 1.12, lineClamp: 2 }, card.title),
        h("div", { marginTop: 22, color: "#a09aad", fontSize: 27, lineHeight: 1.45, lineClamp: 2 }, card.description),
      ]),
      h(
        "div",
        {
          display: "flex",
          justifyContent: "space-between",
          marginTop: 40,
          paddingTop: 22,
          borderTop: "1px solid #302d3b",
          color: "#a09aad",
          fontFamily: "JetBrains Mono",
          fontSize: 21,
        },
        [h("div", { color: "#c9a7f5", textTransform: "uppercase", letterSpacing: 0.8 }, card.label), card.meta],
      ),
    ],
  );
  const svg = await satori(tree as Parameters<typeof satori>[0], { width: 1200, height: 630, fonts });
  return new Resvg(svg).render().asPng();
}
