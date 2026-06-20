import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { Circle, Plus, X } from "lucide-react";
import {
  closeTerminal,
  createTerminal,
  listTerminals,
  resizeTerminal,
  type TerminalEventEnvelope,
  type TerminalSessionInfo,
  writeTerminalInput,
} from "../api";
import { useI18n } from "../i18n";
import { useTheme } from "../theme";
import Tooltip from "./Tooltip";

const TERMINAL_DOCK_HEIGHT_STORAGE_KEY = "sessio.terminalDockHeight";
const TERMINAL_DOCK_DEFAULT_HEIGHT = 220;
const TERMINAL_DOCK_MIN_HEIGHT = 168;
const TERMINAL_DOCK_MAX_HEIGHT = 520;
const TERMINAL_DOCK_VIEWPORT_MARGIN = 220;

function clampTerminalDockHeight(height: number): number {
  const viewportMax =
    typeof window === "undefined"
      ? TERMINAL_DOCK_MAX_HEIGHT
      : Math.max(
          TERMINAL_DOCK_MIN_HEIGHT,
          Math.min(
            TERMINAL_DOCK_MAX_HEIGHT,
            Math.floor(window.innerHeight - TERMINAL_DOCK_VIEWPORT_MARGIN),
          ),
        );
  return Math.min(viewportMax, Math.max(TERMINAL_DOCK_MIN_HEIGHT, height));
}

function readTerminalDockHeight(): number {
  if (typeof localStorage === "undefined") {
    return TERMINAL_DOCK_DEFAULT_HEIGHT;
  }
  const raw = localStorage.getItem(TERMINAL_DOCK_HEIGHT_STORAGE_KEY);
  const next = raw ? Number(raw) : NaN;
  return Number.isFinite(next)
    ? clampTerminalDockHeight(next)
    : TERMINAL_DOCK_DEFAULT_HEIGHT;
}

function normalizeCwd(cwd?: string | null): string {
  return cwd?.trim() || "~";
}

function estimateTerminalSize(
  width: number,
  height: number,
): { cols: number; rows: number } {
  const cols = Math.max(20, Math.floor((width - 20) / 9));
  const rows = Math.max(8, Math.floor((height - 16) / 20));
  return { cols, rows };
}

type TerminalSessionMap = Record<string, TerminalSessionInfo>;

export default function TerminalDock({
  open,
  defaultCwd,
  onOpenChange,
}: {
  open: boolean;
  defaultCwd?: string | null;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useI18n();
  const { effective } = useTheme();
  const [height, setHeight] = useState<number>(() => readTerminalDockHeight());
  const [dragging, setDragging] = useState(false);
  const [sessions, setSessions] = useState<TerminalSessionMap>({});
  const [sessionOrder, setSessionOrder] = useState<string[]>([]);
  const [activeTerminalId, setActiveTerminalId] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const dragStartRef = useRef<{ y: number; height: number } | null>(null);
  const terminalViewportRef = useRef<HTMLDivElement | null>(null);
  const terminalHostRef = useRef<HTMLDivElement | null>(null);
  const xtermRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const terminalSizeRef = useRef<{ cols: number; rows: number }>({ cols: 120, rows: 32 });
  const activeTerminalIdRef = useRef<string | null>(null);
  const loadingRef = useRef(false);
  const titleCarryRef = useRef<Record<string, string>>({});
  const replayingOutputRef = useRef(false);
  const replayTokenRef = useRef(0);
  const terminalTheme = useMemo(
    () =>
      effective === "light"
        ? {
            background: "#eeeeec",
            foreground: "#172033",
            cursor: "#172033",
            selectionBackground: "rgba(71, 111, 255, 0.22)",
            black: "#1f2937",
            red: "#dc2626",
            green: "#15803d",
            yellow: "#b45309",
            blue: "#2563eb",
            magenta: "#c026d3",
            cyan: "#0891b2",
            white: "#f8fafc",
            brightBlack: "#6b7280",
            brightRed: "#ef4444",
            brightGreen: "#22c55e",
            brightYellow: "#f59e0b",
            brightBlue: "#3b82f6",
            brightMagenta: "#d946ef",
            brightCyan: "#06b6d4",
            brightWhite: "#ffffff",
          }
        : {
            background: "#1e232c",
            foreground: "#e5e7eb",
            cursor: "#f8fafc",
            selectionBackground: "rgba(120, 162, 255, 0.28)",
            black: "#0f172a",
            red: "#f87171",
            green: "#4ade80",
            yellow: "#facc15",
            blue: "#60a5fa",
            magenta: "#f472b6",
            cyan: "#22d3ee",
            white: "#e5e7eb",
            brightBlack: "#64748b",
            brightRed: "#fca5a5",
            brightGreen: "#86efac",
            brightYellow: "#fde047",
            brightBlue: "#93c5fd",
            brightMagenta: "#f9a8d4",
            brightCyan: "#67e8f9",
            brightWhite: "#f8fafc",
          },
    [effective],
  );

  useEffect(() => {
    if (typeof localStorage === "undefined") return;
    localStorage.setItem(
      TERMINAL_DOCK_HEIGHT_STORAGE_KEY,
      String(clampTerminalDockHeight(height)),
    );
  }, [height]);

  useEffect(() => {
    const onResize = () => {
      setHeight((current) => clampTerminalDockHeight(current));
    };
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("resize", onResize);
    };
  }, []);

  useEffect(() => {
    if (!dragging) return;
    const previousCursor = document.body.style.cursor;
    const previousUserSelect = document.body.style.userSelect;
    document.body.style.cursor = "row-resize";
    document.body.style.userSelect = "none";
    return () => {
      document.body.style.cursor = previousCursor;
      document.body.style.userSelect = previousUserSelect;
    };
  }, [dragging]);

  useEffect(() => {
    activeTerminalIdRef.current = activeTerminalId;
  }, [activeTerminalId]);

  const orderedSessions = useMemo(
    () =>
      sessionOrder
        .map((id) => sessions[id])
        .filter((session): session is TerminalSessionInfo => Boolean(session)),
    [sessionOrder, sessions],
  );

  const activeSession = useMemo(
    () =>
      (activeTerminalId ? sessions[activeTerminalId] : null) ??
      orderedSessions[0] ??
      null,
    [activeTerminalId, orderedSessions, sessions],
  );
  const showTerminalLoading =
    loading || Boolean(activeSession?.running && activeSession.output.length === 0);

  const ensureTerminal = useCallback(async () => {
    if (!open || loadingRef.current) return;
    loadingRef.current = true;
    setLoading(true);
    try {
      const items = await listTerminals();
      const sorted = items.slice().sort((left, right) => left.createdAtMs - right.createdAtMs);
      const carry: Record<string, string> = {};
      const byId = sorted.reduce<TerminalSessionMap>((acc, item) => {
        const resolved = resolveTerminalSessionTitle(item);
        acc[item.id] = resolved.session;
        if (resolved.carry) {
          carry[item.id] = resolved.carry;
        }
        return acc;
      }, {});
      titleCarryRef.current = carry;
      setSessions(byId);
      setSessionOrder(sorted.map((item) => item.id));
      if (sorted.length > 0) {
        setActiveTerminalId((current) =>
          current && byId[current] ? current : sorted[sorted.length - 1]?.id ?? null,
        );
      } else {
        const created = await createTerminal({
          cwd: normalizeCwd(defaultCwd),
          cols: terminalSizeRef.current.cols,
          rows: terminalSizeRef.current.rows,
        });
        setSessions({ [created.id]: created });
        setSessionOrder([created.id]);
        setActiveTerminalId(created.id);
      }
    } finally {
      loadingRef.current = false;
      setLoading(false);
    }
  }, [defaultCwd, open]);

  useEffect(() => {
    void ensureTerminal();
  }, [ensureTerminal]);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<TerminalEventEnvelope>("terminal-event", ({ payload }) => {
      if (cancelled) return;
      const { terminalId, event } = payload;
      if (event.kind === "created") {
        titleCarryRef.current[terminalId] = "";
        setSessions((current) => ({ ...current, [terminalId]: event.session }));
        setSessionOrder((current) =>
          current.includes(terminalId) ? current : [...current, terminalId],
        );
        setActiveTerminalId(terminalId);
        return;
      }
      if (event.kind === "output") {
        const resolvedTitle = resolveTerminalTitleChunk(
          titleCarryRef.current[terminalId] ?? "",
          event.data,
        );
        titleCarryRef.current[terminalId] = resolvedTitle.carry;
        if (activeTerminalIdRef.current === terminalId) {
          xtermRef.current?.write(event.data);
        }
        setSessions((current) => {
          const session = current[terminalId];
          if (!session) return current;
          return {
            ...current,
            [terminalId]: {
              ...session,
              title: resolvedTitle.title ?? session.title,
              output: appendOutput(session.output, event.data),
            },
          };
        });
        return;
      }
      if (event.kind === "resized") {
        setSessions((current) => {
          const session = current[terminalId];
          if (!session) return current;
          return {
            ...current,
            [terminalId]: {
              ...session,
              cols: event.cols,
              rows: event.rows,
            },
          };
        });
        return;
      }
      if (event.kind === "closed") {
        setSessions((current) => {
          const session = current[terminalId];
          if (!session) return current;
          return {
            ...current,
            [terminalId]: {
              ...session,
              running: false,
              exitCode: event.exitCode,
            },
          };
        });
        return;
      }
      if (event.kind === "removed") {
        delete titleCarryRef.current[terminalId];
        setSessions((current) => {
          const next = { ...current };
          delete next[terminalId];
          return next;
        });
        setSessionOrder((current) => {
          const next = current.filter((id) => id !== terminalId);
          const fallbackId = next[next.length - 1] ?? null;
          setActiveTerminalId((currentActive) =>
            currentActive === terminalId ? fallbackId : currentActive,
          );
          if (next.length === 0) {
            onOpenChange(false);
          }
          return next;
        });
      }
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch((error) => console.warn("terminal-event subscribe failed", error));
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [onOpenChange, open]);

  useEffect(() => {
    if (!open) return;
    if (sessionOrder.length > 0) return;
    void ensureTerminal();
  }, [ensureTerminal, open, sessionOrder.length]);

  const startResize = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (event.button !== 0) return;
      event.currentTarget.setPointerCapture(event.pointerId);
      dragStartRef.current = { y: event.clientY, height };
      setDragging(true);
    },
    [height],
  );

  const resizeDock = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    const start = dragStartRef.current;
    if (!start) return;
    const delta = start.y - event.clientY;
    setHeight(clampTerminalDockHeight(start.height + delta));
  }, []);

  const stopResize = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    if (!dragStartRef.current) return;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    dragStartRef.current = null;
    setDragging(false);
  }, []);

  const syncTerminalSize = useCallback(async () => {
    const viewport = terminalViewportRef.current;
    const activeId = activeTerminalIdRef.current;
    if (!viewport || !activeId) return;
    fitAddonRef.current?.fit();
    const terminal = xtermRef.current;
    const fallback = estimateTerminalSize(viewport.clientWidth, viewport.clientHeight);
    const nextSize = terminal
      ? {
          cols: Math.max(20, terminal.cols),
          rows: Math.max(8, terminal.rows),
        }
      : fallback;
    const prev = terminalSizeRef.current;
    if (prev.cols === nextSize.cols && prev.rows === nextSize.rows) return;
    terminalSizeRef.current = nextSize;
    try {
      await resizeTerminal(activeId, nextSize.cols, nextSize.rows);
    } catch (error) {
      console.warn("resize terminal failed", error);
    }
  }, []);

  const focusTerminal = useCallback(() => {
    const terminal = xtermRef.current;
    if (!terminal) return;
    window.requestAnimationFrame(() => {
      xtermRef.current?.focus();
    });
  }, []);

  const replaySessionOutput = useCallback(
    (session: TerminalSessionInfo | null) => {
      const terminal = xtermRef.current;
      if (!terminal) return;
      const token = replayTokenRef.current + 1;
      replayTokenRef.current = token;
      replayingOutputRef.current = true;
      terminal.reset();
      const completeReplay = () => {
        if (replayTokenRef.current !== token) return;
        replayingOutputRef.current = false;
        void syncTerminalSize();
        focusTerminal();
      };
      if (!session?.output) {
        completeReplay();
        return;
      }
      terminal.write(session.output, completeReplay);
    },
    [focusTerminal, syncTerminalSize],
  );

  useLayoutEffect(() => {
    if (!open || !terminalHostRef.current) return;
    const terminal = new Terminal({
      cursorBlink: true,
      fontFamily: '"SFMono-Regular", "SF Mono", Menlo, Consolas, monospace',
      fontSize: 12.5,
      lineHeight: 1.35,
      allowTransparency: true,
      scrollback: 2000,
      theme: terminalTheme,
    });
    const fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.loadAddon(new WebLinksAddon());
    terminal.open(terminalHostRef.current);
    terminal.onData((data) => {
      if (replayingOutputRef.current) return;
      const activeId = activeTerminalIdRef.current;
      if (!activeId) return;
      void writeTerminalInput(activeId, data).catch((error) => {
        console.warn("write terminal input failed", error);
      });
    });
    xtermRef.current = terminal;
    fitAddonRef.current = fitAddon;
    void syncTerminalSize();
    focusTerminal();
    return () => {
      replayTokenRef.current += 1;
      replayingOutputRef.current = false;
      fitAddonRef.current = null;
      xtermRef.current = null;
      terminal.dispose();
    };
  }, [focusTerminal, open, syncTerminalSize]);

  useEffect(() => {
    const terminal = xtermRef.current;
    if (!terminal) return;
    terminal.options = { ...terminal.options, theme: terminalTheme };
  }, [terminalTheme]);

  useEffect(() => {
    if (!open || !terminalViewportRef.current) return;
    const resizeObserver = new ResizeObserver(() => {
      void syncTerminalSize();
    });
    resizeObserver.observe(terminalViewportRef.current);
    return () => {
      resizeObserver.disconnect();
    };
  }, [open, syncTerminalSize]);

  useEffect(() => {
    if (!open) return;
    replaySessionOutput(activeSession);
  }, [activeSession?.id, open, replaySessionOutput]);

  useEffect(() => {
    if (!open || loading) return;
    focusTerminal();
  }, [focusTerminal, loading, open]);

  const handleAddTerminal = useCallback(async () => {
    if (!open) onOpenChange(true);
    const created = await createTerminal({
      cwd: normalizeCwd(defaultCwd),
      cols: terminalSizeRef.current.cols,
      rows: terminalSizeRef.current.rows,
    });
    setSessions((current) => ({ ...current, [created.id]: created }));
    setSessionOrder((current) =>
      current.includes(created.id) ? current : [...current, created.id],
    );
    setActiveTerminalId(created.id);
  }, [defaultCwd, onOpenChange, open]);

  const handleCloseTerminal = useCallback(
    async (terminalId: string) => {
      await closeTerminal(terminalId);
    },
    [],
  );

  const handleTabKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLButtonElement>) => {
      if (!activeSession) return;
      if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
      event.preventDefault();
      const currentIndex = sessionOrder.indexOf(activeSession.id);
      if (currentIndex < 0) return;
      const nextIndex =
        event.key === "ArrowRight"
          ? Math.min(sessionOrder.length - 1, currentIndex + 1)
          : Math.max(0, currentIndex - 1);
      setActiveTerminalId(sessionOrder[nextIndex] ?? null);
    },
    [activeSession, sessionOrder],
  );

  if (!open) return null;

  return (
    <div
      className={
        "flex shrink-0 flex-col overflow-hidden bg-surface-sidebar " +
        (dragging ? "select-none" : "")
      }
      style={{ height: clampTerminalDockHeight(height) }}
    >
      <div
        aria-label={t("terminal_dock.resize")}
        aria-orientation="horizontal"
        aria-valuemin={TERMINAL_DOCK_MIN_HEIGHT}
        aria-valuemax={TERMINAL_DOCK_MAX_HEIGHT}
        aria-valuenow={clampTerminalDockHeight(height)}
        className="app-terminal-dock-resize-handle shrink-0"
        data-dragging={dragging ? "true" : undefined}
        role="separator"
        tabIndex={0}
        onPointerDown={startResize}
        onPointerMove={resizeDock}
        onPointerUp={stopResize}
        onPointerCancel={stopResize}
        onKeyDown={(event) => {
          if (event.key === "ArrowUp") {
            event.preventDefault();
            setHeight((current) => clampTerminalDockHeight(current + 16));
          } else if (event.key === "ArrowDown") {
            event.preventDefault();
            setHeight((current) => clampTerminalDockHeight(current - 16));
          } else if (event.key === "Home") {
            event.preventDefault();
            setHeight(TERMINAL_DOCK_MIN_HEIGHT);
          } else if (event.key === "End") {
            event.preventDefault();
            setHeight(clampTerminalDockHeight(TERMINAL_DOCK_MAX_HEIGHT));
          }
        }}
      />
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
        <div className="flex h-9 shrink-0 items-center gap-2 border-b border-ink/10 bg-surface-sidebar px-2.5">
          <div className="hide-native-scrollbar flex min-w-0 flex-1 items-center gap-2 overflow-x-auto">
            {orderedSessions.map((session) => {
              const active = session.id === activeSession?.id;
              return (
                <div
                  key={session.id}
                  className={
                    "flex shrink-0 items-center rounded-md transition-colors " +
                    (active
                      ? "bg-ink/10 text-ink"
                      : "text-ink/65 hover:bg-ink/5 hover:text-ink")
                  }
                >
                  <button
                    type="button"
                    aria-pressed={active}
                    onClick={() => setActiveTerminalId(session.id)}
                    onKeyDown={handleTabKeyDown}
                    className="flex min-w-0 items-center gap-2 px-2.5 py-1 text-left"
                    title={session.title}
                  >
                    <Circle
                      className={
                        "h-2.5 w-2.5 shrink-0 " +
                        (session.running ? "fill-emerald-400 text-emerald-400" : "fill-ink/35 text-ink/35")
                      }
                    />
                    <span className="max-w-[14rem] truncate text-body-sm font-medium">
                      {session.title}
                    </span>
                  </button>
                  <button
                    type="button"
                    aria-label={t("terminal_dock.close")}
                    onClick={() => void handleCloseTerminal(session.id)}
                    className="mr-0.5 rounded p-1 text-ink/35 transition-colors hover:bg-ink/[0.08] hover:text-ink/75"
                    title={t("terminal_dock.close")}
                  >
                    <X className="h-3 w-3" />
                  </button>
                </div>
              );
            })}
            <Tooltip content={t("terminal_dock.add")} placement="top">
              <button
                type="button"
                aria-label={t("terminal_dock.add")}
                onClick={() => void handleAddTerminal()}
                className="shrink-0 rounded-md p-1 text-ink/55 transition-colors hover:bg-ink/[0.06] hover:text-ink"
              >
                <Plus className="h-4 w-4" />
              </button>
            </Tooltip>
          </div>
          <Tooltip content={t("terminal_dock.hide")} placement="top">
            <button
              type="button"
              aria-label={t("terminal_dock.hide")}
              onClick={() => onOpenChange(false)}
              className="shrink-0 rounded-md p-1 text-ink/55 transition-colors hover:bg-ink/[0.06] hover:text-ink"
            >
              <X className="h-4 w-4" />
            </button>
          </Tooltip>
        </div>
        <div
          className="flex min-h-0 flex-1 overflow-hidden bg-surface-sidebar"
        >
          <div
            ref={terminalViewportRef}
            className="relative flex min-h-0 min-w-0 flex-1 overflow-hidden bg-surface-sidebar"
          >
            <div
              ref={terminalHostRef}
              className="h-full min-h-0 min-w-0 flex-1 overflow-hidden bg-surface-sidebar"
            />
            {showTerminalLoading ? (
              <div
                className={
                  "pointer-events-none absolute inset-0 flex items-center justify-center " +
                  (effective === "light" ? "bg-white/18" : "bg-black/16")
                }
              >
                <div
                  className={
                    "rounded-full border px-3 py-1 text-caption " +
                    (effective === "light"
                      ? "border-black/8 bg-white/78 text-ink/68"
                      : "border-white/10 bg-black/45 text-white/72")
                  }
                >
                  {t("terminal_dock.loading")}
                </div>
              </div>
            ) : !activeSession ? (
              <div
                className={
                  "pointer-events-none absolute inset-0 flex items-center justify-center text-body-sm " +
                  (effective === "light" ? "text-ink/45" : "text-white/50")
                }
              >
                {t("terminal_dock.empty")}
              </div>
            ) : null}
          </div>
        </div>
      </div>
    </div>
  );
}

function appendOutput(current: string, chunk: string): string {
  const next = current + chunk;
  if (next.length <= 256 * 1024) return next;
  return next.slice(next.length - 256 * 1024);
}

function resolveTerminalSessionTitle(session: TerminalSessionInfo): {
  session: TerminalSessionInfo;
  carry: string;
} {
  const resolved = resolveTerminalTitleChunk("", session.output);
  return {
    session: {
      ...session,
      title: resolved.title ?? session.title,
    },
    carry: resolved.carry,
  };
}

function resolveTerminalTitleChunk(
  carry: string,
  chunk: string,
): { title: string | null; carry: string } {
  const input = carry + chunk;
  let title: string | null = null;
  let nextCarry = "";
  let cursor = 0;

  while (cursor < input.length) {
    const start = input.indexOf("\x1b]", cursor);
    if (start === -1) break;

    const codeIndex = start + 2;
    if (codeIndex >= input.length) {
      nextCarry = input.slice(start);
      break;
    }

    const code = input[codeIndex];
    if (code !== "0" && code !== "1" && code !== "2") {
      cursor = codeIndex;
      continue;
    }

    const separatorIndex = codeIndex + 1;
    if (separatorIndex >= input.length) {
      nextCarry = input.slice(start);
      break;
    }
    if (input[separatorIndex] !== ";") {
      cursor = separatorIndex;
      continue;
    }

    const valueStart = separatorIndex + 1;
    const bellIndex = input.indexOf("\x07", valueStart);
    const stIndex = input.indexOf("\x1b\\", valueStart);

    let valueEnd = -1;
    let terminatorLength = 0;
    if (bellIndex === -1 && stIndex === -1) {
      nextCarry = input.slice(start);
      break;
    }
    if (bellIndex !== -1 && (stIndex === -1 || bellIndex < stIndex)) {
      valueEnd = bellIndex;
      terminatorLength = 1;
    } else {
      valueEnd = stIndex;
      terminatorLength = 2;
    }

    const candidate = normalizeTerminalTitle(input.slice(valueStart, valueEnd));
    if (candidate) {
      title = candidate;
    }
    cursor = valueEnd + terminatorLength;
  }

  return { title, carry: nextCarry };
}

function normalizeTerminalTitle(value: string): string | null {
  const cleaned = value.replace(/[\u0000-\u001f\u007f]+/g, " ").replace(/\s+/g, " ").trim();
  return cleaned || null;
}
