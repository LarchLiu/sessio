import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { PlainMarkdownPreviewContent } from "../src/components/PlainMarkdownPreview";

describe("PlainMarkdownPreviewContent", () => {
  it("renders superscript and subscript syntax without breaking strikethrough", () => {
    const html = renderToStaticMarkup(
      <PlainMarkdownPreviewContent text={"x^2^ + H~2~O + ~~gone~~"} />,
    );

    expect(html).toContain("x<sup>2</sup>");
    expect(html).toContain("H<sub>2</sub>O");
    expect(html).toContain("<del>gone</del>");
  });

  it("wraps fixed-width HTML blocks for viewport fitting", () => {
    const html = renderToStaticMarkup(
      <PlainMarkdownPreviewContent text={'<div style="width: 1280px">wide</div>'} />,
    );

    expect(html).toContain("sessio-plain-editor-html-fit");
    expect(html).toContain("sessio-plain-editor-html-fit-inner");
    expect(html).toContain("wide");
  });

  it("adapts inline HTML colors for dark preview", () => {
    const html = renderToStaticMarkup(
      <PlainMarkdownPreviewContent
        text={'<div style="background: #f8fafc; color: #1e3a8a; border: 1px solid #e2e8f0;">dark</div>'}
        themeType="dark"
      />,
    );

    expect(html).toContain("background:hsl(");
    expect(html).toContain("color:hsl(");
    expect(html).toContain("border:1px solid hsl(");
  });

  it("adds scoped dark overrides for preview HTML style blocks", () => {
    const html = renderToStaticMarkup(
      <PlainMarkdownPreviewContent
        text={"<style>.card { background: #fef3c7; color: #334155; }</style><div class=\"card\">card</div>"}
        themeType="dark"
      />,
    );

    expect(html).toContain('.sessio-plain-editor-preview-content[data-theme-type="dark"] .card');
    expect(html).toContain("background: hsl(");
    expect(html).toContain("color: hsl(");
  });
});
