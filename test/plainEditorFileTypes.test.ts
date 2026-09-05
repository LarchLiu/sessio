import { describe, expect, it } from "vitest";
import {
  isPlainEditorEditableDocumentPath,
  isPlainEditorHtmlDocumentPath,
  isPlainEditorPreviewableDocumentPath,
} from "../src/hooks/plainEditorFileTypes";

describe("plain editor HTML file types", () => {
  it.each(["index.html", "partial.htm", "/workspace/PAGE.HTML?version=1"])(
    "treats %s as editable HTML",
    (path) => {
      expect(isPlainEditorEditableDocumentPath(path)).toBe(true);
      expect(isPlainEditorHtmlDocumentPath(path)).toBe(true);
      expect(isPlainEditorPreviewableDocumentPath(path)).toBe(true);
    },
  );

  it("does not classify other text documents as HTML", () => {
    expect(isPlainEditorHtmlDocumentPath("README.md")).toBe(false);
    expect(isPlainEditorHtmlDocumentPath("template.html.txt")).toBe(false);
  });
});
