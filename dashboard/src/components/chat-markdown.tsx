import { memo } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import { PrismLight as SyntaxHighlighter } from "react-syntax-highlighter";
import { oneDark } from "react-syntax-highlighter/dist/esm/styles/prism";
import "katex/dist/katex.min.css";

// Register only the languages we need
import javascript from "react-syntax-highlighter/dist/esm/languages/prism/javascript";
import typescript from "react-syntax-highlighter/dist/esm/languages/prism/typescript";
import jsx from "react-syntax-highlighter/dist/esm/languages/prism/jsx";
import tsx from "react-syntax-highlighter/dist/esm/languages/prism/tsx";
import python from "react-syntax-highlighter/dist/esm/languages/prism/python";
import rust from "react-syntax-highlighter/dist/esm/languages/prism/rust";
import go from "react-syntax-highlighter/dist/esm/languages/prism/go";
import c from "react-syntax-highlighter/dist/esm/languages/prism/c";
import cpp from "react-syntax-highlighter/dist/esm/languages/prism/cpp";
import csharp from "react-syntax-highlighter/dist/esm/languages/prism/csharp";
import java from "react-syntax-highlighter/dist/esm/languages/prism/java";
import zig from "react-syntax-highlighter/dist/esm/languages/prism/zig";
import sql from "react-syntax-highlighter/dist/esm/languages/prism/sql";
import bash from "react-syntax-highlighter/dist/esm/languages/prism/bash";
import yaml from "react-syntax-highlighter/dist/esm/languages/prism/yaml";
import json from "react-syntax-highlighter/dist/esm/languages/prism/json";
import toml from "react-syntax-highlighter/dist/esm/languages/prism/toml";
import markup from "react-syntax-highlighter/dist/esm/languages/prism/markup";
import css from "react-syntax-highlighter/dist/esm/languages/prism/css";
import solidity from "react-syntax-highlighter/dist/esm/languages/prism/solidity";
import docker from "react-syntax-highlighter/dist/esm/languages/prism/docker";
import ruby from "react-syntax-highlighter/dist/esm/languages/prism/ruby";
import kotlin from "react-syntax-highlighter/dist/esm/languages/prism/kotlin";
import swift from "react-syntax-highlighter/dist/esm/languages/prism/swift";
import lua from "react-syntax-highlighter/dist/esm/languages/prism/lua";
import haskell from "react-syntax-highlighter/dist/esm/languages/prism/haskell";
import diff from "react-syntax-highlighter/dist/esm/languages/prism/diff";
import graphql from "react-syntax-highlighter/dist/esm/languages/prism/graphql";
import protobuf from "react-syntax-highlighter/dist/esm/languages/prism/protobuf";
import hcl from "react-syntax-highlighter/dist/esm/languages/prism/hcl";
import makefile from "react-syntax-highlighter/dist/esm/languages/prism/makefile";
import latex from "react-syntax-highlighter/dist/esm/languages/prism/latex";
import regex from "react-syntax-highlighter/dist/esm/languages/prism/regex";
import markdown from "react-syntax-highlighter/dist/esm/languages/prism/markdown";
import php from "react-syntax-highlighter/dist/esm/languages/prism/php";
import perl from "react-syntax-highlighter/dist/esm/languages/prism/perl";
import scala from "react-syntax-highlighter/dist/esm/languages/prism/scala";
import r from "react-syntax-highlighter/dist/esm/languages/prism/r";
import dart from "react-syntax-highlighter/dist/esm/languages/prism/dart";

SyntaxHighlighter.registerLanguage("javascript", javascript);
SyntaxHighlighter.registerLanguage("typescript", typescript);
SyntaxHighlighter.registerLanguage("jsx", jsx);
SyntaxHighlighter.registerLanguage("tsx", tsx);
SyntaxHighlighter.registerLanguage("python", python);
SyntaxHighlighter.registerLanguage("rust", rust);
SyntaxHighlighter.registerLanguage("go", go);
SyntaxHighlighter.registerLanguage("c", c);
SyntaxHighlighter.registerLanguage("cpp", cpp);
SyntaxHighlighter.registerLanguage("csharp", csharp);
SyntaxHighlighter.registerLanguage("java", java);
SyntaxHighlighter.registerLanguage("zig", zig);
SyntaxHighlighter.registerLanguage("sql", sql);
SyntaxHighlighter.registerLanguage("bash", bash);
SyntaxHighlighter.registerLanguage("yaml", yaml);
SyntaxHighlighter.registerLanguage("json", json);
SyntaxHighlighter.registerLanguage("toml", toml);
SyntaxHighlighter.registerLanguage("html", markup);
SyntaxHighlighter.registerLanguage("xml", markup);
SyntaxHighlighter.registerLanguage("css", css);
SyntaxHighlighter.registerLanguage("solidity", solidity);
SyntaxHighlighter.registerLanguage("dockerfile", docker);
SyntaxHighlighter.registerLanguage("docker", docker);
SyntaxHighlighter.registerLanguage("ruby", ruby);
SyntaxHighlighter.registerLanguage("kotlin", kotlin);
SyntaxHighlighter.registerLanguage("swift", swift);
SyntaxHighlighter.registerLanguage("lua", lua);
SyntaxHighlighter.registerLanguage("haskell", haskell);
SyntaxHighlighter.registerLanguage("diff", diff);
SyntaxHighlighter.registerLanguage("graphql", graphql);
SyntaxHighlighter.registerLanguage("protobuf", protobuf);
SyntaxHighlighter.registerLanguage("hcl", hcl);
SyntaxHighlighter.registerLanguage("terraform", hcl);
SyntaxHighlighter.registerLanguage("makefile", makefile);
SyntaxHighlighter.registerLanguage("latex", latex);
SyntaxHighlighter.registerLanguage("tex", latex);
SyntaxHighlighter.registerLanguage("regex", regex);
SyntaxHighlighter.registerLanguage("markdown", markdown);
SyntaxHighlighter.registerLanguage("php", php);
SyntaxHighlighter.registerLanguage("perl", perl);
SyntaxHighlighter.registerLanguage("scala", scala);
SyntaxHighlighter.registerLanguage("r", r);
SyntaxHighlighter.registerLanguage("dart", dart);

const PROSE_CLASSES =
  "prose prose-invert prose-sm max-w-none break-words " +
  "[&_p]:my-1.5 [&_ul]:my-1.5 [&_ol]:my-1.5 [&_li]:my-0.5 " +
  "[&_h1]:text-[15px] [&_h2]:text-[14px] [&_h3]:text-[13px] " +
  "[&_h1]:mt-3 [&_h2]:mt-2.5 [&_h3]:mt-2 " +
  "[&_hr]:border-white/[0.08] [&_strong]:text-zinc-100 [&_a]:text-blue-400 [&_a]:hover:text-blue-300 " +
  "[&_table]:w-full [&_table]:table-fixed [&_table]:border-collapse [&_table]:text-[12px] " +
  "[&_th]:text-left [&_th]:px-3 [&_th]:py-2 [&_th]:border-b [&_th]:border-white/[0.1] [&_th]:text-zinc-400 [&_th]:font-medium [&_th]:align-top " +
  "[&_td]:px-3 [&_td]:py-2 [&_td]:border-b [&_td]:border-white/[0.06] [&_td]:text-zinc-300 [&_td]:align-top " +
  "[&_blockquote]:border-l-2 [&_blockquote]:border-white/[0.1] [&_blockquote]:pl-4 [&_blockquote]:text-zinc-400";

const LANGUAGE_MAP: Record<string, string> = {
  js: "javascript",
  ts: "typescript",
  py: "python",
  rb: "ruby",
  rs: "rust",
  sh: "bash",
  shell: "bash",
  zsh: "bash",
  yml: "yaml",
  tf: "terraform",
  sol: "solidity",
  cs: "csharp",
  "c++": "cpp",
  "c#": "csharp",
  proto: "protobuf",
  gql: "graphql",
  md: "markdown",
  make: "makefile",
  tex: "latex",
};

function resolveLanguage(raw: string): string {
  const lower = raw.toLowerCase();
  return LANGUAGE_MAP[lower] ?? lower;
}

function CodeBlock({ className, children, ...props }: React.HTMLAttributes<HTMLElement> & { children?: React.ReactNode }) {
  const match = /language-(\w+)/.exec(className || "");
  const code = String(children).replace(/\n$/, "");

  if (!match) {
    return (
      <code className="text-[12px] bg-white/[0.08] px-1.5 py-0.5 rounded-md text-orange-300" {...props}>
        {children}
      </code>
    );
  }

  const lang = resolveLanguage(match[1]);

  return (
    <SyntaxHighlighter
      style={oneDark}
      language={lang}
      PreTag="div"
      customStyle={{
        margin: 0,
        padding: "1rem",
        borderRadius: "0.75rem",
        fontSize: "12px",
        lineHeight: "1.6",
        background: "rgba(255,255,255,0.04)",
        border: "1px solid rgba(255,255,255,0.06)",
      }}
      codeTagProps={{ style: { fontFamily: "inherit" } }}
    >
      {code}
    </SyntaxHighlighter>
  );
}

const components = {
  code: CodeBlock as any,
};

const remarkPlugins = [remarkGfm, remarkMath];
const rehypePlugins = [rehypeKatex];

export const ChatMarkdown = memo(function ChatMarkdown({ text }: { text: string }) {
  return (
    <div className={PROSE_CLASSES}>
      <Markdown
        remarkPlugins={remarkPlugins}
        rehypePlugins={rehypePlugins}
        components={components}
      >
        {text}
      </Markdown>
    </div>
  );
});
