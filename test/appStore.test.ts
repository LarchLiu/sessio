import { describe, expect, it } from "vitest";
import { APP_STORE_BASE_URL, appStoreImageUrl, getAppStoreStatus } from "../src/appStore";

describe("app store", () => {
  it("resolves catalog image paths against the store base URL", () => {
    expect(appStoreImageUrl("generated/apps/example/screenshot.png")).toBe(
      `${APP_STORE_BASE_URL}generated/apps/example/screenshot.png`,
    );
  });

  it("distinguishes downloads, upgrades, and installed apps", () => {
    expect(getAppStoreStatus(false, undefined, "1.0.0")).toBe("download");
    expect(getAppStoreStatus(true, "1.3.0", "1.4.0")).toBe("upgrade");
    expect(getAppStoreStatus(true, "1.4.0", "1.4.0")).toBe("installed");
    expect(getAppStoreStatus(true, "1.5.0", "1.4.0")).toBe("installed");
    expect(getAppStoreStatus(true, null, "1.4.0")).toBe("installed");
  });
});
