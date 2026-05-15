import {
  ReactNode,
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
} from "react";

export type Lang = "en" | "zh";

const STORAGE_KEY = "sessio.lang";

const DICTS: Record<Lang, Record<string, string>> = {
  en: {
    "sidebar.all_sessions": "All Sessions",
    "sidebar.by_agent": "By Agent",
    "sidebar.by_project": "By Project",
    "sidebar.loading": "Loading…",
    "sidebar.indexing": "Indexing…",
    "sidebar.settings": "Settings",
    "sidebar.close_settings": "Close settings",
    "sidebar.language": "Language",
    "sidebar.theme": "Theme",
    "sidebar.rebuild_index": "Rebuild index",
    "sidebar.sessions_count": "{count} sessions",
    "sidebar.close": "Close sidebar",
    "sidebar.open": "Open sidebar",
    "header.search": "Search",
    "header.sessions_count": "{count} sessions",
    "list.empty": "No sessions found.",
    "list.no_user_message": "(no user message)",
    "list.unknown_project": "(unknown project)",
    "list.archived": "archived",
    "list.archived_tooltip":
      "JSONL file was removed by the agent; only index metadata remains.",
    "list.archived_tooltip_by_user":
      "Session was archived by the user; JSONL is preserved.",
    "list.subagent_count": "+{count} subagent{s}",
    "list.subagent_tooltip": "{count} subagent invocation{s}",
    "list.msgs": "{count} msgs",
    "theme.light": "Light",
    "theme.dark": "Dark",
    "theme.system": "System",
    "lang.english": "English",
    "lang.chinese": "中文",
    "detail.close": "Close",
    "detail.main": "Main",
    "detail.no_jsonl": "no jsonl",
    "detail.task": "task",
    "detail.session_archived":
      "Session content is no longer on disk — the agent removed the JSONL file and only metadata remains.",
    "detail.subagent_unreadable": "Subagent jsonl unreadable.",
    "detail.loading_messages": "Loading messages…",
    "detail.no_messages": "No messages.",
    "detail.jump_to_user_msg": "Jump to user message {n}",
    "detail.expand": "Expand",
    "detail.collapse": "Collapse",
    "detail.default_subagent_type": "agent",
    "detail.msgs": "{count} msgs",
  },
  zh: {
    "sidebar.all_sessions": "全部会话",
    "sidebar.by_agent": "按助手",
    "sidebar.by_project": "按项目",
    "sidebar.loading": "加载中…",
    "sidebar.indexing": "索引中…",
    "sidebar.settings": "设置",
    "sidebar.close_settings": "关闭设置",
    "sidebar.language": "语言",
    "sidebar.theme": "主题",
    "sidebar.rebuild_index": "重建索引",
    "sidebar.sessions_count": "{count} 个会话",
    "sidebar.close": "收起侧边栏",
    "sidebar.open": "展开侧边栏",
    "header.search": "搜索",
    "header.sessions_count": "{count} 个会话",
    "list.empty": "暂无会话。",
    "list.no_user_message": "(无用户消息)",
    "list.unknown_project": "(未知项目)",
    "list.archived": "已归档",
    "list.archived_tooltip": "JSONL 文件已被 agent 移除，仅保留索引元数据。",
    "list.archived_tooltip_by_user": "会话由用户主动归档，JSONL 仍完整保留。",
    "list.subagent_count": "+{count} 个子助手",
    "list.subagent_tooltip": "{count} 次子助手调用",
    "list.msgs": "{count} 条消息",
    "theme.light": "浅色",
    "theme.dark": "深色",
    "theme.system": "跟随系统",
    "lang.english": "English",
    "lang.chinese": "中文",
    "detail.close": "关闭",
    "detail.main": "主会话",
    "detail.no_jsonl": "无 jsonl",
    "detail.task": "任务",
    "detail.session_archived":
      "会话内容已不在磁盘上 — agent 删除了 JSONL 文件，仅保留元数据。",
    "detail.subagent_unreadable": "子助手 jsonl 无法读取。",
    "detail.loading_messages": "加载消息中…",
    "detail.no_messages": "暂无消息。",
    "detail.jump_to_user_msg": "跳转到第 {n} 条用户消息",
    "detail.expand": "展开",
    "detail.collapse": "收起",
    "detail.default_subagent_type": "助手",
    "detail.msgs": "{count} 条消息",
  },
};

function detectSystem(): Lang {
  if (typeof navigator === "undefined") return "en";
  const tag = (navigator.language || "").toLowerCase();
  return tag.startsWith("zh") ? "zh" : "en";
}

function readStored(): Lang | null {
  if (typeof localStorage === "undefined") return null;
  const v = localStorage.getItem(STORAGE_KEY);
  return v === "en" || v === "zh" ? v : null;
}

type Vars = Record<string, string | number>;
type TFn = (key: string, vars?: Vars) => string;

interface I18nCtx {
  lang: Lang;
  setLang: (l: Lang) => void;
  t: TFn;
}

const I18nContext = createContext<I18nCtx | null>(null);

export function I18nProvider({ children }: { children: ReactNode }) {
  const [lang, setLang] = useState<Lang>(() => readStored() ?? detectSystem());

  useEffect(() => {
    localStorage.setItem(STORAGE_KEY, lang);
    document.documentElement.setAttribute("lang", lang);
  }, [lang]);

  const value = useMemo<I18nCtx>(() => {
    const dict = DICTS[lang];
    const t: TFn = (key, vars) => {
      let s = dict[key] ?? key;
      if (vars) {
        for (const k of Object.keys(vars)) {
          s = s.replace(new RegExp(`\\{${k}\\}`, "g"), String(vars[k]));
        }
      }
      return s;
    };
    return { lang, setLang, t };
  }, [lang]);

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nCtx {
  const v = useContext(I18nContext);
  if (!v) throw new Error("useI18n must be used inside I18nProvider");
  return v;
}

export function useT(): TFn {
  return useI18n().t;
}

export function localeTag(lang: Lang): string {
  return lang === "zh" ? "zh-CN" : "en-US";
}
