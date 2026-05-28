import { useEffect, useRef, useState } from "react";
import { Search, X } from "lucide-react";
import {
  type ProjectMemorySearchResult,
  searchProjectMemory,
} from "../api";
import { useI18n } from "../i18n";
import InlineMenuSelect, { type InlineMenuSelectOption } from "./InlineMenuSelect";
import ScrollArea from "./ScrollArea";

type ProjectMemorySearchDialogProps = {
  open: boolean;
  initialProjectKey: string;
  projects: Array<{ key: string; label: string }>;
  activeProjectKey: string | null;
  onClose: () => void;
  onExited: () => void;
};

export default function ProjectMemorySearchDialog({
  open,
  initialProjectKey,
  projects,
  activeProjectKey,
  onClose,
  onExited,
}: ProjectMemorySearchDialogProps) {
  const { t } = useI18n();
  const [selectedProjectKey, setSelectedProjectKey] = useState(initialProjectKey);
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [results, setResults] = useState<ProjectMemorySearchResult[]>([]);
  const [searched, setSearched] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const lockedProject = activeProjectKey !== null;
  const selectedProject =
    projects.find((project) => project.key === selectedProjectKey) ?? projects[0];

  useEffect(() => {
    if (!open) return;
    const id = requestAnimationFrame(() => inputRef.current?.focus());
    return () => cancelAnimationFrame(id);
  }, [open]);

  useEffect(() => {
    const nextProjectKey =
      activeProjectKey ??
      (projects.some((project) => project.key === selectedProjectKey)
        ? selectedProjectKey
        : projects[0]?.key);
    if (nextProjectKey && nextProjectKey !== selectedProjectKey) {
      setSelectedProjectKey(nextProjectKey);
      setResults([]);
      setError(null);
      setLoading(false);
      setSearched(false);
    }
  }, [activeProjectKey, projects, selectedProjectKey]);

  const runSearch = () => {
    const text = query.trim();
    setError(null);
    if (!text) {
      setResults([]);
      setLoading(false);
      setSearched(false);
      return;
    }
    setLoading(true);
    setSearched(true);
    searchProjectMemory(selectedProjectKey, text)
      .then((rows) => {
        setResults(rows);
      })
      .catch((err) => {
        setResults([]);
        setError(String(err));
      })
      .finally(() => setLoading(false));
  };

  const selectProject = (key: string) => {
    setSelectedProjectKey(key);
    setResults([]);
    setError(null);
    setLoading(false);
    setSearched(false);
  };

  const clearOrClose = () => {
    if (query.trim()) {
      setQuery("");
      setResults([]);
      setError(null);
      setLoading(false);
      setSearched(false);
      return;
    }
    onClose();
  };

  return (
    <div
      className={
        "project-memory-search-dialog absolute inset-x-0 top-12 bottom-0 z-30 bg-black/35 backdrop-blur-sm flex items-start justify-center pt-10 px-4 " +
        (open ? "project-memory-search-dialog-in" : "project-memory-search-dialog-out")
      }
      onClick={onClose}
      onAnimationEnd={(event) => {
        if (!open && event.currentTarget === event.target) {
          onExited();
        }
      }}
    >
      <div
        className={
          "project-memory-search-panel w-full max-w-[680px] bg-surface-panel border border-ink/10 shadow-[0_24px_80px_rgba(0,0,0,0.22)] rounded-lg overflow-hidden " +
          (open ? "project-memory-search-panel-in" : "project-memory-search-panel-out")
        }
        onClick={(event) => event.stopPropagation()}
      >
        <div className="flex items-center gap-2 px-3 py-2 border-b border-ink/10">
          {!lockedProject && (
            <InlineMenuSelect
              value={selectedProjectKey}
              options={projects.map(
                (project): InlineMenuSelectOption => ({
                  value: project.key,
                  label: project.label,
                }),
              )}
              onChange={selectProject}
              menuAlign="parent"
              placeholder={t("list.unknown_project")}
              ariaLabel={t("memory_search.project_selector")}
              className="max-w-[128px]"
            />
          )}
          <input
            ref={inputRef}
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setSearched(false);
              setResults([]);
              setError(null);
            }}
            onKeyDown={(event) => {
              if (event.key === "Escape") onClose();
              if (event.key === "Enter") runSearch();
            }}
            placeholder={t("memory_search.placeholder", { project: selectedProject?.label ?? "" })}
            className="flex-1 min-w-0 bg-transparent outline-none text-body text-ink placeholder:text-ink/35"
          />
          <button
            type="button"
            aria-label={t("header.search")}
            onClick={runSearch}
            disabled={loading || !query.trim()}
            className="p-1 text-ink/45 hover:text-ink disabled:opacity-35 disabled:hover:text-ink/45 rounded-md transition"
          >
            <Search className="w-4 h-4" />
          </button>
          <button
            type="button"
            aria-label={query.trim() ? t("list.clear") : t("detail.close")}
            onClick={clearOrClose}
            className="p-1 text-ink/45 hover:text-ink rounded-md transition"
          >
            <X className="w-4 h-4" />
          </button>
        </div>
        <ScrollArea className="max-h-[55vh]">
          {loading && (
            <div className="px-4 py-6 text-center text-body-sm text-ink/45">
              {t("memory_search.searching")}
            </div>
          )}
          {!loading && error && (
            <div className="m-4 p-3 rounded bg-status-error/10 text-status-error text-body-sm">
              {error}
            </div>
          )}
          {!loading && !error && searched && query.trim() && results.length === 0 && (
            <div className="px-4 py-6 text-center text-body-sm text-ink/45">
              {t("memory_search.empty")}
            </div>
          )}
          {!loading && !error && results.length > 0 && (
            <ul className="divide-y divide-ink/5">
              {results.map((result, index) => (
                <li key={`${result.recordId ?? result.artifactUri ?? index}`} className="px-4 py-3">
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <div className="text-body-sm font-medium text-ink truncate">
                        {result.title ?? result.recordId ?? result.artifactUri ?? t("memory_search.result")}
                      </div>
                      {result.snippet && (
                        <div className="mt-1 text-body-sm text-ink/60 overflow-hidden [display:-webkit-box] [-webkit-line-clamp:3] [-webkit-box-orient:vertical]">
                          {result.snippet}
                        </div>
                      )}
                      {(result.recordId || result.artifactUri) && (
                        <div className="mt-1 text-meta text-ink/35 truncate">
                          {result.recordId ?? result.artifactUri}
                        </div>
                      )}
                    </div>
                    {result.score !== null && (
                      <span className="shrink-0 text-meta tabular-nums text-ink/40">
                        {result.score.toFixed(3)}
                      </span>
                    )}
                  </div>
                </li>
              ))}
            </ul>
          )}
        </ScrollArea>
      </div>
    </div>
  );
}
