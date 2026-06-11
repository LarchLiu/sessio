import { useMemo } from "react";
import ThreadWorkSnapshotPanel, { useThreadWorkSnapshot } from "../components/ThreadWorkSnapshotPanel";
import ChatPage, { type ChatPageProps } from "./ChatPage";

export default function ThreadChatPage(props: ChatPageProps) {
  const { snapshot, sources } = useThreadWorkSnapshot(props.session.agent, props.session.id);
  const beforeMessages = useMemo(
    () =>
      snapshot ? (
        <ThreadWorkSnapshotPanel
          snapshot={snapshot}
          sources={sources?.sources ?? []}
        />
      ) : null,
    [snapshot, sources],
  );

  return (
    <ChatPage
      {...props}
      beforeMessages={beforeMessages}
      showThreadPromptPlaceholders
    />
  );
}
