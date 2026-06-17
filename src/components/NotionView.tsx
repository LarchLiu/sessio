import { MantineProvider } from "@mantine/core";
import { BlockNoteView } from "@blocknote/mantine";
import { useCreateBlockNote } from "@blocknote/react";
import "@blocknote/mantine/style.css";
import "./notion-theme.css";
import { useNotionDoc } from "../hooks/useNotionDoc";
import { useEffectiveThemeType } from "./shikiHighlight";

export interface NotionViewProps {
  fileKey: string;
  text: string;
}

export default function NotionView({ fileKey, text }: NotionViewProps) {
  const themeType = useEffectiveThemeType();
  const editor = useCreateBlockNote();
  const { usedSourceFallback } = useNotionDoc(editor, fileKey, text);

  return (
    <MantineProvider>
      <div
        className="sessio-notion-view h-full min-h-0 min-w-0 overflow-auto"
        data-theme-type={themeType}
        data-source-fallback={usedSourceFallback ? "true" : "false"}
      >
        <BlockNoteView
          editor={editor}
          editable={false}
          theme={themeType}
          className="sessio-notion-editor"
        />
      </div>
    </MantineProvider>
  );
}
