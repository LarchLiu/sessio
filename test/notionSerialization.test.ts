import { describe, expect, it } from "vitest";
import type { PartialBlock } from "@blocknote/core";
import {
  normalizeEditorText,
  roundTripMatches,
  serializeSourceLineBlocks,
} from "../src/hooks/notionSerialization";

describe("notionSerialization", () => {
  it("serializes source fallback paragraphs back to original lines", () => {
    const blocks: PartialBlock[] = [
      {
        type: "paragraph",
        content: [{ type: "text", text: "first", styles: {} }],
        children: [],
      },
      {
        type: "paragraph",
        content: [],
        children: [],
      },
      {
        type: "paragraph",
        content: [{ type: "text", text: "third", styles: {} }],
        children: [],
      },
    ];

    expect(serializeSourceLineBlocks(blocks)).toEqual({
      ok: true,
      content: "first\n\nthird",
    });
  });

  it("rejects source fallback blocks that cannot be represented linearly", () => {
    expect(serializeSourceLineBlocks([
      {
        type: "heading",
        content: [{ type: "text", text: "title", styles: {} }],
        children: [],
      },
    ])).toMatchObject({
      ok: false,
      reason: "non_linear_source_fallback",
    });

    expect(serializeSourceLineBlocks([
      {
        type: "paragraph",
        content: [{ type: "link", href: "https://example.com", content: [] } as never],
        children: [],
      },
    ])).toMatchObject({
      ok: false,
      reason: "non_linear_source_fallback",
    });
  });

  it("normalizes CRLF for round-trip checks", () => {
    expect(normalizeEditorText("a\r\nb")).toBe("a\nb");
    expect(roundTripMatches("a\r\nb", "a\nb")).toEqual({
      safe: true,
      serialized: "a\nb",
    });
  });
});
