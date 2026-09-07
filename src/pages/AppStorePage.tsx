import { useEffect, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { AppWindow, ChevronLeft, ChevronRight, CircleCheck, Download, LoaderCircle, RefreshCw, Search, X } from "lucide-react";
import { appStoreImageUrl, fetchAppCatalog, getAppStoreStatus, type StoreApp } from "../appStore";
import Tooltip from "../components/Tooltip";
import { useI18n } from "../i18n";
import { installSessioApp, type SessioAppInfo } from "../api";

export default function AppStorePage({ localApps, onAppInstalled }: { localApps: SessioAppInfo[]; onAppInstalled: () => void }) {
  const { lang, t } = useI18n();
  const [apps, setApps] = useState<StoreApp[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(false);
  const [downloadError, setDownloadError] = useState(false);
  const [revision, setRevision] = useState(0);
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState("");
  const [installingSlug, setInstallingSlug] = useState<string | null>(null);
  const [preview, setPreview] = useState<{ src: string; alt: string } | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    setError(false);
    fetchAppCatalog(controller.signal)
      .then((result) => { if (!controller.signal.aborted) setApps(result); })
      .catch(() => { if (!controller.signal.aborted) setError(true); })
      .finally(() => { if (!controller.signal.aborted) setLoading(false); });
    return () => controller.abort();
  }, [revision]);

  const categories = [...new Set(apps.map((app) => app.category))].sort();
  const search = query.trim().toLocaleLowerCase();
  const visibleApps = apps.filter((app) => (!category || app.category === category)
    && [app.nameZh, app.nameEn, app.description, app.author, ...app.topics].join(" ").toLocaleLowerCase().includes(search));

  return (
    <section className="min-h-0 flex-1 overflow-y-auto bg-surface text-ink">
      <div className="mx-auto max-w-6xl p-6">
        <div className="mb-6 flex flex-wrap items-center gap-3">
          <h1 className="mr-auto text-xl font-semibold">{t("appStore.title")}</h1>
          <label className="flex h-9 min-w-0 items-center gap-2 rounded-md border border-ink/15 px-3">
            <Search className="h-4 w-4 shrink-0 text-ink/50" />
            <input aria-label={t("appStore.search")} placeholder={t("appStore.search")} value={query} onChange={(event) => setQuery(event.target.value)} className="w-40 min-w-0 bg-transparent text-body outline-none" />
          </label>
          <select aria-label={t("appStore.category")} value={category} onChange={(event) => setCategory(event.target.value)} className="h-9 max-w-full rounded-md border border-ink/15 bg-surface px-2 text-body">
            <option value="">{t("appStore.all")}</option>
            {categories.map((item) => <option key={item} value={item}>{item}</option>)}
          </select>
          <Tooltip content={t("appStore.refresh")}>
            <button type="button" aria-label={t("appStore.refresh")} disabled={loading} onClick={() => setRevision((value) => value + 1)} className="flex h-9 w-9 items-center justify-center rounded-md hover:bg-ink/5 disabled:opacity-40">
              <RefreshCw className="h-4 w-4" />
            </button>
          </Tooltip>
        </div>
        {downloadError && <p role="alert" className="mb-4 text-body text-red-600">{t("appStore.download_error")}</p>}
        {loading ? (
          <div role="status" className="flex items-center justify-center gap-2 py-20 text-body text-ink/55"><LoaderCircle className="h-4 w-4 animate-spin" />{t("appStore.loading")}</div>
        ) : error ? (
          <div role="alert" className="py-20 text-center text-body">
            <p className="mb-3 text-ink/60">{t("appStore.error")}</p>
            <button type="button" onClick={() => setRevision((value) => value + 1)} className="inline-flex items-center gap-2 rounded-md border border-ink/15 px-3 py-2 hover:bg-ink/5"><RefreshCw className="h-4 w-4" />{t("appStore.retry")}</button>
          </div>
        ) : visibleApps.length === 0 ? (
          <p role="status" className="py-20 text-center text-body text-ink/55">{t("appStore.empty")}</p>
        ) : (
          <div className="grid grid-cols-[repeat(auto-fit,minmax(min(100%,280px),1fr))] gap-5">
            {visibleApps.map((app) => {
              const name = (lang === "zh" ? app.nameZh : app.nameEn) || app.nameEn || app.nameZh;
              return (
                <article key={app.slug} className="flex min-w-0 flex-col overflow-hidden rounded-lg border border-ink/10">
                  <StoreScreenshots paths={app.screenshots.length > 0 ? app.screenshots : app.logoUrl ? [app.logoUrl] : []} alt={name} onPreview={setPreview} t={t} />
                  <div className="flex flex-1 flex-col gap-3 p-4">
                    <div className="min-w-0"><h2 className="break-words text-base font-semibold">{name}</h2><p className="mt-1 break-words text-caption text-ink/50">{app.category} · {app.author} · v{app.version}</p></div>
                    <p className="break-words text-body text-ink/70">{app.description}</p>
                    <div className="flex flex-wrap gap-x-3 gap-y-1 text-caption text-ink/45">{app.topics.map((topic) => <span key={topic}>{topic}</span>)}</div>
                    <StoreAction app={app} localApp={localApps.find((item) => item.slug === app.slug)} installing={installingSlug === app.slug} onInstallStart={() => { setDownloadError(false); setInstallingSlug(app.slug); }} onInstallComplete={() => { setInstallingSlug(null); onAppInstalled(); }} onDownloadError={() => { setInstallingSlug(null); setDownloadError(true); }} t={t} />
                  </div>
                </article>
              );
            })}
          </div>
        )}
      </div>
      {preview && <StoreImagePreview image={preview} onClose={() => setPreview(null)} t={t} />}
    </section>
  );
}

function StoreAction({
  app,
  localApp,
  installing,
  onInstallStart,
  onInstallComplete,
  onDownloadError,
  t,
}: {
  app: StoreApp;
  localApp?: SessioAppInfo;
  installing: boolean;
  onInstallStart: () => void;
  onInstallComplete: () => void;
  onDownloadError: () => void;
  t: (key: string) => string;
}) {
  const status = getAppStoreStatus(Boolean(localApp), localApp?.version, app.version);
  if (status === "installed") {
    return <span className="mt-auto inline-flex items-center justify-center gap-2 rounded-md border border-ink/10 px-3 py-2 text-body text-[rgb(var(--color-emerald))]"><CircleCheck className="h-4 w-4" />{t("appStore.installed")}</span>;
  }
  return (
    <a
      href={app.downloadUrl}
      target="_blank"
      rel="noopener noreferrer"
      onClick={(event) => {
        if (!isTauri()) return;
        event.preventDefault();
        if (installing) return;
        onInstallStart();
        void installSessioApp(app.slug, app.downloadUrl, status === "upgrade")
          .then(onInstallComplete)
          .catch(onDownloadError);
      }}
      aria-disabled={installing}
      className="mt-auto inline-flex items-center justify-center gap-2 rounded-md border border-ink/15 px-3 py-2 text-body hover:bg-ink/5 aria-disabled:pointer-events-none aria-disabled:opacity-50"
    >
      {installing ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <Download className="h-4 w-4" />}
      {t(installing ? "appStore.installing" : status === "upgrade" ? "appStore.upgrade" : "appStore.download")}
    </a>
  );
}

function StoreScreenshots({ paths, alt, onPreview, t }: { paths: string[]; alt: string; onPreview: (image: { src: string; alt: string }) => void; t: (key: string) => string }) {
  const [index, setIndex] = useState(0);
  useEffect(() => setIndex(0), [paths.join("\u0000")]);
  const images = paths.map(appStoreImageUrl).filter((src): src is string => Boolean(src));
  const src = images[index];
  const hasMultiple = images.length > 1;
  if (!src) return <StoreImage alt={alt} />;
  return <div className="relative flex aspect-video items-center justify-center border-b border-ink/10 bg-ink/5">
    <button type="button" className="h-full w-full cursor-zoom-in" onClick={() => onPreview({ src, alt })} aria-label={t("appStore.preview_image")}>
      <img src={src} alt={alt} loading="lazy" className="h-full w-full object-contain" />
    </button>
    {hasMultiple && <>
      <button type="button" aria-label={t("appStore.previous_image")} onClick={() => setIndex((value) => (value - 1 + images.length) % images.length)} className="absolute left-2 top-1/2 flex h-8 w-8 -translate-y-1/2 items-center justify-center rounded-full bg-black/45 text-white hover:bg-black/65"><ChevronLeft className="h-4 w-4" /></button>
      <button type="button" aria-label={t("appStore.next_image")} onClick={() => setIndex((value) => (value + 1) % images.length)} className="absolute right-2 top-1/2 flex h-8 w-8 -translate-y-1/2 items-center justify-center rounded-full bg-black/45 text-white hover:bg-black/65"><ChevronRight className="h-4 w-4" /></button>
      <span className="absolute bottom-2 left-1/2 -translate-x-1/2 rounded-full bg-black/55 px-2 py-0.5 text-caption text-white">{index + 1} / {images.length}</span>
    </>}
  </div>;
}

function StoreImage({ src, alt }: { src?: string; alt: string }) {
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [src]);
  return <div className="flex aspect-video items-center justify-center border-b border-ink/10 bg-ink/5">
    {src && !failed ? <img src={src} alt={alt} loading="lazy" onError={() => setFailed(true)} className="h-full w-full object-contain" /> : <AppWindow aria-hidden="true" className="h-10 w-10 text-ink/25" />}
  </div>;
}

function StoreImagePreview({ image, onClose, t }: { image: { src: string; alt: string }; onClose: () => void; t: (key: string) => string }) {
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => { if (event.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);
  return <div role="dialog" aria-modal="true" aria-label={t("appStore.preview_image")} className="fixed inset-0 z-50 flex items-center justify-center bg-black/75 p-6" onClick={onClose}>
    <button type="button" aria-label={t("appStore.close_preview")} onClick={onClose} className="absolute right-4 top-4 flex h-9 w-9 items-center justify-center rounded-full bg-white/15 text-white hover:bg-white/25"><X className="h-5 w-5" /></button>
    <img src={image.src} alt={image.alt} className="max-h-full max-w-full object-contain" onClick={(event) => event.stopPropagation()} />
  </div>;
}
