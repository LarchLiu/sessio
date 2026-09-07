import { z } from "zod";

export const APP_STORE_BASE_URL = "https://larchliu.github.io/sessio-web/";
export const APP_STORE_CATALOG_URL = new URL("generated/catalog.json", APP_STORE_BASE_URL).href;

const httpsUrl = z.string().url().refine((value) => new URL(value).protocol === "https:");
const appSchema = z.object({
  slug: z.string().min(1),
  nameZh: z.string(),
  nameEn: z.string(),
  description: z.string(),
  author: z.string(),
  version: z.string(),
  category: z.string(),
  topics: z.array(z.string()).default([]),
  logoUrl: z.string().nullable().optional(),
  screenshots: z.array(z.string()).default([]),
  downloadUrl: httpsUrl,
});
export const appCatalogSchema = z.object({ schemaVersion: z.literal(1), apps: z.array(appSchema) });
export type StoreApp = z.infer<typeof appSchema>;
export type AppStoreStatus = "download" | "upgrade" | "installed";

export function appStoreImageUrl(path: string | null | undefined): string | undefined {
  if (!path) return undefined;
  try {
    const url = new URL(path, APP_STORE_BASE_URL);
    return url.protocol === "https:" ? url.href : undefined;
  } catch {
    return undefined;
  }
}

export async function fetchAppCatalog(signal: AbortSignal): Promise<StoreApp[]> {
  const response = await fetch(APP_STORE_CATALOG_URL, { signal });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  return appCatalogSchema.parse(await response.json()).apps;
}

export function getAppStoreStatus(
  installed: boolean,
  localVersion: string | null | undefined,
  storeVersion: string,
): AppStoreStatus {
  if (!installed) return "download";
  if (!localVersion) return "installed";
  return compareVersions(localVersion, storeVersion) < 0 ? "upgrade" : "installed";
}

function compareVersions(left: string, right: string): number {
  const parse = (value: string) =>
    value
      .replace(/^v/i, "")
      .split(/[.+-]/)
      .map((part) => Number.parseInt(part, 10) || 0);
  const a = parse(left);
  const b = parse(right);
  for (let index = 0; index < Math.max(a.length, b.length); index += 1) {
    if ((a[index] ?? 0) !== (b[index] ?? 0)) {
      return (a[index] ?? 0) - (b[index] ?? 0);
    }
  }
  return 0;
}
