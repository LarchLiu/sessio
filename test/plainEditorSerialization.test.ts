import { describe, expect, it } from "vitest";
import type { PartialBlock } from "@blocknote/core";
import {
  normalizeEditorText,
  roundTripMatches,
  serializeSourceLineBlocks,
} from "../src/hooks/plainEditorSerialization";

describe("plainEditorSerialization", () => {
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

  it("allows safe Markdown formatting changes from the BlockNote serializer", () => {
    expect(roundTripMatches("- item\n- [ ] task\n", "* item\n* [ ] task\n")).toMatchObject({
      safe: true,
    });
    expect(roundTripMatches("- first\n- second\n", "* first\n\n* second\n")).toMatchObject({
      safe: true,
    });
    expect(roundTripMatches("---\n", "***\n")).toMatchObject({
      safe: true,
    });
    expect(roundTripMatches("```text\nplain\n```\n", "```\nplain\n```\n")).toMatchObject({
      safe: true,
    });
  });

  it("still rejects Markdown changes that lose document semantics", () => {
    expect(roundTripMatches("---\ntitle: Test\n---\n\n# Test\n", "***\n\n## title: Test\n\n# Test\n"))
      .toMatchObject({
        safe: false,
      });
    expect(roundTripMatches("<div>x</div>\n", "")).toMatchObject({
      safe: false,
    });
    expect(roundTripMatches("```ts\nconst x = 1;\n```\n", "```\nconst x = 1;\n```\n"))
      .toMatchObject({
        safe: false,
      });
  });
});
