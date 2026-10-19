import { Link } from "react-router";

import { api } from "../api";
import { Sidebar } from "../components/Layout";
import { ErrorState, SectionHeader, Shelf } from "../components/ui";
import { useDocumentTitle, useLoad } from "../hooks";
import { isDesktop } from "../platform";

/** Upcoming apps that are still alive, soonest deadline first. Dead projects are never promoted. */
async function upcoming(limit: number) {
  const [soon, beta] = await Promise.all([
    api.apps({ stage: "coming_soon", limit }),
    api.apps({ stage: "beta", limit }),
  ]);
  return [...soon.apps, ...beta.apps]
    .filter((a) => !a.is_dead)
    .sort((a, b) => (a.stage_deadline ?? "").localeCompare(b.stage_deadline ?? ""))
    .slice(0, limit);
}

export function Store() {
  useDocumentTitle(undefined);
  const fresh = useLoad(() => api.apps({ stage: "released", sort: "recent", limit: 8 }), []);
  const soon = useLoad(() => upcoming(4), []);
  const categories = useLoad(() => api.categories(), []);
  const server = useLoad(() => api.server(), []);
  const downloads = useLoad(async () => {
    if (isDesktop) return [];
    const platforms = [
      { platform: "windows", label: "Windows" },
      { platform: "linux-deb", label: "Ubuntu / Debian (.deb)" },
      { platform: "linux-rpm", label: "RHEL / Rocky / Alma 10 (.rpm)" },
    ] as const;
    const found = await Promise.all(
      platforms.map((p) => api.desktopLatest(p.platform).then((b) => ({ ...p, ...b }), () => null)),
    );
    return found.filter((d) => d !== null);
  }, []);

  return (
    <div className="page">
      <div className="store">
        <Sidebar />
        <div>
          {server.data && !server.data.scanning && (
            <p className="notice" role="note">
              <b>Test server.</b> Automatic scanning isn't running here yet, so uploads are not scanned.
            </p>
          )}
          <section className="intro panel">
            <h1>
              Apps and games made with AI. <span className="slime">Open source, every one.</span>
            </h1>
            <ul>
              <li>
                <b>Source you can read.</b> Every app links its code, under an open-source licence.
              </li>
              {server.data?.scanning ? (
                <li>
                  <b>Scanned before it's listed.</b> Malware, stolen code and scams are checked on every upload.
                </li>
              ) : (
                <li>
                  <b>Reviewed before it's listed.</b>{" "}
                  {server.data?.maker_signatures_required
                    ? "Every build is signed by its maker and checked by a person."
                    : "Every build is checked by a person."}
                </li>
              )}
              <li>
                <b>Free to download.</b> Makers can take donations. Nothing is ever for sale.
              </li>
            </ul>
            {downloads.data && downloads.data.length > 0 && (
              <div className="get-app">
                <div className="row-actions">
                  {downloads.data.map((d, i) => (
                    <a key={d.platform} className={`btn ${i === 0 ? "slime" : ""}`} href={d.url}>
                      Download for {d.label}
                    </a>
                  ))}
                </div>
                <p className="muted small">
                  Version {downloads.data[0].version}. The app keeps itself up to date.
                  {downloads.data.some((d) => d.platform === "linux-rpm") &&
                    " The .rpm needs EPEL and CRB enabled (sudo dnf install epel-release && sudo dnf config-manager --set-enabled crb); RHEL 9 isn't supported."}
                </p>
              </div>
            )}
          </section>

          <SectionHeader title="Coming soon & in beta" sub="Projects on their way to release">
            <Link to="/upcoming" className="sub">
              See all →
            </Link>
          </SectionHeader>
          {soon.error ? (
            <ErrorState error={soon.error} onRetry={soon.reload} />
          ) : (
            <Shelf apps={soon.data} empty="Nothing upcoming right now." />
          )}

          <SectionHeader title="Browse by category">
            <Link to="/browse" className="sub">
              All tags →
            </Link>
          </SectionHeader>
          <div className="cat-grid">
            {categories.data?.map((c) => (
              <Link className="cat" key={c.slug} to={`/browse/${c.slug}`}>
                <b>{c.name}</b>
                <span>{c.description}</span>
              </Link>
            ))}
          </div>

          <SectionHeader title="New & updated" />
          {fresh.error ? (
            <ErrorState error={fresh.error} onRetry={fresh.reload} />
          ) : (
            <Shelf
              apps={fresh.data?.apps}
              empty={
                <>
                  Nothing released yet. <Link to="/publish">Be the first to publish</Link>.
                </>
              }
            />
          )}
        </div>
      </div>
    </div>
  );
}
