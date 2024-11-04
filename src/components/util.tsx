declare global {
  interface Date {
    customFormat(): string;
  }
}

import { renderToString } from "react-dom/server";

Date.prototype.customFormat = function () {
  return this.toISOString().split("T")[0];
};

import { evaluateSync, type Jsx } from "@mdx-js/mdx";
import { components, macros } from "./MdxPage";
import * as runtime from "react/jsx-runtime";
import rehypeKatex from "rehype-katex";
import remarkMath from "remark-math";
import { hash } from "./siphash13";

export function stringToOklch(s: string) {
  const n = hash(0, s);

  const c = (n.h & 0xff) / (255 * 3);
  const h = ((n.l & 0xff) / 255) * 360;

  const light = `oklch(0.8 ${c} ${h})`;
  const dark = `oklch(0.6 ${c} ${h})`;

  return { light, dark };
}

export function sumTags(body: string) {
  const module = evaluateSync(body, {
    rehypePlugins: [[rehypeKatex, { macros: macros }]],
    remarkPlugins: [remarkMath],
    jsx: runtime.jsx as Jsx,
    jsxs: runtime.jsxs as Jsx,
    Fragment: runtime.Fragment,
  });

  var allTags: string[] = [];

  renderToString(
    <module.default
      components={{
        ...components,
        Question({ tags = [], children }) {
          allTags.push(...tags);
          return children;
        },
      }}
    />,
  );

  return [...new Set(allTags)];
}
