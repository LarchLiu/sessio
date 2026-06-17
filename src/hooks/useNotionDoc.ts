import { useEffect, useMemo } from "react";
import type { PartialBlock } from "@blocknote/core";
import type { useCreateBlockNote } from "@blocknote/react";

type BlockNoteEditor = ReturnType<typeof useCreateBlockNote>;

export interface ParsedNotionDoc {
  blocks: PartialBlock[];
  usedSourceFallback: boolean;
}

export function useNotionDoc(
  editor: BlockNoteEditor,
  fileKey: string,
  text: string,
): ParsedNotionDoc {
  const parsed = useMemo(
    () => parseMarkdownBlocksWithFallback(editor, text),
    [editor, text],
  );

  useEffect(() => {
    editor.replaceBlocks(editor.document, parsed.blocks);
  }, [editor, fileKey, parsed]);

  return parsed;
}

export function parseMarkdownBlocksWithFallback(
  editor: BlockNoteEditor,
  text: string,
): ParsedNotionDoc {
  try {
    const blocks = editor.tryParseMarkdownToBlocks(text);
    if (blocks.length > 0 || text.length === 0) {
      return {
        blocks: blocks.length > 0 ? blocks : [buildSourceLineBlock("")],
        usedSourceFallback: false,
      };
    }
  } catch {
    // Fall through to source-line blocks below.
  }

  return {
    blocks: buildSourceLineBlocks(text),
    usedSourceFallback: true,
  };
}

function buildSourceLineBlocks(text: string): PartialBlock[] {
  const lines = text.split("\n");
  return lines.map(buildSourceLineBlock);
}

function buildSourceLineBlock(line: string): PartialBlock {
  return {
    type: "paragraph",
    content: line
      ? [
          {
            type: "text",
            text: line,
            styles: {},
          },
        ]
      : [],
    children: [],
  };
}
