import { useCallback, useEffect, useMemo, useState } from "react";
import { host, type ArticleStatus, type ArticleSummary, type Platform, type UiError, type VoiceSummary } from "../platform/host";
import { asUiError } from "../errors";
import { t } from "../i18n";
import type { MessageKey } from "../i18n/en";
import { useQuietLoad, peek, prime, profileCacheKey, VOICES_KEY } from "../hooks/useQuietLoad";
import { ErrorNote } from "./ErrorNote";

type Props = {
  profileId: string;
  filter: ArticleStatus | "all";
  onOpen: (id: string) => void;
  onGoVoices: () => void;
  onGoTopics: () => void;
  // The sidebar's 「題から書く」 steps back while this screen carries its own
  // main button, so only one of them is mint at a time (D-68, principle 3).
  onMainPrimary: (v: boolean) => void;
  onShowEngineSteps: () => void;
};
// When the whole queue stops, the sentence says what to do next, not what
// went wrong: the sidebar already carries the cause.
const STOPPED: Record<string, MessageKey> = {
  codex_not_logged_in: "queue.stopped.codex_not_logged_in",
  claude_not_logged_in: "queue.stopped.claude_not_logged_in",
  not_signed_in: "queue.stopped.not_signed_in",
  session_expired: "queue.stopped.session_expired",
  network: "queue.stopped.network",
};

// The home screen: what has been written, newest first. Nothing on it is a
// number for its own sake; dates and destinations only.
export function ArticleList({ profileId, filter, onOpen, onGoVoices, onGoTopics, onMainPrimary, onShowEngineSteps }: Props) {
  const [platforms, setPlatforms] = useState<Platform[]>([]);
  const [q, setQ] = useState("");
  const [stopped, setStopped] = useState<UiError | null>(null);
  const [actionError, setActionError] = useState<UiError | null>(null);

  // Moving between 「すべての記事」 and 下書き must not blank the page either:
  // the wider list already holds those rows, so the narrower one starts from
  // them and the fetch only confirms it.
  const key = profileCacheKey(profileId, `articles:${filter}`);
  useMemo(() => {
    if (filter === "all" || peek(key)) return;
    const all = peek<ArticleSummary[]>(profileCacheKey(profileId, "articles:all"));
    if (all) prime(key, all.filter((a) => a.status === filter));
  }, [key, filter]);

  // The rows stay on screen while the same filter is fetched again; only a
  // first-ever load leaves the page blank, and only past 300 ms does it say
  // so (see useQuietLoad).
  const list = useQuietLoad<ArticleSummary[]>(
    key,
    useCallback(() => host.listArticles(filter === "all" ? undefined : filter), [filter]),
  );
  const voices = useQuietLoad<VoiceSummary[]>(profileCacheKey(profileId, VOICES_KEY), useCallback(() => host.listVoices(), []));
  const reload = list.reload;

  useEffect(() => { void host.platforms().then(setPlatforms).catch(() => {}); }, []);


  useEffect(() => {
    let live = true;
    let off: (() => void) | undefined;
    void host.onQueueEvent((e) => {
      if (!live) return;
      if (e.kind === "stopped") { setStopped(e.error); void reload().catch(() => {}); return; }
      setStopped(null);
      void reload().catch(() => {});
    }).then((u) => { if (live) off = u; else u(); }).catch(() => {});
    return () => { live = false; off?.(); };
  }, [reload]);

  const platformName = (id: string) => platforms.find((p) => p.id === id)?.name ?? id;
  const voiceName = (id: string | null) => voices.value?.find((v) => v.id === id)?.name ?? "";
  // "No voice yet" is a fact, not a guess: it waits for the fetch.
  const noVoice = voices.settled && voices.value?.length === 0;
  const needle = q.trim().toLowerCase();
  const all = list.value ?? [];
  const rows = all.filter((r) => !needle || (r.title + " " + r.excerpt).toLowerCase().includes(needle));
  const nothingAtAll = list.settled && all.length === 0;
  const inQueue = all.filter((r) => r.queue === "waiting" || r.queue === "writing").length;
  const error = actionError ?? list.error;

  // Only one mint button at a time: while this screen is empty it carries
  // the main button, so the sidebar's steps back (D-68).
  useEffect(() => { onMainPrimary(nothingAtAll); return () => onMainPrimary(false); }, [nothingAtAll, onMainPrimary]);

  const drop = async (id: string) => { try { setActionError(null); await host.dequeueArticle(id); await reload(); } catch (e) { setActionError(asUiError(e)); } };
  const clear = async () => { try { setActionError(null); await host.clearQueue(); await reload(); } catch (e) { setActionError(asUiError(e)); } };

  return (
    <div className="articles">
      <div className="page-head">
        <h2>{t(`article.filter.${filter}` as const)}</h2>
        <div className="head-right">
          {/* The search keeps its place from the first paint, so the heading
              row never shifts when the rows arrive. The way into topics is
              no longer here: it lives in the sidebar, above the blank page
              (D-68). */}
          <input className={`search${nothingAtAll ? " hidden-keep" : ""}`} type="search" value={q} onChange={(e) => setQ(e.target.value)} placeholder={t("article.search")} aria-label={t("article.search")} tabIndex={nothingAtAll ? -1 : undefined} />
        </div>
      </div>

      {stopped && (
        <p className="notice" role="status">
          {t(STOPPED[stopped.code] ?? "queue.stopped")}
          {(stopped.code === "codex_not_logged_in" || stopped.code === "claude_not_logged_in") && <button className="link" onClick={onShowEngineSteps}>{t("action.seeSteps")}</button>}
        </p>
      )}
      {/* While it is stopped, one sentence is enough; the rows still say
          which of them are waiting. */}
      {inQueue > 0 && !stopped && (
        <p className="saved" role="status">
          {t("queue.queued", { n: String(inQueue) })}
          <button className="link" onClick={() => void clear()}>{t("queue.clear")}</button>
        </p>
      )}

      {list.slow && <p className="muted">{t("article.loading")}</p>}
      {error && <ErrorNote error={error} />}

      {nothingAtAll && (
        <div className="empty">
          {noVoice ? (
            <>
              <p className="lead-sm">{t("article.emptyNoVoice")}</p>
              <button className="btn pri" onClick={onGoVoices}>{t("article.goVoices")}</button>
            </>
          ) : (
            <>
              {/* The one next step is writing from a title; the blank page
                  is gone (D-69). */}
              <p className="lead-sm">{filter === "all" ? t("article.emptyAll") : t("article.emptyFilter")}</p>
              {filter === "all" && <button className="btn pri" onClick={onGoTopics}>{t("topics.entry")}</button>}
            </>
          )}
        </div>
      )}

      {list.settled && !nothingAtAll && (
        rows.length === 0 ? <p className="muted">{t("article.noMatch")}</p> : (
          <ul className="article-rows">
            {rows.map((a) => (
              <li key={a.id} className="arow">
                <button className="article-row" onClick={() => onOpen(a.id)}>
                  <span className="a-title">{a.title.trim() || a.excerpt || t("article.untitled")}</span>
                  {a.title.trim() && a.excerpt && <span className="a-excerpt">{a.excerpt}</span>}
                  <span className="a-meta">
                    <span className="badge">{platformName(a.platform_id)}</span>
                    {a.status !== "draft" && <span className={`badge status-${a.status}`}>{t(`article.status.${a.status}` as const)}</span>}
                    {a.queue === "writing" && <span className="badge now"><span className="spinner" aria-hidden="true" />{t("queue.writing")}</span>}
                    {a.queue === "waiting" && <span className="badge status-archived">{t("queue.waiting")}</span>}
                    {a.queue === "failed" && <span className="badge status-archived">{t("queue.failed")}</span>}
                    {voiceName(a.voice_id) && <span>{voiceName(a.voice_id)}</span>}
                    <span>{relativeDate(a.updated_at)}</span>
                  </span>
                </button>
                {(a.queue === "writing" || a.queue === "waiting") && (
                  <span className="arow-act">
                    <button className="link quiet-link" onClick={() => void drop(a.id)}>{t(a.queue === "writing" ? "queue.stop" : "queue.drop")}</button>
                  </span>
                )}
              </li>
            ))}
          </ul>
        )
      )}
    </div>
  );
}

export function relativeDate(iso: string): string {
  const d = new Date(iso);
  const mins = Math.round((Date.now() - d.getTime()) / 60000);
  if (mins < 1) return t("time.justNow");
  if (mins < 60) return t("time.minutesAgo", { n: String(mins) });
  const hours = Math.round(mins / 60);
  if (hours < 24) return t("time.hoursAgo", { n: String(hours) });
  const days = Math.round(hours / 24);
  if (days < 7) return t("time.daysAgo", { n: String(days) });
  return d.toLocaleDateString();
}
