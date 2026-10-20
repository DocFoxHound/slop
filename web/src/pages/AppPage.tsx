import { useState, type FormEvent, type MouseEvent, type ReactNode } from "react";
import { Link, useParams } from "react-router";

import { api, type AppDetail, type AppPlatform, type Credit } from "../api";
import { engines } from "../components/Checks";
import { Gallery } from "../components/Gallery";
import { RatingPanel } from "../components/RatingPanel";
import { ReportButton } from "../components/Report";
import { Reviews } from "../components/Reviews";
import { Art, ErrorState, ExternalLink, Loading, SectionHeader, Shelf, StageBadge, StatusBadge } from "../components/ui";
import { ageLabel, daysSince, deadlineText, formatDate, STAGE_LABEL } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { renderMarkdown } from "../markdown";
import { InstallControls } from "../components/InstallControls";
import { DonateBox } from "../components/Donate";
import { InterestedButton } from "../components/Interested";
import { workflowName } from "./Releases";
import { isDesktop, openExternal } from "../platform";
import { useSession } from "../session";

export function AppPage() {
  const { slug = "" } = useParams();
  const app = useLoad(() => api.app(slug), [slug]);
  useDocumentTitle(app.data?.title);

  if (app.error) {
    return (
      <div className="page">
        <ErrorState error={app.error} onRetry={app.reload} />
      </div>
    );
  }
  if (!app.data) return <Loading />;
  return <AppView app={app.data} />;
}

function AppView({ app }: { app: AppDetail }) {
  const { me } = useSession();
  const library = useLoad(() => (me && !isDesktop ? api.library() : Promise.resolve([])), [me?.id]);
  const inLibrary = library.data?.some((e) => e.slug === app.slug);
  const players = useLoad(() => (app.kind === "game" ? api.players(app.slug) : Promise.resolve(undefined)), [app.slug]);
  const counted = players.data && (players.data.playing_now > 0 || players.data.peak_this_month > 0);
  const releases = useLoad(() => (app.stage === "coming_soon" ? Promise.resolve([]) : api.releases(app.slug)), [app.slug]);
  const latest = releases.data?.find((r) => r.status === "approved");
  const provenance = latest && latest.files.length > 0 && latest.files.every((f) => f.provenance) ? latest.files[0].provenance : null;
  const scannedWith = latest ? engines(latest.files.find((f) => f.scan_report)?.scan_report) : "";
  const windowsFile = latest?.files.find((f) => f.platform === "windows" && f.defender);
  const packages = latest?.files.filter((f) => f.platform === "linux-deb" || f.platform === "linux-rpm") ?? [];
  const upcoming = app.stage !== "released";
  const deadline = deadlineText(app);
  const age = ageLabel(app.min_age);
  const firstCategory = app.kind === "game" ? { slug: "games", name: "Games" } : null;
  const hasGallery = app.gallery.length > 0;

  // Links inside the maker's Markdown leave Slop.
  const onDescriptionClick = (e: MouseEvent) => {
    const link = (e.target as HTMLElement).closest("a[data-external]");
    if (link) {
      e.preventDefault();
      void openExternal((link as HTMLAnchorElement).href);
    }
  };

  return (
    <div className="page">
      <div className="crumbs">
        <Link to="/">Store</Link> ›{" "}
        {upcoming ? <Link to="/upcoming">Upcoming</Link> : firstCategory ? <Link to={`/browse/${firstCategory.slug}`}>{firstCategory.name}</Link> : <Link to="/search">Apps</Link>} ›{" "}
        {app.title}
      </div>

      {app.is_mine && app.status && app.status !== "listed" && (
        <div className="notice">
          <StatusBadge app={app} /> Only you can see this page.{" "}
          {app.status === "draft" && "It goes in the store once it's published and passes every scan."}{" "}
          <Link to={`/publish/${app.slug}`}>Edit draft</Link>
        </div>
      )}

      {app.private && (
        <div className="notice">
          <span className="badge neutral">PRIVATE SLOP</span>{" "}
          {app.is_mine
            ? "Only you and the people you share it with can see this. It's never in the store."
            : `${app.maker.display_name} shared this with you. Only the people it's shared with can see it.`}
        </div>
      )}
      {app.private && app.is_mine && <SharePanel slug={app.slug} />}

      {app.branch && (
        <div className="notice community-notice">
          <span className="badge neutral">COMMUNITY</span> A {app.branch.kind_label.toLowerCase()} of{" "}
          <Link to={`/app/${app.branch.parent_slug}`}>{app.branch.parent_title}</Link>, made by{" "}
          <b>{app.maker.display_name}</b>, not {app.branch.parent_maker}.
          {app.branch.note && <> {app.branch.note}</>}
        </div>
      )}

      <div className="app-head">
        {hasGallery ? <Gallery items={app.gallery} title={app.title} /> : <Art app={app} withName={false} className="hero-art" />}
        <div className="app-side">
          {hasGallery && app.thumbnail_url && <Art app={app} withName={false} className="capsule" />}
          <div>
            <div className="title-row">
              <h1>{app.title}</h1>
              <StageBadge app={app} />
              {age && <span className="badge neutral" title={`Rated ${age}`}>{age}</span>}
            </div>
            <div className="muted">
              by <b className="text">{app.maker.display_name}</b>
              {app.official_team && (
                <>
                  {" "}
                  of{" "}
                  <Link to={`/teams/${app.official_team.slug}`} className="official-team">
                    {app.official_team.name} <span className="badge official" title="A studio verified through its GitHub organization">✓ OFFICIAL</span>
                  </Link>
                </>
              )}{" "}
              · {app.kind === "game" ? "Game" : "App"}
            </div>
          </div>
          {upcoming && (
            <div className="panel panel-pad stage-panel">
              {app.is_dead ? (
                <>
                  <b className="bad">Dead project</b>
                  <div className="muted">
                    No code changes in {app.last_commit_at ? daysSince(app.last_commit_at) : "90+"} days
                  </div>
                </>
              ) : (
                <>
                  <b>{deadline}</b>
                  <div className="muted">
                    {STAGE_LABEL[app.stage]} since {formatDate(app.stage_entered_at)}
                  </div>
                </>
              )}
            </div>
          )}
          {counted && players.data && (
            <div className="playing" title="Counted by Slop Pulse: players who launched it from Slop">
              <b>{players.data.playing_now.toLocaleString()}</b> playing now ·{" "}
              {players.data.peak_this_month.toLocaleString()} peak this month
            </div>
          )}
          <p className="lede">{app.summary}</p>
          {app.credits.length > 0 && (
            <div className="credits">
              <span className="badge official" title="This app's maker hired these creatives through People">
                ✋ MADE WITH PEOPLE
              </span>
              <span className="small">
                {creditsByRole(app.credits).map(([role, people], i) => (
                  <span key={role}>
                    {i > 0 && " · "}
                    {role} by{" "}
                    {people.map((c, k) => (
                      <span key={c.handle}>
                        {k > 0 && (k === people.length - 1 ? " and " : ", ")}
                        <Link to={`/people/${c.handle}`}>{c.name}</Link>
                      </span>
                    ))}
                  </span>
                ))}
              </span>
            </div>
          )}
          <div className="chips">
            {app.tag_details.map((t) => (
              <Link className="chip" key={t.slug} to={`/tag/${t.slug}`}>
                {t.name}
              </Link>
            ))}
          </div>
          {app.is_mine && (
            <div className="row-actions">
              <Link className="btn" to={`/publish/${app.slug}`}>
                ✎ Edit store page &amp; media
              </Link>
              <Link className="btn" to={`/publish/${app.slug}/releases`}>
                ⬆ Releases
              </Link>
            </div>
          )}
          <dl className="kv">
            <dt>Source</dt>
            <dd>
              <ExternalLink href={app.repo_url}>{app.repo_url.replace(/^https:\/\//, "")}</ExternalLink>
            </dd>
            <dt>Licence</dt>
            <dd>
              <code>{app.license_spdx}</code>
            </dd>
            <dt>Stage</dt>
            <dd>{STAGE_LABEL[app.stage]}</dd>
            {app.platforms.length > 0 && (
              <>
                <dt>Runs on</dt>
                <dd>{runsOn(app.platforms)}</dd>
              </>
            )}
            {app.last_commit_at && (
              <>
                <dt>Last commit</dt>
                <dd>{formatDate(app.last_commit_at)}</dd>
              </>
            )}
          </dl>
        </div>
      </div>

      <div className="two-col">
        <div>
          {app.description_md.trim() ? (
            <div
              className="prose"
              onClick={onDescriptionClick}
              // Rendered with raw HTML disabled; see markdown.ts.
              dangerouslySetInnerHTML={{ __html: renderMarkdown(app.description_md) }}
            />
          ) : (
            <p className="muted">The maker hasn't written a description yet.</p>
          )}
          {!app.private && <RatingPanel slug={app.slug} isMine={app.is_mine} signedIn={Boolean(me)} />}
          {!app.private && <Reviews slug={app.slug} />}
        </div>
        <aside className="side-col">
          <div className="panel panel-pad install-box">
            <h3>{app.stage === "coming_soon" ? "Coming soon" : "Get it"}</h3>
            {app.stage === "coming_soon" ? (
              <p className="muted">No builds yet. This is a Coming soon page.</p>
            ) : app.branch && !app.is_mine ? (
              <CommunityGate app={app}>
                <InstallControls slug={app.slug} title={app.title} inLibrary={inLibrary} />
              </CommunityGate>
            ) : (
              <InstallControls slug={app.slug} title={app.title} inLibrary={inLibrary} />
            )}
            {!app.private && <InterestedButton app={app} isInterested={app.is_interested} isMine={app.is_mine} />}
            {packages.length > 0 && (
              <p className="muted small packages">
                Linux packages:{" "}
                {packages.map((f, i) => (
                  <span key={f.platform}>
                    {i > 0 && " · "}
                    <ExternalLink href={f.url}>{f.platform === "linux-deb" ? ".deb" : ".rpm"}</ExternalLink>
                  </span>
                ))}{" "}
                <span className="faint">(installed by your system, not by Slop)</span>
              </p>
            )}
          </div>
          <div className="panel panel-pad trust">
            {provenance && (
              <div className="trust-row">
                <span className="ic">⇄</span>
                <span>
                  <b>{provenance.built_by_slop_build ? "Built by Slop Build from source" : "Built from source"}</b> · signed by{" "}
                  {provenance.run_url ? (
                    <ExternalLink href={provenance.run_url}>{workflowName(provenance.signer_workflow)}</ExternalLink>
                  ) : (
                    workflowName(provenance.signer_workflow)
                  )}{" "}
                  at <code>{provenance.commit.slice(0, 7)}</code>
                </span>
              </div>
            )}
            {(scannedWith || windowsFile) && (
              <div className="trust-row">
                <span className="ic">⌕</span>
                <span>
                  <b>Scanned</b>
                  {scannedWith && ` with ${scannedWith}`}
                  {windowsFile?.defender && windowsFile.defender.detections.length === 0 && " · Microsoft Defender found nothing"}
                  {windowsFile?.defender?.signed_by && ` · Windows program signed by ${windowsFile.defender.signed_by}`}
                </span>
              </div>
            )}
            <div className="trust-row">
              <span className="ic">{"{ }"}</span>
              <span>
                <b>Open source</b> · {app.license_spdx} · <ExternalLink href={app.repo_url}>view code</ExternalLink>
              </span>
            </div>
            <div className="trust-row">
              <span className="ic">🛍</span>
              <span>
                <b>Nothing for sale</b> · donations never unlock anything
              </span>
            </div>
          </div>
          {!app.branch && !app.private && <CommunityBuilds app={app} />}
          {app.stage !== "coming_soon" && !app.private && <DonateBox slug={app.slug} title={app.title} />}
          {!app.is_mine && (
            <p className="report-row">
              <ReportButton kind="app" subject={app.slug} label="this app" />
            </p>
          )}
        </aside>
      </div>

      <SimilarShelves slug={app.slug} kind={app.kind} />
    </div>
  );
}

/** More like this (tags), what's popular, and something new to discover. Empty shelves are left out. */
function SimilarShelves({ slug, kind }: { slug: string; kind: AppDetail["kind"] }) {
  const similar = useLoad(() => api.similar(slug), [slug]);
  if (!similar.data) return null;
  const things = kind === "game" ? "games" : "apps";
  const shelves = [
    { title: "More like this", sub: "Shares the most tags", apps: similar.data.like_this },
    { title: "Popular right now", sub: `The most played ${things} this week`, apps: similar.data.popular },
    { title: "Discover something new", sub: `Fresh ${things} that haven't been found yet`, apps: similar.data.discover },
  ].filter((s) => s.apps.length > 0);
  return (
    <div className="similar">
      {shelves.map((s) => (
        <section key={s.title}>
          <SectionHeader title={s.title} sub={s.sub} />
          <Shelf apps={s.apps} empty={null} />
        </section>
      ))}
    </div>
  );
}

/** "Art by A and B · Music by C": each creative once per role, in the order they were credited. */
export function creditsByRole(credits: Credit[]): Array<[string, Credit[]]> {
  const roles: Array<[string, Credit[]]> = [];
  for (const c of credits) {
    let entry = roles.find(([r]) => r === c.role);
    if (!entry) roles.push((entry = [c.role, []]));
    if (!entry[1].some((x) => x.handle === c.handle)) entry[1].push(c);
  }
  return roles;
}

function runsOn(platforms: AppPlatform[]): string {
  const parts: string[] = [];
  if (platforms.includes("windows")) parts.push("Windows");
  if (platforms.includes("linux")) {
    const pkgs = [platforms.includes("linux-deb") && ".deb", platforms.includes("linux-rpm") && ".rpm"].filter(Boolean);
    parts.push(pkgs.length ? `Linux (also as ${pkgs.join(" and ")})` : "Linux");
  }
  return parts.join(" · ");
}

const PLATFORM_NAME: Record<string, string> = { windows: "Windows", linux: "Linux", "linux-deb": ".deb", "linux-rpm": ".rpm" };

/** A community build's caveats (DESIGN §9), ticked before it can be installed. */
function CommunityGate({ app, children }: { app: AppDetail; children: ReactNode }) {
  const [ok, setOk] = useState(false);
  if (!app.branch) return <>{children}</>;
  if (ok) return <>{children}</>;
  return (
    <div className="community-gate">
      <ul className="small">
        <li>
          Made by {app.maker.display_name}, not {app.branch.parent_maker}.
        </li>
        <li>Checked the same way as every app on Slop.</li>
        <li>May lag behind {app.branch.parent_title}'s official updates.</li>
        <li>
          Read <ExternalLink href={app.repo_url}>its README</ExternalLink> first.
        </li>
      </ul>
      <label className="check">
        <input type="checkbox" checked={ok} onChange={(e) => setOk(e.target.checked)} />
        <span>I understand this is a community build.</span>
      </label>
    </div>
  );
}

/** Community builds of this app: ones that fill a platform gap first. */
function CommunityBuilds({ app }: { app: AppDetail }) {
  return (
    <div className="panel panel-pad community-builds">
      <h3>Community builds</h3>
      {app.community.length === 0 ? (
        <p className="muted small">None yet. Mods, fixes, translations and ports from forks show here.</p>
      ) : (
        <ul className="plain">
          {app.community.map((b) => (
            <li key={b.slug}>
              <Link to={`/app/${b.slug}`}>
                <b>{b.title}</b>
              </Link>{" "}
              <span className="badge neutral">COMMUNITY</span>
              <div className="muted small">
                {b.branch_kind} by {b.author}
                {b.adds.length > 0 && <> · adds {b.adds.map((p) => PLATFORM_NAME[p] ?? p).join(", ")}</>}
                {b.not_slop_percent !== null && <> · {b.not_slop_percent}% Not Slop</>}
              </div>
            </li>
          ))}
        </ul>
      )}
      {!app.is_mine && (
        <Link className="small" to={`/app/${app.slug}/branch`}>
          Register a branch from your fork →
        </Link>
      )}
    </div>
  );
}

/** Who a Private Slop app is shared with (the owner's view): people, and player teams. */
function SharePanel({ slug }: { slug: string }) {
  const shares = useLoad(() => api.private.shares(slug), [slug]);
  const myTeams = useLoad(() => api.teams.mine().then((m) => m.teams.filter((t) => t.kind === "player")), []);
  const [who, setWho] = useState("");
  const [team, setTeam] = useState("");
  const [error, setError] = useState<string>();
  const run = async (action: () => Promise<unknown>) => {
    setError(undefined);
    try {
      await action();
      shares.reload();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };
  const add = async (e: FormEvent) => {
    e.preventDefault();
    await run(() => api.private.share(slug, { user: who }));
    setWho("");
  };
  const teamsLeft = (myTeams.data ?? []).filter((t) => !shares.data?.some((s) => s.team === t.slug));
  return (
    <div className="panel panel-pad share-panel">
      <h3>Shared with</h3>
      {shares.data?.length ? (
        <ul className="plain">
          {shares.data.map((s) => (
            <li key={s.user_id ?? s.team ?? s.name}>
              {s.user_id ? <Link to={`/u/${s.user_id}`}>{s.name}</Link> : <Link to={`/teams/${s.team}`}>👥 {s.name}</Link>}{" "}
              <button
                type="button"
                className="link-btn small muted"
                onClick={() =>
                  void run(() => (s.user_id ? api.private.unshare(slug, s.user_id) : api.private.unshareTeam(slug, s.team!)))
                }
              >
                Stop sharing
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <p className="muted small">Nobody yet.</p>
      )}
      <form className="row-actions" onSubmit={(e) => void add(e)}>
        <input aria-label="Their profile link" placeholder="Paste their profile link" value={who} onChange={(e) => setWho(e.target.value)} />
        <button className="btn small" disabled={!who.trim()}>
          Share
        </button>
      </form>
      {teamsLeft.length > 0 && (
        <div className="row-actions">
          <select aria-label="One of your teams" value={team} onChange={(e) => setTeam(e.target.value)}>
            <option value="">Or one of your player teams…</option>
            {teamsLeft.map((t) => (
              <option key={t.slug} value={t.slug}>
                {t.name}
              </option>
            ))}
          </select>
          <button type="button" className="btn small" disabled={!team} onClick={() => void run(() => api.private.share(slug, { team })).then(() => setTeam(""))}>
            Share with the team
          </button>
        </div>
      )}
      <p className="faint small">
        They see it once a release is approved, and download it straight from your release page through short-lived links.
        Never with official teams.
      </p>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
