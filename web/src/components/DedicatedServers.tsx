// SPDX-License-Identifier: AGPL-3.0-or-later
// A game's dedicated servers, on its store page (docs/DEDICATED-SERVERS.md §3): programs players
// run on their own computer or a host so others can play. Each is its own app with its own
// releases; it has no store page, so this is where players get it, and where people hosting on
// a machine without the SLAPorSLOP app download the file and check it themselves (§5).

import { useState } from "react";
import { Link } from "react-router";

import { api, ApiError, type DedicatedServer } from "../api";
import { isDesktop, openExternal } from "../platform";
import { LabelNotes } from "./AppLabel";
import { formatBytes, InstallControls } from "./InstallControls";
import { ReportButton } from "./Report";
import { StatusBadge } from "./ui";

const FILE_FOR: Record<string, string> = {
  windows: "Windows",
  linux: "Linux",
  "linux-deb": "Linux (.deb)",
  "linux-rpm": "Linux (.rpm)",
};

/** "Windows · Linux": where a server runs. */
export function serverRunsOn(platforms: readonly string[]): string {
  return [platforms.includes("windows") && "Windows", platforms.includes("linux") && "Linux"].filter(Boolean).join(" · ");
}

export function DedicatedServers({
  servers,
  isMine,
  library,
}: {
  servers: DedicatedServer[];
  /** The game's maker: drafts show, with where to finish them. */
  isMine: boolean;
  /** Slugs in the viewer's library (the website). */
  library: string[];
}) {
  if (servers.length === 0) return null;
  return (
    <section className="panel panel-pad dedicated-servers" id="servers" aria-labelledby="servers-title">
      <h3 id="servers-title">{servers.length === 1 ? "Dedicated server" : "Dedicated servers"}</h3>
      <p className="muted small">Run your own server on your computer or a host, so others can play.</p>
      <ul className="server-list">
        {servers.map((s) => (
          <ServerRow key={s.slug} server={s} isMine={isMine} inLibrary={library.includes(s.slug)} />
        ))}
      </ul>
    </section>
  );
}

function ServerRow({ server, isMine, inLibrary }: { server: DedicatedServer; isMine: boolean; inLibrary: boolean }) {
  const release = server.release;
  const size = release?.files.reduce((n, f) => n + (f.size_bytes ?? 0), 0) ?? 0;
  return (
    <li className="server-row">
      <div className="server-head">
        <b>{server.title}</b> <StatusBadge app={server} />
      </div>
      {server.summary && <p className="muted small">{server.summary}</p>}
      <p className="small">
        {serverRunsOn(server.platforms)}
        {release && ` · version ${release.version}`}
        {size > 0 && ` · ${formatBytes(size)}`}
      </p>
      {release && !server.status && (
        <>
          <LabelNotes slug={server.slug} link={false} />
          <InstallControls slug={server.slug} title={server.title} inLibrary={inLibrary} />
          {!isDesktop && <DownloadYourself server={server} />}
        </>
      )}
      {isMine && (
        <p className="small">
          {server.status ? "Not in the store yet. " : ""}
          <Link to={`/publish/${encodeURIComponent(server.slug)}/releases`}>Releases &amp; progress</Link>
        </p>
      )}
      {!isMine && !server.status && (
        <p className="report-row">
          <ReportButton kind="app" subject={server.slug} label="this server" />
        </p>
      )}
    </li>
  );
}

/** For hosting where the SLAPorSLOP app can't run (a Linux server, say): the file, and how to check it. */
function DownloadYourself({ server }: { server: DedicatedServer }) {
  const [error, setError] = useState<string>();
  const release = server.release;
  if (!release || release.files.length === 0) return null;
  const download = async (platform: string, part?: number) => {
    setError(undefined);
    try {
      const { url } = await api.downloadLink(server.slug, release.id, platform, part);
      await openExternal(url);
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Couldn't get the download. Try again.");
    }
  };
  return (
    <details className="download-yourself">
      <summary>Download it yourself</summary>
      <p className="muted small">
        For a computer or host without the SLAPorSLOP app. Check the file's SHA-256 before you run it: <code>sha256sum</code> on
        Linux, <code>Get-FileHash</code> in PowerShell. It must match exactly.
      </p>
      <ul className="server-files">
        {release.files.map((f) => (
          <li key={f.platform}>
            {f.parts > 0 ? (
              <>
                <span className="small">
                  {FILE_FOR[f.platform] ?? f.platform}: <b>{f.file_name}</b>, in {f.parts} parts
                </span>{" "}
                {Array.from({ length: f.parts }, (_, i) => (
                  <button key={i} className="btn small" onClick={() => void download(f.platform, i + 1)}>
                    Part {i + 1}
                  </button>
                ))}
                <p className="muted small">Join the parts in order into one file, then check it.</p>
              </>
            ) : (
              <button className="btn small" onClick={() => void download(f.platform)}>
                Download for {FILE_FOR[f.platform] ?? f.platform}
                {f.size_bytes ? ` (${formatBytes(f.size_bytes)})` : ""}
              </button>
            )}
            {f.sha256 && (
              <p className="small sha">
                SHA-256 of {f.file_name}: <code>{f.sha256}</code>
              </p>
            )}
          </li>
        ))}
      </ul>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </details>
  );
}
