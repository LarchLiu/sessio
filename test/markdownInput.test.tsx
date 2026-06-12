import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { renderMarkdownInput } from "../src/components/markdownInput";

describe("renderMarkdownInput", () => {
  it("renders markdown checkboxes safely", () => {
    const html = renderToStaticMarkup(renderMarkdownInput({ type: "checkbox", checked: true })!);

    expect(html).toContain('type="checkbox"');
    expect(html).toContain("checked");
    expect(html).toContain("readOnly");
  });

  it("drops unsupported raw input tags", () => {
    expect(renderMarkdownInput({ type: "text" })).toBeNull();
    expect(renderMarkdownInput({ type: undefined })).toBeNull();
  });
});
