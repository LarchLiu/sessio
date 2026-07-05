const THREAD_ID_PATTERN = /\bthread-(?!stage\b|agent\b)[A-Za-z0-9][A-Za-z0-9_-]*/g;
const SESSIO_THREAD_CREATE_PATTERN = /\bsessio\s+thread\s+create\b/;

export interface CreatedThreadIds {
  threadIds: string[];
  refreshKey: string;
}

export interface CreatedThreadCollector {
  addRefreshPart(value: string | number | null | undefined): void;
  collectText(text: string | null | undefined): void;
  result(): CreatedThreadIds;
}

export function createCreatedThreadCollector(): CreatedThreadCollector {
  const threadIds: string[] = [];
  const seen = new Set<string>();
  const refreshParts: string[] = [];

  return {
    addRefreshPart(value) {
      refreshParts.push(String(value ?? ""));
    },
    collectText(text) {
      if (!text) return;
      refreshParts.push(`${text.length}:${text.slice(-80)}`);
      for (const match of text.matchAll(THREAD_ID_PATTERN)) {
        const id = match[0];
        if (!id || seen.has(id)) continue;
        seen.add(id);
        threadIds.push(id);
      }
    },
    result() {
      return {
        threadIds,
        refreshKey: `${threadIds.join("|")}::${refreshParts.join("|")}`,
      };
    },
  };
}

export function collectCreatedThreadIdsFromTexts(texts: Iterable<string>): CreatedThreadIds {
  const collector = createCreatedThreadCollector();
  for (const text of texts) {
    collector.collectText(text);
  }
  return collector.result();
}

export function isSessioThreadCreateCommand(text: string | null | undefined): boolean {
  if (!text) return false;
  return SESSIO_THREAD_CREATE_PATTERN.test(
    text
      .replace(/\\\s*\n\s*/g, " ")
      .replace(/["',[\]]/g, " "),
  );
}
