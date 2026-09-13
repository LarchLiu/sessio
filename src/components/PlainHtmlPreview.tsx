import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
} from "react";
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
export const SESSIO_APP_STATE_PROBE_TYPE = "sessio-app-state-probe";
export const SESSIO_APP_STATE_READY_TYPE = "sessio-app-state-ready";
export const SESSIO_APP_STATE_RESTORE_TYPE = "sessio-app-state-restore";
export const SESSIO_APP_STATE_SAVE_REQUEST_TYPE = "sessio-app-state-save-request";
export const SESSIO_APP_STATE_SAVE_RESULT_TYPE = "sessio-app-state-save-result";
export const SESSIO_APP_STATE_MAX_BYTES = 1024 * 1024;
const SESSIO_APP_STATE_SAVE_TIMEOUT_MS = 500;
export const SESSIO_CHAT_BACKGROUND_BY_THEME: Record<SessioPreviewTheme, string> = {
  light: "#f6f6f4",
  dark: "#232831",
};

export const SESSIO_PREVIEW_BRIDGE_SCRIPT = `(() => {
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
    if (!message || message.source !== "sessio") return;
    if (message.type === "${SESSIO_THEME_MESSAGE_TYPE}") {
      applyTheme(message.theme, message.chatBackground);
      return;
    }

    const bridge = window.SESSIO_APP_STATE;
    const schemaVersion = Number(bridge?.schemaVersion);
    const bridgeReady =
      Number.isSafeInteger(schemaVersion) &&
      schemaVersion > 0 &&
      typeof bridge?.capture === "function" &&
      typeof bridge?.restore === "function";

    if (message.type === "${SESSIO_APP_STATE_PROBE_TYPE}") {
      if (bridgeReady) {
        window.parent.postMessage({
          source: "sessio-app",
          type: "${SESSIO_APP_STATE_READY_TYPE}",
          schemaVersion
        }, "*");
      }
      return;
    }

    if (!bridgeReady) return;
    if (message.type === "${SESSIO_APP_STATE_RESTORE_TYPE}") {
      if (message.schemaVersion === schemaVersion) {
        try {
          bridge.restore(message.state);
        } catch (error) {
          console.error("Sessio App state restore failed", error);
        }
      }
      return;
    }

    if (
      message.type === "${SESSIO_APP_STATE_SAVE_REQUEST_TYPE}" &&
      typeof message.requestId === "string"
    ) {
      Promise.resolve()
        .then(() => bridge.capture())
        .then((state) => {
          window.parent.postMessage({
            source: "sessio-app",
            type: "${SESSIO_APP_STATE_SAVE_RESULT_TYPE}",
            requestId: message.requestId,
            ok: true,
            schemaVersion,
            state
          }, "*");
        })
        .catch((error) => {
          window.parent.postMessage({
            source: "sessio-app",
            type: "${SESSIO_APP_STATE_SAVE_RESULT_TYPE}",
            requestId: message.requestId,
            ok: false,
            error: String(error)
          }, "*");
        });
    }
  });
})();`;

function previewCsp(scriptsEnabled: boolean, appResourceBase: string | null): string {
  const appResourceSource = appResourceBase ? " sessio-app:" : "";
  return [
    "default-src 'none'",
    `img-src data: blob:${appResourceSource}`,
    `media-src data: blob:${appResourceSource}`,
    `style-src 'unsafe-inline' https://fonts.googleapis.com${appResourceSource}`,
    `font-src data: https://fonts.gstatic.com${appResourceSource}`,
    `script-src ${scriptsEnabled ? "'unsafe-inline' blob: data:" : "'none'"}${appResourceSource}`,
    `connect-src${appResourceSource || " 'none'"}`,
    "frame-src 'none'",
    "object-src 'none'",
    `base-uri ${appResourceBase ? "sessio-app:" : "'none'"}`,
    "form-action 'none'",
  ].join("; ");
}

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

export interface SessioAppStateSnapshot {
  schemaVersion: number;
  state: unknown;
}

export interface PlainHtmlPreviewHandle {
  saveAppState: () => Promise<SessioAppStateSnapshot | null>;
}

interface SessioAppStateReadyMessage {
  schemaVersion: number;
}

interface SessioAppStateSaveResultMessage {
  requestId: string;
  snapshot: SessioAppStateSnapshot | null;
}

function isJsonValue(value: unknown, ancestors = new Set<object>()): boolean {
  if (value === null || typeof value === "string" || typeof value === "boolean") {
    return true;
  }
  if (typeof value === "number") return Number.isFinite(value);
  if (typeof value !== "object") return false;
  if (ancestors.has(value)) return false;

  const prototype = Object.getPrototypeOf(value);
  if (!Array.isArray(value) && prototype !== Object.prototype && prototype !== null) {
    return false;
  }

  ancestors.add(value);
  const valid = Array.isArray(value)
    ? value.every((item) => isJsonValue(item, ancestors))
    : Object.values(value).every((item) => isJsonValue(item, ancestors));
  ancestors.delete(value);
  return valid;
}

function normalizeAppStateSnapshot(
  schemaVersion: unknown,
  state: unknown,
): SessioAppStateSnapshot | null {
  if (
    typeof schemaVersion !== "number" ||
    !Number.isSafeInteger(schemaVersion) ||
    schemaVersion <= 0
  ) {
    return null;
  }
  if (!isJsonValue(state)) return null;
  try {
    const serialized = JSON.stringify({ schemaVersion, state });
    if (new TextEncoder().encode(serialized).byteLength > SESSIO_APP_STATE_MAX_BYTES) {
      return null;
    }
    const snapshot = JSON.parse(serialized) as SessioAppStateSnapshot;
    return snapshot;
  } catch {
    return null;
  }
}

export function parseSessioAppStateReadyMessage(
  value: unknown,
): SessioAppStateReadyMessage | null {
  if (!value || typeof value !== "object") return null;
  const message = value as Record<string, unknown>;
  if (
    message.source !== "sessio-app" ||
    message.type !== SESSIO_APP_STATE_READY_TYPE ||
    typeof message.schemaVersion !== "number" ||
    !Number.isSafeInteger(message.schemaVersion) ||
    message.schemaVersion <= 0
  ) {
    return null;
  }
  return { schemaVersion: message.schemaVersion };
}

export function parseSessioAppStateSaveResultMessage(
  value: unknown,
): SessioAppStateSaveResultMessage | null {
  if (!value || typeof value !== "object") return null;
  const message = value as Record<string, unknown>;
  if (
    message.source !== "sessio-app" ||
    message.type !== SESSIO_APP_STATE_SAVE_RESULT_TYPE ||
    typeof message.requestId !== "string" ||
    message.requestId.length === 0 ||
    message.requestId.length > 128 ||
    typeof message.ok !== "boolean"
  ) {
    return null;
  }
  if (!message.ok) return { requestId: message.requestId, snapshot: null };
  if (!Object.prototype.hasOwnProperty.call(message, "state")) return null;
  const snapshot = normalizeAppStateSnapshot(message.schemaVersion, message.state);
  if (!snapshot) return null;
  return { requestId: message.requestId, snapshot };
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
  appResourceBase: string | null = null,
): string {
  const document = new DOMParser().parseFromString(html, "text/html");

  document.documentElement.setAttribute("data-sessio-theme", theme);
  document.documentElement.style.colorScheme = theme;
  document.documentElement.style.setProperty(
    "--sessio-chat-background",
    SESSIO_CHAT_BACKGROUND_BY_THEME[theme],
  );

  document.querySelectorAll("base").forEach((element) => element.remove());
  if (appResourceBase) {
    const base = document.createElement("base");
    base.href = appResourceBase.endsWith("/") ? appResourceBase : `${appResourceBase}/`;
    document.head.prepend(base);
  }
  document
    .querySelectorAll('meta[http-equiv="refresh" i]')
    .forEach((element) => element.remove());
  document.querySelectorAll<HTMLAnchorElement>("a[href]").forEach((element) => {
    element.target = "_blank";
    element.rel = "noopener noreferrer";
  });

  const securityPolicy = document.createElement("meta");
  securityPolicy.httpEquiv = "Content-Security-Policy";
  securityPolicy.content = previewCsp(scriptsEnabled, appResourceBase);
  document.head.prepend(securityPolicy);

  if (scriptsEnabled) {
    const themeBridge = document.createElement("script");
    themeBridge.setAttribute("data-sessio-theme-bridge", "");
    themeBridge.textContent = SESSIO_PREVIEW_BRIDGE_SCRIPT;
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

interface PlainHtmlPreviewProps {
  html: string;
  filePath: string | null;
  scriptsInitiallyEnabled?: boolean;
  showScriptsControl?: boolean;
  permissions?: readonly SessioAppPermission[];
  appDirectoryPath?: string | null;
  appResourceBase?: string | null;
  appStateBridgeEnabled?: boolean;
  cachedAppState?: SessioAppStateSnapshot | null;
  onAppStateSnapshot?: (snapshot: SessioAppStateSnapshot) => void;
}

const PlainHtmlPreview = forwardRef<PlainHtmlPreviewHandle, PlainHtmlPreviewProps>(
function PlainHtmlPreview({
  html,
  filePath,
  scriptsInitiallyEnabled = false,
  showScriptsControl = true,
  permissions = [],
  appDirectoryPath = null,
  appResourceBase = null,
  appStateBridgeEnabled = false,
  cachedAppState = null,
  onAppStateSnapshot,
}, ref) {
  const { t } = useI18n();
  const themeType = useEffectiveThemeType();
  const themeTypeRef = useRef(themeType);
  themeTypeRef.current = themeType;
  const iframeRef = useRef<HTMLIFrameElement | null>(null);
  const stateBridgeSchemaVersionRef = useRef<number | null>(null);
  const cachedAppStateRef = useRef(cachedAppState);
  const onAppStateSnapshotRef = useRef(onAppStateSnapshot);
  const stateSaveSequenceRef = useRef(0);
  const pendingStateSaveRef = useRef<{
    requestId: string;
    promise: Promise<SessioAppStateSnapshot | null>;
    resolve: (snapshot: SessioAppStateSnapshot | null) => void;
    timeout: ReturnType<typeof setTimeout>;
  } | null>(null);
  const stateProbeTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  cachedAppStateRef.current = cachedAppState;
  onAppStateSnapshotRef.current = onAppStateSnapshot;
  const [scriptPermission, setScriptPermission] = useState<{
    filePath: string | null;
    enabled: boolean;
  }>({ filePath, enabled: scriptsInitiallyEnabled });
  const scriptsEnabled =
    scriptPermission.filePath === filePath && scriptPermission.enabled;
  const [previewDocument, setPreviewDocument] = useState(() =>
    buildPlainHtmlPreviewDocument(html, scriptsEnabled, themeType, appResourceBase),
  );

  const settlePendingStateSave = useCallback(
    (snapshot: SessioAppStateSnapshot | null) => {
      const pending = pendingStateSaveRef.current;
      if (!pending) return;
      clearTimeout(pending.timeout);
      pendingStateSaveRef.current = null;
      if (snapshot) onAppStateSnapshotRef.current?.(snapshot);
      pending.resolve(snapshot);
    },
    [],
  );

  const saveAppState = useCallback((): Promise<SessioAppStateSnapshot | null> => {
    if (
      !appStateBridgeEnabled ||
      !scriptsEnabled ||
      stateBridgeSchemaVersionRef.current === null ||
      !iframeRef.current?.contentWindow
    ) {
      return Promise.resolve(null);
    }
    if (pendingStateSaveRef.current) return pendingStateSaveRef.current.promise;

    const requestId = `state-${Date.now()}-${++stateSaveSequenceRef.current}`;
    let resolvePromise: (snapshot: SessioAppStateSnapshot | null) => void = () => {};
    const promise = new Promise<SessioAppStateSnapshot | null>((resolve) => {
      resolvePromise = resolve;
    });
    const timeout = setTimeout(
      () => settlePendingStateSave(null),
      SESSIO_APP_STATE_SAVE_TIMEOUT_MS,
    );
    pendingStateSaveRef.current = {
      requestId,
      promise,
      resolve: resolvePromise,
      timeout,
    };
    iframeRef.current.contentWindow.postMessage(
      {
        source: "sessio",
        type: SESSIO_APP_STATE_SAVE_REQUEST_TYPE,
        requestId,
      },
      "*",
    );
    return promise;
  }, [appStateBridgeEnabled, scriptsEnabled, settlePendingStateSave]);

  useImperativeHandle(ref, () => ({ saveAppState }), [saveAppState]);

  useEffect(() => {
    setScriptPermission({ filePath, enabled: scriptsInitiallyEnabled });
  }, [filePath, scriptsInitiallyEnabled]);

  useEffect(() => {
    let active = true;
    const prepare = async () => {
      const source = scriptsEnabled && filePath && !appResourceBase
        ? await inlineLocalScripts(html, filePath)
        : html;
      if (active) {
        setPreviewDocument(
          buildPlainHtmlPreviewDocument(
            source,
            scriptsEnabled,
            themeTypeRef.current,
            appResourceBase,
          ),
        );
      }
    };
    void prepare();
    return () => {
      active = false;
    };
  }, [appResourceBase, filePath, html, scriptsEnabled]);

  useEffect(() => {
    if (!scriptsEnabled) {
      setPreviewDocument(buildPlainHtmlPreviewDocument(html, false, themeType, appResourceBase));
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
  }, [appResourceBase, html, scriptsEnabled, themeType]);

  useEffect(() => {
    if (!appStateBridgeEnabled || !scriptsEnabled) {
      stateBridgeSchemaVersionRef.current = null;
      settlePendingStateSave(null);
      return;
    }

    const handleStateMessage = (event: MessageEvent) => {
      if (event.source !== iframeRef.current?.contentWindow) return;

      const ready = parseSessioAppStateReadyMessage(event.data);
      if (ready) {
        if (stateProbeTimerRef.current) {
          clearInterval(stateProbeTimerRef.current);
          stateProbeTimerRef.current = null;
        }
        stateBridgeSchemaVersionRef.current = ready.schemaVersion;
        const cached = cachedAppStateRef.current;
        if (cached && cached.schemaVersion === ready.schemaVersion) {
          iframeRef.current?.contentWindow?.postMessage(
            {
              source: "sessio",
              type: SESSIO_APP_STATE_RESTORE_TYPE,
              schemaVersion: cached.schemaVersion,
              state: cached.state,
            },
            "*",
          );
        }
        return;
      }

      const result = parseSessioAppStateSaveResultMessage(event.data);
      if (!result || result.requestId !== pendingStateSaveRef.current?.requestId) return;
      const snapshot = result.snapshot;
      settlePendingStateSave(
        snapshot?.schemaVersion === stateBridgeSchemaVersionRef.current ? snapshot : null,
      );
    };

    window.addEventListener("message", handleStateMessage);
    return () => {
      window.removeEventListener("message", handleStateMessage);
      if (stateProbeTimerRef.current) {
        clearInterval(stateProbeTimerRef.current);
        stateProbeTimerRef.current = null;
      }
      stateBridgeSchemaVersionRef.current = null;
      settlePendingStateSave(null);
    };
  }, [appStateBridgeEnabled, scriptsEnabled, settlePendingStateSave]);

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
          stateBridgeSchemaVersionRef.current = null;
          iframeRef.current?.contentWindow?.postMessage(
            {
              source: "sessio",
              type: SESSIO_THEME_MESSAGE_TYPE,
              theme: themeType,
              chatBackground: SESSIO_CHAT_BACKGROUND_BY_THEME[themeType],
            },
            "*",
          );
          if (appStateBridgeEnabled) {
            const probe = () =>
              iframeRef.current?.contentWindow?.postMessage(
                {
                  source: "sessio",
                  type: SESSIO_APP_STATE_PROBE_TYPE,
                },
                "*",
              );
            if (stateProbeTimerRef.current) clearInterval(stateProbeTimerRef.current);
            probe();
            let attempts = 0;
            stateProbeTimerRef.current = setInterval(() => {
              if (stateBridgeSchemaVersionRef.current !== null || ++attempts >= 50) {
                if (stateProbeTimerRef.current) {
                  clearInterval(stateProbeTimerRef.current);
                  stateProbeTimerRef.current = null;
                }
                return;
              }
              probe();
            }, 100);
          }
        }}
      />
    </div>
  );
});

export default PlainHtmlPreview;
