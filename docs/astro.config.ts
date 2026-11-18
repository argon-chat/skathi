import mdx from "@astrojs/mdx";
import sitemap from "@astrojs/sitemap";
import vue from "@astrojs/vue";
import { satteri } from "@astrojs/markdown-satteri";
import { defineConfig, fontProviders } from "astro/config";
import { codeTransformers } from "./src/lib/code";
import docsContent, { sectionSince } from "./src/lib/docs-content";
import slang from "./src/lib/slang";

const site = process.env["SITE_URL"] ?? "https://skathi.land";

export default defineConfig({
  site,
  output: "static",
  trailingSlash: "always",
  integrations: [vue(), mdx(), sitemap({ filter: (page) => !page.endsWith("/404/") })],
  fonts: [
    {
      name: "DM Sans Variable",
      cssVariable: "--font-body",
      provider: fontProviders.local(),
      weights: ["100 1000"],
      styles: ["normal"],
      fallbacks: ["Arial", "sans-serif"],
      options: {
        variants: [
          {
            src: ["@fontsource-variable/dm-sans/files/dm-sans-latin-wght-normal.woff2"],
            weight: "100 1000",
            style: "normal",
          },
        ],
      },
    },
    {
      name: "Lora Variable",
      cssVariable: "--font-heading",
      provider: fontProviders.local(),
      weights: ["400 700"],
      styles: ["normal"],
      fallbacks: ["Georgia", "serif"],
      options: {
        variants: [
          { src: ["@fontsource-variable/lora/files/lora-latin-wght-normal.woff2"], weight: "400 700", style: "normal" },
        ],
      },
    },
    {
      name: "JetBrains Mono Variable",
      cssVariable: "--font-code",
      provider: fontProviders.local(),
      weights: ["100 800"],
      styles: ["normal"],
      fallbacks: ["Courier New", "monospace"],
      options: {
        variants: [
          {
            src: ["@fontsource-variable/jetbrains-mono/files/jetbrains-mono-latin-wght-normal.woff2"],
            weight: "100 800",
            style: "normal",
          },
        ],
      },
    },
  ],
  markdown: {
    processor: satteri({ mdastPlugins: [docsContent], hastPlugins: [sectionSince] }),
    shikiConfig: {
      themes: { light: "github-light", dark: "github-dark" },
      defaultColor: false,
      langs: [slang],
      transformers: codeTransformers,
    },
  },
  vite: { resolve: { dedupe: ["vue"] } },
});
