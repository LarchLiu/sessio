// @vitest-environment happy-dom

import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import PlainHtmlPreview, {
  buildPlainHtmlPreviewDocument,
  resolveLocalScriptPath,
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
        }),
      ),
    );

    expect(rendered).not.toContain("sessio-plain-html-preview-toolbar");
    expect(rendered).toContain("unsafe-inline");
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
