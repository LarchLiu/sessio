// @vitest-environment happy-dom

import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import PlainHtmlPreview, {
  buildPlainHtmlPreviewDocument,
  parseSessioAppFileWriteMessage,
  parseSessioAppStateReadyMessage,
  parseSessioAppStateSaveResultMessage,
  resolveLocalScriptPath,
  resolveAppIframePermissions,
  SESSIO_PREVIEW_BRIDGE_SCRIPT,
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
    expect(html).toContain("style-src 'unsafe-inline' https://fonts.googleapis.com");
    expect(html).toContain("font-src data: https://fonts.gstatic.com");
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

  it("adds an App-scoped resource base and CSP source when requested", () => {
    const document = buildPlainHtmlPreviewDocument(
      '<html><head><base href="https://untrusted.example/"><link rel="stylesheet" href="assets/app.css"></head><body></body></html>',
      true,
      "dark",
      "sessio-app://localhost/grant-1/",
    );

    expect(document).toContain('<base href="sessio-app://localhost/grant-1/">');
    expect(document).toContain("style-src 'unsafe-inline' https://fonts.googleapis.com sessio-app:");
    expect(document).toContain("connect-src sessio-app:");
    expect(document).toContain("base-uri sessio-app:");
    expect(document).not.toContain("https://untrusted.example");
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
    expect(enabled).toContain("SESSIO_APP_STATE");
    expect(enabled).toContain("sessio-app-state-save-request");
    expect(enabled).toContain("sessio-app-state-restore");
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

  it("accepts only versioned and bounded App state bridge messages", () => {
    expect(
      parseSessioAppStateReadyMessage({
        source: "sessio-app",
        type: "sessio-app-state-ready",
        schemaVersion: 2,
      }),
    ).toEqual({ schemaVersion: 2 });
    expect(
      parseSessioAppStateReadyMessage({
        source: "sessio-app",
        type: "sessio-app-state-ready",
        schemaVersion: 0,
      }),
    ).toBeNull();

    expect(
      parseSessioAppStateSaveResultMessage({
        source: "sessio-app",
        type: "sessio-app-state-save-result",
        requestId: "state-1",
        ok: true,
        schemaVersion: 1,
        state: { moves: [{ row: 7, col: 7 }] },
      }),
    ).toEqual({
      requestId: "state-1",
      snapshot: {
        schemaVersion: 1,
        state: { moves: [{ row: 7, col: 7 }] },
      },
    });
    expect(
      parseSessioAppStateSaveResultMessage({
        source: "sessio-app",
        type: "sessio-app-state-save-result",
        requestId: "state-2",
        ok: false,
        error: "capture failed",
      }),
    ).toEqual({ requestId: "state-2", snapshot: null });
    expect(
      parseSessioAppStateSaveResultMessage({
        source: "sessio-app",
        type: "sessio-app-state-save-result",
        requestId: "state-3",
        ok: true,
        schemaVersion: 1,
        state: "x".repeat(1024 * 1024 + 1),
      }),
    ).toBeNull();
    expect(
      parseSessioAppStateSaveResultMessage({
        source: "sessio-app",
        type: "sessio-app-state-save-result",
        requestId: "state-4",
        ok: true,
        schemaVersion: 1,
        state: { unsupported: new Map([["key", "value"]]) },
      }),
    ).toBeNull();
  });

  it("captures and restores through the injected App state adapter", async () => {
    const sent: unknown[] = [];
    type MessageHandler = (event: { source: unknown; data: unknown }) => void;
    let handleMessage: MessageHandler = () => {
      throw new Error("The preview bridge did not register its message handler");
    };
    let restored: unknown = null;
    const parent = {
      postMessage(message: unknown) {
        sent.push(message);
      },
    };
    const fakeWindow = {
      parent,
      SESSIO_APP_STATE: {
        schemaVersion: 1,
        capture: () => ({ count: 3 }),
        restore: (state: unknown) => {
          restored = state;
        },
      },
      addEventListener(type: string, listener: MessageHandler) {
        if (type === "message") handleMessage = listener;
      },
      dispatchEvent() {},
    };

    new Function("window", "document", SESSIO_PREVIEW_BRIDGE_SCRIPT)(
      fakeWindow,
      {},
    );
    handleMessage({
      source: parent,
      data: { source: "sessio", type: "sessio-app-state-probe" },
    });
    expect(sent).toContainEqual({
      source: "sessio-app",
      type: "sessio-app-state-ready",
      schemaVersion: 1,
    });

    handleMessage({
      source: parent,
      data: {
        source: "sessio",
        type: "sessio-app-state-restore",
        schemaVersion: 1,
        state: { count: 2 },
      },
    });
    expect(restored).toEqual({ count: 2 });

    handleMessage({
      source: parent,
      data: {
        source: "sessio",
        type: "sessio-app-state-save-request",
        requestId: "state-1",
      },
    });
    await Promise.resolve();
    await Promise.resolve();
    expect(sent).toContainEqual({
      source: "sessio-app",
      type: "sessio-app-state-save-result",
      requestId: "state-1",
      ok: true,
      schemaVersion: 1,
      state: { count: 3 },
    });
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
