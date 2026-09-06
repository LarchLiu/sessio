import { useEffect, useRef, useState } from "react";
import {
  readLocalTextFile,
  writeSessioAppFile,
  type SessioAppFileWriteRequest,
  type SessioAppPermission,
} from "../api";
import { useI18n } from "../i18n";
import { useEffectiveThemeType } from "./shikiHighlight";
import SwitchControl from "./SwitchControl";

export type SessioPreviewTheme = "light" | "dark";

export const SESSIO_THEME_MESSAGE_TYPE = "sessio-theme-change";
export const SESSIO_APP_FILE_WRITE_REQUEST_TYPE = "sessio-app-write-file";
export const SESSIO_APP_FILE_WRITE_RESULT_TYPE = "sessio-app-write-file-result";
export const SESSIO_CHAT_BACKGROUND_BY_THEME: Record<SessioPreviewTheme, string> = {
  light: "#f6f6f4",
  dark: "#232831",
};

const SESSIO_THEME_BRIDGE_SCRIPT = `(() => {
  const applyTheme = (theme, chatBackground) => {
    if (theme !== "light" && theme !== "dark") return;
    document.documentElement.setAttribute("data-sessio-theme", theme);
    document.documentElement.style.colorScheme = theme;
    if (typeof chatBackground === "string" && /^#[0-9a-f]{6}$/i.test(chatBackground)) {
      document.documentElement.style.setProperty("--sessio-chat-background", chatBackground);
    }
    window.dispatchEvent(new CustomEvent("sessio:themechange", {
      detail: { theme, chatBackground }
    }));
  };
  window.addEventListener("message", (event) => {
    if (event.source !== window.parent) return;
    const message = event.data;
    if (!message || message.source !== "sessio" || message.type !== "${SESSIO_THEME_MESSAGE_TYPE}") return;
    applyTheme(message.theme, message.chatBackground);
  });
})();`;

const STATIC_PREVIEW_CSP = [
  "default-src 'none'",
  "img-src data: blob:",
  "media-src data: blob:",
  "style-src 'unsafe-inline'",
  "font-src data:",
  "script-src 'none'",
  "connect-src 'none'",
  "frame-src 'none'",
  "object-src 'none'",
  "base-uri 'none'",
  "form-action 'none'",
].join("; ");

const SCRIPT_PREVIEW_CSP = STATIC_PREVIEW_CSP.replace(
  "script-src 'none'",
  "script-src 'unsafe-inline' blob: data:",
);

const APP_PERMISSION_POLICY: Record<
  SessioAppPermission,
  { allow?: string; sandbox?: string }
> = {
  autoplay: {
    allow: "autoplay",
  },
  clipboardWrite: {
    allow: "clipboard-write",
  },
  downloads: {
    sandbox: "allow-downloads",
  },
  fullscreen: {
    allow: "fullscreen",
  },
  gamepad: {
    allow: "gamepad",
  },
  modals: {
    sandbox: "allow-modals",
  },
  pointerLock: {
    sandbox: "allow-pointer-lock",
  },
  popups: {
    sandbox: "allow-popups",
  },
};

export interface SessioAppFileWriteMessage {
  source: "sessio-app";
  type: typeof SESSIO_APP_FILE_WRITE_REQUEST_TYPE;
  requestId: string;
  path: string;
  data: string;
  encoding: "utf8" | "base64";
  overwrite: boolean;
}

export function parseSessioAppFileWriteMessage(
  value: unknown,
): SessioAppFileWriteMessage | null {
  if (!value || typeof value !== "object") return null;
  const message = value as Record<string, unknown>;
  if (
    message.source !== "sessio-app" ||
    message.type !== SESSIO_APP_FILE_WRITE_REQUEST_TYPE ||
    typeof message.requestId !== "string" ||
    message.requestId.length === 0 ||
    message.requestId.length > 128 ||
    typeof message.path !== "string" ||
    typeof message.data !== "string"
  ) {
    return null;
  }
  const encoding = message.encoding ?? "utf8";
  if (encoding !== "utf8" && encoding !== "base64") return null;
  return {
    source: "sessio-app",
    type: SESSIO_APP_FILE_WRITE_REQUEST_TYPE,
    requestId: message.requestId,
    path: message.path,
    data: message.data,
    encoding,
    overwrite: message.overwrite === true,
  };
}

export function resolveAppIframePermissions(
  permissions: readonly SessioAppPermission[],
  scriptsEnabled: boolean,
): { allow?: string; sandbox: string } {
  const policies = scriptsEnabled
    ? [...new Set(permissions)].map((permission) => APP_PERMISSION_POLICY[permission])
    : [];
  const allow = policies.flatMap((policy) => policy.allow ?? []).join("; ");
  return {
    allow: allow || undefined,
    sandbox: [
      scriptsEnabled && "allow-scripts",
      ...policies.flatMap((policy) => policy.sandbox ?? []),
    ]
      .filter(Boolean)
      .join(" "),
  };
}

export function buildPlainHtmlPreviewDocument(
  html: string,
  scriptsEnabled: boolean,
  theme: SessioPreviewTheme = "dark",
): string {
  const document = new DOMParser().parseFromString(html, "text/html");

  document.documentElement.setAttribute("data-sessio-theme", theme);
  document.documentElement.style.colorScheme = theme;
  document.documentElement.style.setProperty(
    "--sessio-chat-background",
    SESSIO_CHAT_BACKGROUND_BY_THEME[theme],
  );

  document.querySelectorAll("base").forEach((element) => element.remove());
  document
    .querySelectorAll('meta[http-equiv="refresh" i]')
    .forEach((element) => element.remove());
  document.querySelectorAll<HTMLAnchorElement>("a[href]").forEach((element) => {
    element.target = "_blank";
    element.rel = "noopener noreferrer";
  });

  const securityPolicy = document.createElement("meta");
  securityPolicy.httpEquiv = "Content-Security-Policy";
  securityPolicy.content = scriptsEnabled ? SCRIPT_PREVIEW_CSP : STATIC_PREVIEW_CSP;
  document.head.prepend(securityPolicy);

  if (scriptsEnabled) {
    const themeBridge = document.createElement("script");
    themeBridge.setAttribute("data-sessio-theme-bridge", "");
    themeBridge.textContent = SESSIO_THEME_BRIDGE_SCRIPT;
    securityPolicy.after(themeBridge);
  }

  if (!document.querySelector('meta[name="viewport" i]')) {
    const viewport = document.createElement("meta");
    viewport.name = "viewport";
    viewport.content = "width=device-width, initial-scale=1";
    securityPolicy.after(viewport);
  }

  return `<!doctype html>\n${document.documentElement.outerHTML}`;
}

export function resolveLocalScriptPath(src: string, htmlPath: string): string | null {
  const rawValue = src.trim().split(/[?#]/, 1)[0];
  if (!rawValue || rawValue.startsWith("/") || rawValue.startsWith("\\") || rawValue.startsWith("//") || /^[a-z][a-z\d+.-]*:/i.test(rawValue)) {
    return null;
  }

  let value: string;
  try {
    value = decodeURIComponent(rawValue);
  } catch {
    return null;
  }

  const normalizedHtmlPath = htmlPath.replaceAll("\\", "/");
  const baseSegments = normalizedHtmlPath.split("/");
  baseSegments.pop();
  for (const segment of value.split("/")) {
    if (!segment || segment === ".") continue;
    if (segment === ".." || segment.includes("\\")) return null;
    else baseSegments.push(segment);
  }
  return baseSegments.join("/");
}

async function inlineLocalScripts(html: string, htmlPath: string): Promise<string> {
  const document = new DOMParser().parseFromString(html, "text/html");
  const scripts = Array.from(document.querySelectorAll<HTMLScriptElement>("script[src]"));

  await Promise.all(
    scripts.map(async (script) => {
      const scriptPath = resolveLocalScriptPath(script.getAttribute("src") ?? "", htmlPath);
      if (!scriptPath) return;
      try {
        const source = await readLocalTextFile(scriptPath);
        script.removeAttribute("src");
        script.textContent = source;
      } catch {
        script.remove();
      }
    }),
  );

  return `<!doctype html>\n${document.documentElement.outerHTML}`;
}

export default function PlainHtmlPreview({
  html,
  filePath,
  scriptsInitiallyEnabled = false,
  showScriptsControl = true,
  permissions = [],
  appDirectoryPath = null,
}: {
  html: string;
  filePath: string | null;
  scriptsInitiallyEnabled?: boolean;
  showScriptsControl?: boolean;
  permissions?: readonly SessioAppPermission[];
  appDirectoryPath?: string | null;
}) {
  const { t } = useI18n();
  const themeType = useEffectiveThemeType();
  const themeTypeRef = useRef(themeType);
  themeTypeRef.current = themeType;
  const iframeRef = useRef<HTMLIFrameElement | null>(null);
  const [scriptPermission, setScriptPermission] = useState<{
    filePath: string | null;
    enabled: boolean;
  }>({ filePath, enabled: scriptsInitiallyEnabled });
  const scriptsEnabled =
    scriptPermission.filePath === filePath && scriptPermission.enabled;
  const [previewDocument, setPreviewDocument] = useState(() =>
    buildPlainHtmlPreviewDocument(html, scriptsEnabled, themeType),
  );

  useEffect(() => {
    setScriptPermission({ filePath, enabled: scriptsInitiallyEnabled });
  }, [filePath, scriptsInitiallyEnabled]);

  useEffect(() => {
    let active = true;
    const prepare = async () => {
      const source = scriptsEnabled && filePath
        ? await inlineLocalScripts(html, filePath)
        : html;
      if (active) {
        setPreviewDocument(
          buildPlainHtmlPreviewDocument(source, scriptsEnabled, themeTypeRef.current),
        );
      }
    };
    void prepare();
    return () => {
      active = false;
    };
  }, [filePath, html, scriptsEnabled]);

  useEffect(() => {
    if (!scriptsEnabled) {
      setPreviewDocument(buildPlainHtmlPreviewDocument(html, false, themeType));
      return;
    }
    iframeRef.current?.contentWindow?.postMessage(
      {
        source: "sessio",
        type: SESSIO_THEME_MESSAGE_TYPE,
        theme: themeType,
        chatBackground: SESSIO_CHAT_BACKGROUND_BY_THEME[themeType],
      },
      "*",
    );
  }, [html, scriptsEnabled, themeType]);

  useEffect(() => {
    if (
      !scriptsEnabled ||
      !appDirectoryPath ||
      !permissions.includes("downloads")
    ) {
      return;
    }

    const handleMessage = (event: MessageEvent) => {
      if (event.source !== iframeRef.current?.contentWindow) return;
      const message = parseSessioAppFileWriteMessage(event.data);
      if (!message) return;

      const request: SessioAppFileWriteRequest = {
        appDirectoryPath,
        relativePath: message.path,
        data: message.data,
        encoding: message.encoding,
        overwrite: message.overwrite,
      };
      const reply = (payload: Record<string, unknown>) => {
        iframeRef.current?.contentWindow?.postMessage(
          {
            source: "sessio",
            type: SESSIO_APP_FILE_WRITE_RESULT_TYPE,
            requestId: message.requestId,
            ...payload,
          },
          "*",
        );
      };

      void writeSessioAppFile(request).then(
        (result) => reply({ ok: true, ...result }),
        (error) => reply({ ok: false, error: String(error) }),
      );
    };

    window.addEventListener("message", handleMessage);
    return () => window.removeEventListener("message", handleMessage);
  }, [appDirectoryPath, permissions, scriptsEnabled]);

  const scriptsLabel = t("chat.files.html_preview_scripts");
  const iframePermissions = resolveAppIframePermissions(permissions, scriptsEnabled);

  return (
    <div className="sessio-plain-html-preview h-full min-h-0 flex-1">
      {showScriptsControl && (
        <div className="sessio-plain-html-preview-toolbar">
          <span>{scriptsLabel}</span>
          <SwitchControl
            checked={scriptsEnabled}
            tooltip={scriptsLabel}
            ariaLabel={scriptsLabel}
            onToggle={() =>
              setScriptPermission({ filePath, enabled: !scriptsEnabled })
            }
          />
        </div>
      )}
      <iframe
        ref={iframeRef}
        key={scriptsEnabled ? "scripts-enabled" : "scripts-disabled"}
        title={t("chat.files.html_preview_title")}
        className="sessio-plain-html-preview-frame"
        referrerPolicy="no-referrer"
        allow={iframePermissions.allow}
        sandbox={iframePermissions.sandbox}
        srcDoc={previewDocument}
        onLoad={() => {
          if (!scriptsEnabled) return;
          iframeRef.current?.contentWindow?.postMessage(
            {
              source: "sessio",
              type: SESSIO_THEME_MESSAGE_TYPE,
              theme: themeType,
              chatBackground: SESSIO_CHAT_BACKGROUND_BY_THEME[themeType],
            },
            "*",
          );
        }}
      />
    </div>
  );
}
