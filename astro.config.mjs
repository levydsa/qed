// @ts-check
import { defineConfig } from "astro/config";

import mdx from "@astrojs/mdx";
import rehypeKatex from "rehype-katex";
import remarkMath from "remark-math";

import tailwind from "@astrojs/tailwind";
import react from "@astrojs/react";
import vercel from "@astrojs/vercel/serverless";

const macros = {
  "\\vv": "\\overrightarrow",
  "\\deg": "#1\\!\\degree",
  "\\norm": "\\lVert#1\\rVert",
};

// https://astro.build/config
export default defineConfig({
  integrations: [
    react(),
    mdx({
      rehypePlugins: [[rehypeKatex, { macros }]],
      remarkPlugins: [remarkMath],
    }),
    tailwind({}),
  ],

  output: "server",
  adapter: vercel(),
});
