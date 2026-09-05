import { useEffect, useState } from "react";
import { readLocalTextFile } from "../api";
import { useI18n } from "../i18n";
import SwitchControl from "./SwitchControl";

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

export function buildPlainHtmlPreviewDocument(html: string, scriptsEnabled: boolean): string {
  const document = new DOMParser().parseFromString(html, "text/html");

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
}: {
  html: string;
  filePath: string | null;
  scriptsInitiallyEnabled?: boolean;
  showScriptsControl?: boolean;
}) {
  const { t } = useI18n();
  const [scriptPermission, setScriptPermission] = useState<{
    filePath: string | null;
    enabled: boolean;
  }>({ filePath, enabled: scriptsInitiallyEnabled });
  const scriptsEnabled =
    scriptPermission.filePath === filePath && scriptPermission.enabled;
  const [previewDocument, setPreviewDocument] = useState(() =>
    buildPlainHtmlPreviewDocument(html, scriptsEnabled),
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
      if (active) setPreviewDocument(buildPlainHtmlPreviewDocument(source, scriptsEnabled));
    };
    void prepare();
    return () => {
      active = false;
    };
  }, [filePath, html, scriptsEnabled]);

  const scriptsLabel = t("chat.files.html_preview_scripts");

  return (
    <div className="sessio-plain-html-preview min-h-0 flex-1">
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
        key={scriptsEnabled ? "scripts-enabled" : "scripts-disabled"}
        title={t("chat.files.html_preview_title")}
        className="sessio-plain-html-preview-frame"
        referrerPolicy="no-referrer"
        sandbox={scriptsEnabled ? "allow-scripts" : ""}
        srcDoc={previewDocument}
      />
    </div>
  );
}
