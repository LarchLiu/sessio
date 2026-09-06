// @vitest-environment happy-dom

import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import PlainHtmlPreview, {
  buildPlainHtmlPreviewDocument,
  parseSessioAppFileWriteMessage,
  resolveLocalScriptPath,
  resolveAppIframePermissions,
} from "../src/components/PlainHtmlPreview";
import { I18nProvider } from "../src/i18n";

describe("buildPlainHtmlPreviewDocument", () => {
  it("applies a restrictive policy and neutralizes document navigation", () => {
    const html = buildPlainHtmlPreviewDocument(
      '<html><head><base href="https://example.com"><meta http-equiv="refresh" content="0; url=https://example.com"></head><body><a href="https://example.com">Open</a></body></html>',
      false,
    );

    expect(html).toContain("script-src 'none'");
    expect(html).toContain("connect-src 'none'");
    expect(html).not.toContain("<base");
    expect(html).not.toContain("http-equiv=\"refresh\"");
    expect(html).toContain('target="_blank"');
    expect(html).toContain('rel="noopener noreferrer"');
  });

  it("only adds inline script permission when explicitly enabled", () => {
    const disabled = buildPlainHtmlPreviewDocument("<script>document.body.textContent = 'no'</script>", false);
    const enabled = buildPlainHtmlPreviewDocument("<script>document.body.textContent = 'yes'</script>", true);

    expect(disabled).toContain("script-src 'none'");
    expect(enabled).toContain("script-src 'unsafe-inline' blob: data:");
    expect(enabled).toContain("connect-src 'none'");
  });

  it("injects the Sessio theme contract and bridge", () => {
    const disabled = buildPlainHtmlPreviewDocument("<main>Static</main>", false, "light");
    const enabled = buildPlainHtmlPreviewDocument("<main>App</main>", true, "dark");

    expect(disabled).toContain('data-sessio-theme="light"');
    expect(disabled).toContain("color-scheme: light");
    expect(disabled).toContain("--sessio-chat-background: #f6f6f4");
    expect(disabled).not.toContain("data-sessio-theme-bridge");
    expect(enabled).toContain('data-sessio-theme="dark"');
    expect(enabled).toContain("--sessio-chat-background: #232831");
    expect(enabled).toContain("data-sessio-theme-bridge");
    expect(enabled).toContain("sessio-theme-change");
    expect(enabled).toContain("sessio:themechange");
  });

  it("runs scripts without showing the JavaScript control for app previews", () => {
    const rendered = renderToStaticMarkup(
      createElement(
        I18nProvider,
        null,
        createElement(PlainHtmlPreview, {
          html: "<script>document.body.textContent = 'app'</script>",
          filePath: "/workspace/example/example.html",
          scriptsInitiallyEnabled: true,
          showScriptsControl: false,
          permissions: [
            "fullscreen",
            "downloads",
            "modals",
            "popups",
            "clipboardWrite",
            "gamepad",
            "autoplay",
            "pointerLock",
          ],
        }),
      ),
    );

    expect(rendered).not.toContain("sessio-plain-html-preview-toolbar");
    expect(rendered).toContain("unsafe-inline");
    expect(rendered).toContain(
      'allow="fullscreen; clipboard-write; gamepad; autoplay"',
    );
    expect(rendered).not.toContain("pointer-lock;");
    expect(rendered).toContain(
      'sandbox="allow-scripts allow-downloads allow-modals allow-popups allow-pointer-lock"',
    );
  });

  it("keeps the JavaScript control visible for regular HTML previews", () => {
    const rendered = renderToStaticMarkup(
      createElement(
        I18nProvider,
        null,
        createElement(PlainHtmlPreview, {
          html: "<p>document</p>",
          filePath: "/workspace/document.html",
        }),
      ),
    );

    expect(rendered).toContain("sessio-plain-html-preview-toolbar");
    expect(rendered).toContain("script-src &#x27;none&#x27;");
    expect(rendered).not.toContain("pointer-lock");
  });

  it("maps app capabilities to controlled iframe permissions only while scripts run", () => {
    expect(
      resolveAppIframePermissions(
        [
          "fullscreen",
          "downloads",
          "modals",
          "popups",
          "clipboardWrite",
          "gamepad",
          "autoplay",
          "pointerLock",
          "pointerLock",
        ],
        true,
      ),
    ).toEqual({
      allow: "fullscreen; clipboard-write; gamepad; autoplay",
      sandbox:
        "allow-scripts allow-downloads allow-modals allow-popups allow-pointer-lock",
    });
    expect(resolveAppIframePermissions(["pointerLock"], false)).toEqual({
      allow: undefined,
      sandbox: "",
    });
    expect(resolveAppIframePermissions([], true)).toEqual({
      allow: undefined,
      sandbox: "allow-scripts",
    });
  });

  it("accepts only well-formed app file-write bridge messages", () => {
    expect(
      parseSessioAppFileWriteMessage({
        source: "sessio-app",
        type: "sessio-app-write-file",
        requestId: "save-1",
        path: "exports/state.json",
        data: "{}",
      }),
    ).toEqual({
      source: "sessio-app",
      type: "sessio-app-write-file",
      requestId: "save-1",
      path: "exports/state.json",
      data: "{}",
      encoding: "utf8",
      overwrite: false,
    });
    expect(
      parseSessioAppFileWriteMessage({
        source: "untrusted",
        type: "sessio-app-write-file",
        requestId: "save-1",
        path: "state.json",
        data: "{}",
      }),
    ).toBeNull();
    expect(
      parseSessioAppFileWriteMessage({
        source: "sessio-app",
        type: "sessio-app-write-file",
        requestId: "save-1",
        path: "state.json",
        data: "{}",
        encoding: "binary",
      }),
    ).toBeNull();
  });

  it("resolves same-directory and child-directory scripts only", () => {
    const htmlPath = "/workspace/reports/case-report-trends.html";

    expect(resolveLocalScriptPath("./case-report-trends-data.js", htmlPath)).toBe(
      "/workspace/reports/case-report-trends-data.js",
    );
    expect(resolveLocalScriptPath("scripts/data.js?version=2", htmlPath)).toBe(
      "/workspace/reports/scripts/data.js",
    );
    expect(resolveLocalScriptPath("%73cripts/%64ata.js", htmlPath)).toBe(
      "/workspace/reports/scripts/data.js",
    );
    expect(resolveLocalScriptPath("../outside.js", htmlPath)).toBeNull();
    expect(resolveLocalScriptPath("https://example.com/data.js", htmlPath)).toBeNull();
  });
});
