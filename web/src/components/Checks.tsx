// What Slop's checks found on a release file (docs/DESIGN.md §10, crates/slop-api/src/scan.rs):
// Slop's scanner (ClamAV, YARA, capa), Microsoft Defender for Windows builds, the program's
// signature, Slop Build's source checks, and Slop's own source check and dependency watch. The wording says "scanned" and what was found,
// never "safe" (docs/LEGAL.md §12).

import { Link } from "react-router";

import type {
  Advisory,
  DefenderCheck,
  ReleaseFile,
  RepoScan,
  ScanReport,
  SourceChecks,
} from "../api";
import { formatDate } from "../format";
import { ExternalLink } from "./ui";

const ENGINE_NAMES: Record<string, string> = {
  clamav: "ClamAV",
  yara: "YARA",
  capa: "capa",
};

export function engines(report: ScanReport | null | undefined): string {
  return Object.keys(report?.engines ?? {})
    .map((k) => ENGINE_NAMES[k] ?? k)
    .join(", ");
}

/** The Defender line: checked clean, flagged, or not checked yet. */
export function DefenderLine({
  defender,
  audience,
}: {
  defender: DefenderCheck | null;
  audience: "maker" | "reviewer" | "public";
}) {
  if (!defender) {
    if (audience === "public") return null;
    return (
      <div className="small">
        <span className="badge warn">DEFENDER: NOT CHECKED YET</span>{" "}
        <span className="muted">
          {audience === "maker" ? (
            <>
              Windows builds need Microsoft Defender's check before they're
              published. Slop Build does it; if you build elsewhere, add{" "}
              <Link to="/publish/guide#slop-check">Slop Check</Link> to your
              repository.
            </>
          ) : (
            "No Defender check has arrived for this file (Slop Build or Slop Check)."
          )}
        </span>
      </div>
    );
  }
  const flagged = defender.detections.length > 0;
  return (
    <div className="small">
      <span className={`badge ${flagged ? "bad" : "official"}`}>
        {flagged
          ? "MICROSOFT DEFENDER FLAGS IT"
          : "MICROSOFT DEFENDER: NOTHING FOUND"}
      </span>{" "}
      <span className="muted">
        {flagged
          ? defender.detections
              .map((d) => `${d.threat} in ${d.file}`)
              .join("; ")
          : `definitions ${defender.signatures}${defender.signatures_updated ? ` of ${formatDate(defender.signatures_updated)}` : ""}`}
      </span>
      {flagged && audience === "maker" && (
        <p className="muted small">
          If it's a false positive, submit the file to Microsoft as a software
          developer at{" "}
          <ExternalLink href="https://www.microsoft.com/en-us/wdsi/filesubmission">
            Microsoft's submission page
          </ExternalLink>
          , then build again once they've updated their definitions. Signing the
          program (<Link to="/publish/guide#signing">free with SignPath</Link>)
          helps most. A reviewer can also clear it if they agree it's a false
          positive.
        </p>
      )}
    </div>
  );
}

/** Whether the Windows program is signed, and by whom. */
export function SignedLine({
  defender,
  audience,
}: {
  defender: DefenderCheck | null;
  audience: "maker" | "reviewer" | "public";
}) {
  if (!defender) return null;
  if (defender.signed_by) {
    return (
      <div className="small">
        <span className="badge official">SIGNED</span>{" "}
        <span className="muted">by {defender.signed_by}</span>
      </div>
    );
  }
  if (audience === "public") return null;
  return (
    <div className="small">
      <span className="badge neutral">NOT CODE-SIGNED</span>{" "}
      <span className="muted">
        Windows trusts signed programs more.{" "}
        {audience === "maker" && (
          <>
            Open-source projects can sign for free:{" "}
            <Link to="/publish/guide#signing">how</Link>.
          </>
        )}
      </span>
    </div>
  );
}

/** Findings and capabilities from Slop's scanner, for the reviewer. */
export function ScanFindings({ report }: { report: ScanReport | null }) {
  if (!report) return null;
  return (
    <>
      {report.findings.length > 0 && (
        <ul className="small findings">
          {report.findings.map((f, i) => (
            <li key={i}>
              <b>{f.engine}</b>: {f.name}{" "}
              <span className="muted">in {f.file}</span>
            </li>
          ))}
        </ul>
      )}
      {report.capabilities.length > 0 && (
        <details className="small">
          <summary>
            What the programs can do, per capa ({report.capabilities.length})
          </summary>
          <ul>
            {report.capabilities.map((c, i) => (
              <li key={i}>
                {c.capability}{" "}
                <span className="muted">
                  ({c.namespace}, {c.file})
                </span>
              </li>
            ))}
          </ul>
        </details>
      )}
    </>
  );
}

/** Slop Build's source checks: possible secrets (by file and line) and vulnerable dependencies. */
export function SourceCheckList({
  checks,
  audience,
}: {
  checks: SourceChecks | null | undefined;
  audience: "maker" | "reviewer";
}) {
  if (!checks) return null;
  return (
    <div className="small source-checks">
      {checks.secrets.length > 0 ? (
        <div>
          <span className="badge warn">
            POSSIBLE SECRETS IN THE SOURCE ({checks.secrets.length})
          </span>{" "}
          <span className="muted">
            {audience === "maker" &&
              "If any of these is a real key or password, revoke it where it was issued (deleting it from the repository isn't enough), then remove it. "}
            {checks.secrets
              .slice(0, 5)
              .map((s) => `${s.file}:${s.line} (${s.rule})`)
              .join(", ")}
            {checks.secrets.length > 5 && " …"}
          </span>
        </div>
      ) : (
        <div>
          <span className="badge official">NO SECRETS FOUND IN THE SOURCE</span>{" "}
          <span className="muted">gitleaks</span>
        </div>
      )}
      {checks.vulnerabilities.length > 0 ? (
        <details>
          <summary>
            <span className="badge warn">
              DEPENDENCIES WITH KNOWN VULNERABILITIES (
              {checks.vulnerabilities.length})
            </span>
          </summary>
          <ul>
            {checks.vulnerabilities.slice(0, 30).map((v, i) => (
              <li key={i}>
                {v.package} {v.version} ({v.ecosystem}): {osvLink(v.id)}{" "}
                {v.summary}
              </li>
            ))}
          </ul>
        </details>
      ) : (
        <div>
          <span className="badge official">
            NO KNOWN-VULNERABLE DEPENDENCIES
          </span>{" "}
          <span className="muted">OSV-Scanner</span>
        </div>
      )}
    </div>
  );
}

function osvLink(id: string) {
  return (
    <ExternalLink
      href={`https://osv.dev/vulnerability/${encodeURIComponent(id)}`}
    >
      {id}
    </ExternalLink>
  );
}

/**
 * Slop's own source check at the release commit: whether the licence files match the store page,
 * signs of purchases or loot boxes (apps on Slop can't sell anything), and the dependency watch's
 * advisories. Notes for the maker and the reviewer; nothing here holds a release by itself.
 */
export function RepoScanList({
  scan,
  advisories,
  audience,
}: {
  scan: RepoScan | null | undefined;
  advisories: Advisory[] | undefined;
  audience: "maker" | "reviewer";
}) {
  if (!scan) {
    return (
      <div className="small muted">
        Slop hasn't read the source yet; it checks within a few minutes.
      </div>
    );
  }
  if (scan.status !== "checked") {
    return (
      <div className="small">
        <span className="badge neutral">SOURCE NOT CHECKED</span>{" "}
        <span className="muted">{scan.detail}</span>
      </div>
    );
  }
  const licence = scan.licence;
  const money = scan.monetization ?? [];
  const code = scan.code_loading ?? [];
  const network = scan.network ?? [];
  const found = advisories ?? [];
  const malicious = found.filter((a) => a.malicious);
  const since = found.filter((a) => !a.malicious && !a.known_at_build);
  return (
    <div className="small source-checks">
      {licence && licence.problems.length === 0 ? (
        <div>
          <span className="badge official">LICENCE FILE MATCHES</span>{" "}
          <span className="muted">
            {licence.declared}, in {licence.files.map((f) => f.path).join(", ")}
          </span>
        </div>
      ) : (
        licence && (
          <div>
            <span className="badge warn">
              LICENCE NOTES ({licence.problems.length})
            </span>
            <ul>
              {licence.problems.map((p, i) => (
                <li key={i}>{p}</li>
              ))}
            </ul>
          </div>
        )
      )}
      {money.length > 0 ? (
        <div>
          <span className="badge warn">
            POSSIBLE PURCHASES OR LOOT BOXES ({money.length})
          </span>{" "}
          <span className="muted">
            {audience === "maker"
              ? "Apps on Slop can't sell anything: no items, currencies, loot boxes or paid access. A reviewer looks at each; if one isn't a purchase (a word in a comment, say), nothing needs changing."
              : "Patterns only: open the files and decide. Selling anything in the app breaks the Publisher Agreement."}
          </span>
          <ul>
            {money.map((h) => (
              <li key={h.rule}>
                {h.label} ({h.count}):{" "}
                <span className="muted">{h.examples.join(", ")}</span>
              </li>
            ))}
          </ul>
        </div>
      ) : (
        <div>
          <span className="badge official">
            NO SIGNS OF PURCHASES IN THE SOURCE
          </span>{" "}
          <span className="muted">
            payment libraries, in-app purchase APIs, checkout links, loot boxes
          </span>
        </div>
      )}
      {code.length > 0 ? (
        <div>
          <span className="badge warn">
            CODE FROM OUTSIDE SLOP? ({code.length})
          </span>{" "}
          <span className="muted">
            {audience === "maker"
              ? "Apps on Slop only run code that came in a release or mod Slop checked, never programs, scripts or mods from a server. Data from servers must use formats that can't rebuild objects. A reviewer looks at each; loading your own files is fine. See \"Online play and mods\" in the publishing guide."
              : "Patterns only: open the files and decide. Running code a server sends, a self-updater, or a mod loader outside Slop's mods breaks the Content Policy; an unsafe format reading server data is a reason to reject until it's fixed."}
          </span>
          <ul>
            {code.map((h) => (
              <li key={h.rule}>
                {h.label} ({h.count}):{" "}
                <span className="muted">{h.examples.join(", ")}</span>
              </li>
            ))}
          </ul>
        </div>
      ) : (
        <div>
          <span className="badge official">
            NO SIGNS OF OUTSIDE CODE IN THE SOURCE
          </span>{" "}
          <span className="muted">
            code loading, self-updaters, unsafe data formats
          </span>
        </div>
      )}
      {network.length > 0 && (
        <div>
          {scan.online_undeclared ? (
            <span className="badge warn">GOES ONLINE, NOT ON THE STORE PAGE</span>
          ) : (
            <span className="badge neutral">GOES ONLINE</span>
          )}{" "}
          <span className="muted">
            {scan.online_undeclared
              ? audience === "maker"
                ? "Your code connects to the network, but your store page doesn't say it plays online. Tick what it connects to under Online on Edit store page, so players know about servers Slop can't check."
                : "The store page doesn't declare online play. Ask the maker to tick what it connects to, or reject it as a misleading page if it's hidden on purpose."
              : "Declared on the store page; players see a notice."}
          </span>
          <ul>
            {network.map((h) => (
              <li key={h.rule}>
                {h.label} ({h.count}):{" "}
                <span className="muted">{h.examples.join(", ")}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
      <SaveFolderCheck scan={scan} audience={audience} />
      {(scan.design ?? []).length > 0 && (
        <div>
          <span className="badge warn">PAY-TO-WIN DESIGN? ({scan.design!.length})</span>{" "}
          <span className="muted">
            {audience === "maker"
              ? "Patterns that usually go with paying to win: premium currencies, skipping timers, VIP tiers, passes, bought boosts. Nothing on Slop can be sold, so a reviewer checks they're earned in play. If they are, nothing needs changing."
              : "Patterns only: check each is earned in play and can't be bought or traded for anything outside the game. Paying to win breaks the Content Policy."}
          </span>
          <ul>
            {scan.design!.map((h) => (
              <li key={h.rule}>
                {h.label} ({h.count}): <span className="muted">{h.examples.join(", ")}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
      {scan.pulse_missing ? (
        <div>
          <span className="badge warn">NO SLOP PULSE</span>{" "}
          <span className="muted">
            {audience === "maker"
              ? "The game this builds on counts players with Slop Pulse, so community builds must too: keep its Pulse code (see Slop Pulse in the publishing guide)."
              : "A community build of a game that counts players, with no Pulse code: it can't be listed before its first Pulse handshake."}
          </span>
        </div>
      ) : (
        scan.pulse === "start_only" && (
          <div>
            <span className="badge neutral">PULSE WITHOUT HEARTBEATS</span>{" "}
            <span className="muted">
              The code starts a Pulse session but never sends heartbeats, so a player counts for 10 minutes at most. Send{" "}
              <code>/heartbeat</code> every <code>SLOP_PULSE_INTERVAL</code> seconds.
            </span>
          </div>
        )
      )}
      {(scan.mod_code || scan.mod_online) && (
        <div>
          {scan.mod_code && <span className="badge warn">MOD WITH CODE</span>}{" "}
          {scan.mod_online && <span className="badge warn">MOD GOES ONLINE</span>}{" "}
          <span className="muted">
            {audience === "maker"
              ? `Mods may carry code. It gets a full read before approval${scan.mod_online ? ", and because it goes online, players see that and are told to use sandbox mode" : ""}. Say in your description what the code does${scan.mod_online ? " and what it connects to" : ""}.`
              : `Read all of the mod's code: it runs inside the game. It may only add to its game, never run code from a server, update itself or load other mods.${scan.mod_online ? " It goes online: check what it connects to and that the description says so. Approving is fine; players see GOES ONLINE and get the sandbox suggestion." : ""}`}
          </span>
          {(scan.code_files ?? []).length > 0 && (
            <ul>
              {(scan.code_files ?? []).map((h) => (
                <li key={h.rule}>
                  {h.label} ({h.count}): <span className="muted">{h.examples.join(", ")}</span>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
      {malicious.length > 0 && (
        <div>
          <span className="badge bad">
            MALICIOUS PACKAGE ({malicious.length})
          </span>{" "}
          <span className="muted">
            {audience === "maker"
              ? "OSV lists this package as malware. Remove it, rotate any keys the build machine had, and release again."
              : "OSV lists it as malware: the build may carry it. Consider withdrawing the release."}
          </span>
          <ul>
            {malicious.map((a) => (
              <li key={a.vuln_id + a.package}>
                {a.package} {a.version} ({a.ecosystem}): {osvLink(a.vuln_id)}{" "}
                {a.summary}
              </li>
            ))}
          </ul>
        </div>
      )}
      {since.length > 0 && (
        <details>
          <summary>
            <span className="badge warn">
              NEW ADVISORIES SINCE THIS BUILD ({since.length})
            </span>
          </summary>
          <ul>
            {since.map((a) => (
              <li key={a.vuln_id + a.package}>
                {a.package} {a.version} ({a.ecosystem}): {osvLink(a.vuln_id)}{" "}
                {a.summary}
                {a.fixed && (
                  <span className="muted"> Fixed in {a.fixed}.</span>
                )}{" "}
                <span className="muted">Found {formatDate(a.found_at)}.</span>
              </li>
            ))}
          </ul>
        </details>
      )}
      {(scan.packages ?? 0) > 0 && (
        <div className="muted">
          {scan.packages} packages from {scan.lockfiles?.join(", ")}, checked
          daily against OSV for new advisories.
        </div>
      )}
    </div>
  );
}

/**
 * A lookup of the file's hash on VirusTotal, as an optional second opinion. It's only a link: Slop
 * sends nothing to VirusTotal, and it shows a report only if someone has already uploaded the file.
 */
export function VirusTotalLink({
  sha256,
  audience,
}: {
  sha256: string;
  audience: "maker" | "reviewer";
}) {
  return (
    <div className="small muted">
      <ExternalLink
        href={`https://www.virustotal.com/gui/file/${encodeURIComponent(sha256)}`}
      >
        Look up on VirusTotal
      </ExternalLink>{" "}
      (optional second opinion; finds a report only if the file was uploaded
      there before.
      {audience === "maker"
        ? " Uploading it yourself shares the file with VirusTotal's subscribers, so don't for a private app."
        : " A few detections from little-known engines are common for new, unsigned programs."}
      )
    </div>
  );
}

/** All of a file's checks, for the maker or a reviewer. */
export function FileChecks({
  file,
  audience,
}: {
  file: ReleaseFile;
  audience: "maker" | "reviewer";
}) {
  const windows = file.platform === "windows";
  return (
    <>
      {windows && file.sha256 && (
        <DefenderLine defender={file.defender} audience={audience} />
      )}
      {windows && <SignedLine defender={file.defender} audience={audience} />}
      {audience === "reviewer" && <ScanFindings report={file.scan_report} />}
      {file.sha256 && (
        <VirusTotalLink sha256={file.sha256} audience={audience} />
      )}
      {file.cleared_reason && (
        <div className="small">
          <span className="badge neutral">CLEARED BY A REVIEWER</span>{" "}
          <span className="muted">{file.cleared_reason}</span>
        </div>
      )}
    </>
  );
}

/** Where the release keeps local saves: every checked release says (DESIGN §6). */
function SaveFolderCheck({ scan, audience }: { scan: RepoScan; audience: "maker" | "reviewer" }) {
  const saves = scan.saves ?? [];
  const guide = <Link to="/publish/guide#saves-and-sandbox-mode">Saves and sandbox mode</Link>;
  if (scan.save_folder === "slop") {
    return (
      <div>
        <span className="badge official">SAVES IN SLOP'S SAVE FOLDER</span>{" "}
        <span className="muted">the code reads SLOP_SAVE_DIR</span>
      </div>
    );
  }
  if (scan.save_folder === "engine") {
    return (
      <div>
        <span className="badge official">SAVES IN SLOP'S SAVE FOLDER</span>{" "}
        <span className="muted">through Godot's user://, which the Slop app points there</span>
      </div>
    );
  }
  if (saves.length === 0) {
    // "none", or a check from before Slop looked.
    return scan.save_folder === "none" ? (
      <div>
        <span className="badge neutral">NO LOCAL SAVES FOUND</span>{" "}
        <span className="muted">
          {audience === "maker" ? (
            <>No SLOP_SAVE_DIR and no writes to the player's folders. If it saves on the player's computer, use SLOP_SAVE_DIR: see {guide}.</>
          ) : (
            "Neither SLOP_SAVE_DIR nor writes to fixed per-user folders: it likely saves nothing locally, or on a server."
          )}
        </span>
      </div>
    ) : null;
  }
  return (
    <div>
      {scan.sandbox_mismatch ? (
        <span className="badge warn">SAYS IT WORKS IN SANDBOX MODE, BUT SAVES ELSEWHERE</span>
      ) : (
        <span className="badge warn">SAVES OUTSIDE SLOP'S SAVE FOLDER</span>
      )}{" "}
      <span className="muted">
        {audience === "maker" ? (
          <>
            Your code writes to fixed places in the player's folders. Slop asks apps to keep local saves, settings and logs in
            SLOP_SAVE_DIR, which the Slop app sets when it starts yours, so they follow players into sandbox mode.
            {scan.sandbox_mismatch && " Until it does, players aren't told your app works in sandbox mode."} See {guide}.
          </>
        ) : scan.sandbox_mismatch ? (
          "The store page says it works in sandbox mode, but it writes to the player's folders. Players aren't told it works in sandbox mode; ask the maker to use SLOP_SAVE_DIR or untick it."
        ) : (
          "It writes to the player's folders rather than SLOP_SAVE_DIR, so its saves won't follow players into sandbox mode. Ask the maker to move them; not a reason to reject."
        )}
      </span>
      <ul>
        {saves.map((h) => (
          <li key={h.rule}>
            {h.label} ({h.count}): <span className="muted">{h.examples.join(", ")}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
