import { evaluateSync, type Jsx } from "@mdx-js/mdx";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";

import Giscus from "@giscus/react";
import * as runtime from "react/jsx-runtime";
import { useCollapse } from "react-collapsed";
import * as fa from "react-icons/fa";
import "./util.tsx";

import { useCount, CountProvider } from "./count.tsx";
import { useTags, TagsProvider } from "./tags.tsx";
import { useTheme, ThemeProvider, type Theme } from "./theme.tsx";
import { useMetadata, MetadataProvider, type Metadata } from "./metadata.tsx";

import React, { useEffect, useState, type FC, type ReactNode } from "react";
import type { MDXComponents } from "mdx/types";
import { stringToOklch } from "./util.tsx";

const GiscusDocument: FC = () => {
  const theme = useTheme();
  const metadata = useMetadata();

  return (
    <Giscus
      repo="levydsa/qed"
      repoId="R_kgDOMDj7gQ"
      category="Provas"
      categoryId="DIC_kwDOMDj7gc4CiDgE"
      mapping="specific"
      term={generateDocumentTerm(metadata)}
      strict="0"
      reactions-enabled="1"
      emit-metadata="0"
      input-position="bottom"
      theme={theme as Theme}
      lang="pt"
    />
  );
};

const GiscusQuestion: FC<{ count: number }> = ({ count }) => {
  const theme = useTheme();
  const metadata = useMetadata();

  return (
    <Giscus
      repo="levydsa/qed"
      repoId="R_kgDOMDj7gQ"
      category="Questões"
      categoryId="DIC_kwDOMDj7gc4CiDqd"
      mapping="specific"
      term={`Questão ${count} — ${generateDocumentTerm(metadata)}`}
      strict="1"
      reactions-enabled="1"
      emit-metadata="0"
      input-position="bottom"
      theme={theme}
      lang="pt"
    />
  );
};

function generateDocumentTerm(metadata: Metadata) {
  return `${metadata.title} (${metadata.university}, ${metadata.department}) (${metadata.date.customFormat()})`;
}

const Question: FC<{ tags: string[]; children: ReactNode }> = ({
  tags = [],
  children,
}) => {
  const nextCount = useCount();
  const { addTags } = useTags();

  const [count, setCount] = useState<number | null>(null);
  const { getCollapseProps, getToggleProps, isExpanded } = useCollapse();

  useEffect(() => {
    setCount(nextCount());
    addTags(tags);
  }, []);

  return (
    <>
      <div
        className="relative flex flex-col rounded-xl border-2 border-zinc-200 px-5 pt-4 shadow
          dark:border-zinc-800 dark:shadow-zinc-800"
      >
        <div>{children}</div>

        <aside className="absolute bottom-2 right-2.5 font-mono text-xs text-zinc-400 dark:text-zinc-600">
          {count}
        </aside>

        <button
          {...getToggleProps()}
          className="flex justify-center self-center px-5 py-3"
        >
          <fa.FaChevronDown
            className={`transition-transform ${isExpanded ? "rotate-180" : "rotate-0"}`}
          />
        </button>

        <section
          {...getCollapseProps()}
          className="mb-4 h-96 max-h-96 overflow-y-auto"
        >
          {isExpanded && count != null && <GiscusQuestion count={count} />}
        </section>
      </div>
    </>
  );
};

const Tags: FC = () => {
  const { tags } = useTags();

  return (
    <>
      {tags
        .sort((a, b) => a.localeCompare(b))
        .map((value) => {
          const { light, dark } = stringToOklch(value);

          return (
            <div
              key={value}
              style={
                {
                  "--light-tag-color": light,
                  "--dark-tag-color": dark,
                } as React.CSSProperties
              }
              className="tag mx-0.5 my-0.5 text-nowrap rounded-xl px-2"
            >
              {value}
            </div>
          );
        })}
    </>
  );
};

export const components: MDXComponents = {
  Question,
  ul({ children }) {
    return <ul className={"mb-8 mt-4 flex flex-col gap-3"}>{children}</ul>;
  },
  Image({ invertable, ...rest }) {
    return (
      <img
        className={`${invertable && "mx-auto my-4 rounded opacity-80 dark:invert"}`}
        {...rest}
      />
    );
  },
  Note({ children }) {
    return (
      <div
        className="my-10 rounded-xl border-2 border-orange-200 bg-orange-50 px-5 py-4
          dark:border-orange-500/50 dark:bg-orange-500/20"
      >
        {children}
      </div>
    );
  },
};

export const macros = {
  "\\vv": "\\overrightarrow",
  "\\deg": "#1\\!\\degree",
  "\\norm": "\\lVert#1\\rVert",
};

export const MdxDocument: FC<{ body: string; metadata: Metadata }> = ({
  body,
  metadata,
}) => {
  const [Content, setContent] = useState<React.FC | null>(null);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    const { default: Content } = evaluateSync(body, {
      rehypePlugins: [[rehypeKatex, { macros: macros }]],
      remarkPlugins: [remarkMath],
      jsx: runtime.jsx as Jsx,
      jsxs: runtime.jsxs as Jsx,
      Fragment: runtime.Fragment,
    });

    // NOTE: Eu não faço a menor ideia do porquê que isso só funciona assim.
    setContent(() => () => <Content components={components} />);

    setLoaded(true);
  }, [body]);

  return (
    <>
      <TagsProvider initial={metadata.tags}>
        <MetadataProvider value={metadata}>
          <ThemeProvider>
            <h1 className="text-center font-['Computer_Modern_Serif'] text-2xl">
              {metadata.title}
            </h1>
            <aside className="text-nowrap text-center opacity-60">
              {`(${metadata.university}, ${metadata.department}) (${metadata.date.customFormat()})`}
            </aside>

            <div className="mt-2 flex flex-wrap justify-center">
              <Tags />
            </div>

            <div
              className={`my-10 hyphens-auto text-justify font-['Computer_Modern_Serif'] text-lg
                leading-tight transition-opacity duration-500
                ${loaded ? "opacity-100" : "opacity-0"}`}
            >
              <CountProvider>{Content && <Content />}</CountProvider>
            </div>

            <div className="mx-auto mb-16">
              {metadata && <GiscusDocument />}
            </div>
          </ThemeProvider>
        </MetadataProvider>
      </TagsProvider>
    </>
  );
};
