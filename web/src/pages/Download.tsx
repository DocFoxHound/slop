// Get the Slop desktop app: Windows, and Linux as .deb or .rpm. Reached from the "Download" button
// in the website's top bar (not shown in the desktop app itself).

import { Link } from "react-router";

import { api } from "../api";
import { Loading } from "../components/ui";
import { useDocumentTitle, useLoad } from "../hooks";
import { detectOs, isDesktop } from "../platform";

const BUILDS = [
  {
    platform: "windows",
    os: "windows",
    title: "Windows",
    detail: "Windows 10 and 11. Runs in the tray when you close the window.",
  },
  {
    platform: "linux-deb",
    os: "linux",
    title: "Ubuntu / Debian",
    detail: "A .deb package for Ubuntu, Debian, Mint and friends.",
  },
  {
    platform: "linux-rpm",
    os: "linux",
    title: "RHEL / Rocky / Alma 10",
    detail: "An .rpm package. Needs EPEL and CRB enabled first.",
    note: "sudo dnf install epel-release && sudo dnf config-manager --set-enabled crb",
  },
] as const;

export function Download() {
  useDocumentTitle("Download Slop");
  const os = detectOs();
  const builds = useLoad(
    () =>
      Promise.all(
        BUILDS.map((b) =>
          api.desktopLatest(b.platform).then(
            (latest) => ({ ...b, ...latest }),
            () => ({ ...b, url: null, version: null }),
          ),
        ),
      ),
    [],
  );

  return (
    <div className="page narrow download-page">
      <h1 className="page-title">Get the Slop app</h1>
      <p className="muted">
        Install, update and launch everything from the store in one place. Every download is checked against Slop's
        signature before it runs, and the app keeps itself up to date.
      </p>
      {isDesktop && <p className="notice">You're already using the Slop app.</p>}
      {!builds.data && <Loading />}
      <div className="download-grid">
        {builds.data?.map((b) => (
          <section key={b.platform} className={`panel panel-pad download-card ${b.os === os ? "yours" : ""}`}>
            <h2>{b.title}</h2>
            {b.os === os && <span className="badge official">YOUR SYSTEM</span>}
            <p className="muted small">{b.detail}</p>
            {"note" in b && <code className="download-note">{b.note}</code>}
            {b.url ? (
              <>
                <a className={`btn ${b.os === os ? "slime" : ""}`} href={b.url}>
                  Download
                </a>
                <span className="muted small">Version {b.version}</span>
              </>
            ) : (
              <span className="muted small">Not available right now.</span>
            )}
          </section>
        ))}
      </div>
      <p className="muted small">
        No Mac or phone version yet. You can still browse the store and read every app's source on the{" "}
        <Link to="/">website</Link>.
      </p>
    </div>
  );
}
