import { isValidElement, useMemo, type ReactNode } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import rehypeKatex from "rehype-katex";
import rehypeRaw from "rehype-raw";
import rehypeSanitize, { defaultSchema } from "rehype-sanitize";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import type { Options as SanitizeSchema } from "rehype-sanitize";
import { renderMarkdownInput } from "./markdownInput";
import ScrollArea from "./ScrollArea";
import { useEffectiveThemeType, useShikiHighlightedCode } from "./shikiHighlight";
import "katex/dist/katex.min.css";

const markdownSanitizeSchema: SanitizeSchema = {
  ...defaultSchema,
  tagNames: [
    ...(defaultSchema.tagNames ?? []),
    "details",
    "summary",
    "input",
    "section",
    "article",
  ],
  attributes: {
    ...defaultSchema.attributes,
    "*": [
      ...(defaultSchema.attributes?.["*"] ?? []),
      "className",
      "data*",
      "ariaLabel",
      "ariaHidden",
    ],
    a: [
      ...(defaultSchema.attributes?.a ?? []),
      "href",
      "title",
      "target",
      "rel",
    ],
    img: [
      ...(defaultSchema.attributes?.img ?? []),
      "alt",
      "src",
      "title",
      "width",
      "height",
    ],
    input: [["type", "checkbox"], "checked", "disabled"],
    code: [...(defaultSchema.attributes?.code ?? []), "className"],
    pre: [...(defaultSchema.attributes?.pre ?? []), "className"],
    span: [...(defaultSchema.attributes?.span ?? []), "className"],
    div: [...(defaultSchema.attributes?.div ?? []), "className"],
  },
  protocols: {
    ...defaultSchema.protocols,
    href: ["http", "https", "mailto"],
    src: ["http", "https", "data", "asset", "blob"],
  },
};

export default function PlainMarkdownPreview({ text }: { text: string }) {
  const components = useMemo(() => createMarkdownComponents(), []);

  return (
    <ScrollArea
      className="sessio-plain-editor-preview min-h-0 flex-1"
      viewportClassName="sessio-plain-editor-preview-viewport"
      persistScrollbars
    >
      <article className="sessio-plain-editor-preview-content markdown-content">
        <ReactMarkdown
          remarkPlugins={[remarkGfm, remarkBreaks, remarkMath]}
          rehypePlugins={[rehypeRaw, [rehypeSanitize, markdownSanitizeSchema], rehypeKatex]}
          components={components}
          urlTransform={markdownUrlTransform}
        >
          {text}
        </ReactMarkdown>
      </article>
    </ScrollArea>
  );
}

function createMarkdownComponents(): Components {
  return {
    p: ({ children }) => <p>{children}</p>,
    blockquote: ({ children }) => <blockquote>{children}</blockquote>,
    input: ({ type, checked, disabled }) => renderMarkdownInput({ type, checked, disabled }),
    pre: ({ children }) => <>{children}</>,
    code: ({ children, className }) => {
      if (className) {
        return (
          <PlainPreviewCodeBlock
            code={codeTextFromChildren(children)}
            language={codeLanguageFromClassName(className)}
          />
        );
      }
      return <code>{children}</code>;
    },
    a: ({ children, href }) => {
      const safe = safeHref(href ?? "");
      if (!safe) return <>{children}</>;
      return (
        <a href={safe} target="_blank" rel="noreferrer">
          {children}
        </a>
      );
    },
    img: ({ src, alt }) => {
      const safeSrc = markdownImageSrc(src ?? "");
      if (!safeSrc) {
        return <code>{`![${alt ?? "image"}](${src ?? ""})`}</code>;
      }
      return <img src={safeSrc} alt={alt ?? "image"} loading="lazy" />;
    },
  };
}

function PlainPreviewCodeBlock({
  code,
  language,
}: {
  code: string;
  language: string;
}) {
  const themeType = useEffectiveThemeType();
  const highlighted = useShikiHighlightedCode(code, language, themeType);

  return (
    <ScrollArea
      className="sessio-plain-editor-preview-code"
      viewportClassName="px-3 py-2"
      orientation="horizontal"
      persistScrollbars
    >
      <pre>
        <code>{highlighted ?? code}</code>
      </pre>
    </ScrollArea>
  );
}

function codeLanguageFromClassName(className?: string): string {
  return className
    ?.split(/\s+/)
    .map((item) => item.match(/^language-(.+)$/)?.[1])
    .find(Boolean)
    ?.toLowerCase() ?? "";
}

function codeTextFromChildren(children: ReactNode): string {
  if (typeof children === "string" || typeof children === "number") return String(children);
  if (Array.isArray(children)) return children.map(codeTextFromChildren).join("");
  if (isValidElement<{ children?: ReactNode }>(children)) {
    return codeTextFromChildren(children.props.children);
  }
  return "";
}

function markdownUrlTransform(url: string): string {
  if (url.startsWith("#")) return url;
  return safeHref(url) ?? "";
}

function safeHref(rawHref: string): string | null {
  const href = rawHref.trim();
  if (!href) return null;
  if (
    href.startsWith("/") ||
    href.startsWith("./") ||
    href.startsWith("../") ||
    href.startsWith("#")
  ) {
    return href;
  }
  try {
    const url = new URL(href);
    if (url.protocol === "http:" || url.protocol === "https:" || url.protocol === "mailto:") {
      return href;
    }
  } catch {
    return null;
  }
  return null;
}

function markdownImageSrc(rawSrc: string): string | null {
  const src = rawSrc.trim();
  if (!src) return null;
  if (
    src.startsWith("data:") ||
    src.startsWith("blob:") ||
    src.startsWith("asset:") ||
    src.startsWith("/") ||
    src.startsWith("./") ||
    src.startsWith("../")
  ) {
    return src;
  }
  try {
    const url = new URL(src);
    if (url.protocol === "http:" || url.protocol === "https:") return src;
  } catch {
    return null;
  }
  return null;
}
