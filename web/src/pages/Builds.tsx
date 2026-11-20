// SPDX-License-Identifier: AGPL-3.0-or-later
// An app's Builds tab (/publish/{slug}/builds): Slop Build (one file in the maker's repository, and
// releases arrive by themselves, docs/ARCHITECTURE.md §6.2), Slop Check for releases built
// anywhere else, and signing builds yourself. Moved off the Releases page on 2026-11-20, where
// makers missed it. SLAPorSLOP looks in the repository's .github/workflows first (slop_build.rs
// `setup`): a Slop Build or Slop Check workflow already there is shown as set up, with nothing to
// add, and a file of the maker's own at the usual name gets another name suggested, so GitHub
// never answers "A file with the same name already exists". A browser game has nothing to build.

import { useEffect } from "react";
import { Link, useLocation, useParams } from "react-router";

import { api, ApiError } from "../api";
import { MakerFrame, rememberSlopBuild } from "../components/MakerFrame";
import { ErrorState, ExternalLink, Loading } from "../components/ui";
import { playsInBrowser } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { useSession } from "../session";

export type SlopBuildSetupInfo = Awaited<ReturnType<typeof api.slopBuildSetup>>;

export function Builds() {
  const { slug = "" } = useParams();
  const { me } = useSession();
  const location = useLocation();
  const app = useLoad(() => (me ? api.app(slug) : Promise.resolve(undefined)), [slug, me?.id]);
  const browser = app.data ? playsInBrowser(app.data) : undefined;
  // Only once the app is known: a browser game has no setup to ask GitHub about.
  const setup = useLoad(
    () => (me && browser === false ? api.slopBuildSetup(slug) : Promise.resolve(undefined)),
    [slug, me?.id, browser],
  );
  useDocumentTitle(app.data ? `${app.data.title} builds` : "Builds");
  const found = setup.data?.workflows_checked ? Boolean(setup.data.existing) : undefined;
  useEffect(() => {
    if (found !== undefined) rememberSlopBuild(slug, found);
  }, [slug, found]);
  // "#slop-check" (from a release on hold, or an older link): open Slop Check and go to it.
  const ready = Boolean(setup.data);
  useEffect(() => {
    if (!ready || location.hash !== "#slop-check") return;
    const details = document.getElementById("slop-check");
    if (details instanceof HTMLDetailsElement) details.open = true;
    details?.scrollIntoView?.({ block: "start" });
  }, [ready, location.hash]);

  if (me === undefined || app.loading) return <Loading />;
  if (!me) {
    return (
      <div className="page narrow">
        <p>
          <Link to={`/signin?return_to=/publish/${slug}/builds`}>Sign in</Link> to set up builds.
        </p>
      </div>
    );
  }
  if (app.error || (app.data && !app.data.is_mine)) {
    return (
      <div className="page">
        <ErrorState error={app.error ?? new ApiError(404, "not_found", "")} />
      </div>
    );
  }
  if (!app.data) return <Loading />;

  return (
    <MakerFrame app={app.data} tab="builds" slopBuildSetUp={found} onStageMoved={app.reload}>
      {browser ? (
        <section className="panel panel-pad">
          <h2>Nothing to build</h2>
          <p>
            This game plays in the browser, on your own website: Slop Build makes downloads that players install. Give its
            play address on the <Link to={`/publish/${slug}/releases`}>Releases</Link> tab.
          </p>
        </section>
      ) : (
        <div className="maker-cols">
          <div className="maker-main">
            <SlopBuildSetup setup={setup} />
          </div>
          <div className="maker-side">
            <SigningHelp />
          </div>
        </div>
      )}
    </MakerFrame>
  );
}

const WORKFLOW_SNIPPET = `permissions:
  contents: write        # upload to your release
  id-token: write        # get a signing certificate from Sigstore
  attestations: write    # store the attestation on GitHub

steps:
  # … build your app into dist/ …
  - uses: actions/attest-build-provenance@v2
    with:
      subject-path: dist/*`;

const PROJECT_LABEL: Record<string, string> = {
  rust: "Rust",
  go: "Go",
  godot: "Godot",
  "godot-dotnet": "Godot (C#)",
  dotnet: ".NET",
  unity: "Unity",
  web: "web (HTML5)",
  electron: "Electron",
  tauri: "Tauri",
  python: "Python",
  custom: "custom",
};

/** A workflow file in the repository, linked to its page on GitHub. */
function WorkflowLink({ file }: { file: { path: string; html_url: string } }) {
  return (
    <ExternalLink href={file.html_url}>
      <code>{file.path}</code>
    </ExternalLink>
  );
}

/** Slop Build: one file in the maker's repository, and releases arrive by themselves. Exported for tests. */
export function SlopBuildSetup({
  setup,
}: {
  setup: { data?: SlopBuildSetupInfo; error?: ApiError; loading: boolean };
}) {
  if (setup.error?.status === 503) {
    return (
      <section className="panel panel-pad help slop-build">
        <h2>Slop Build</h2>
        <p className="notice">Slop Build isn't switched on on this SLAPorSLOP server: build and sign releases yourself.</p>
      </section>
    );
  }
  const s = setup.data;
  const existing = s?.supported ? s.existing : null;
  const buildNow = s?.build_now_url && (
    <p>
      <ExternalLink href={s.build_now_url}>
        <span className="btn">Build now</span>
      </ExternalLink>{" "}
      <span className="muted small">
        {existing
          ? "Opens it on GitHub, where Run workflow builds your main branch straight away."
          : "Once the file is in your repository: opens it on GitHub, where Run workflow builds your main branch straight away."}
      </span>
    </p>
  );
  return (
    <>
      <section className="panel panel-pad help slop-build" aria-labelledby="slop-build-title">
        <h2 id="slop-build-title">{existing ? "Slop Build" : "Let Slop Build do it"}</h2>
        {existing ? (
          <p>
            <b>Slop Build is set up:</b> <WorkflowLink file={existing} />. Its builds are signed, attached to a release and
            sent here by themselves: follow them on the Releases tab.
          </p>
        ) : (
          <p>
            Add one file to your repository and every push to your main branch, or every <code>v*</code> tag, builds your app
            (Rust, Go, .NET, Godot, Unity, Python, Electron, Tauri, a website, or your own build command) for Windows and Linux
            on GitHub Actions (a .zip, a .tar.gz and .deb/.rpm packages), signs it, attaches it to a release, and sends it
            here. No keys or secrets to set up.
          </p>
        )}
        {setup.loading && !s && <p className="muted">Looking at your repository…</p>}
        {setup.error && (
          <p className="form-error" role="alert">
            {setup.error.message}
          </p>
        )}
        {s && (
          <>
            {/* The note is about the suggested file: not once the maker's own is there. */}
            {s.note && !existing && <p className="notice">{s.note}</p>}
            {existing ? (
              <>
                {buildNow}
                <details>
                  <summary className="small">See the suggested workflow</summary>
                  <p className="muted small">What SLAPorSLOP would suggest for this repository today, to compare with yours.</p>
                  <pre>
                    <code>{s.workflow}</code>
                  </pre>
                </details>
              </>
            ) : (
              s.supported && (
                <>
                  {s.taken && (
                    <p className="notice">
                      Your repository already has <WorkflowLink file={s.taken} />, and it isn't Slop Build, so this file goes
                      at <code>{s.file_path}</code> instead.
                    </p>
                  )}
                  {s.workflows_checked === false && (
                    <p className="muted small">
                      SLAPorSLOP couldn't look in your repository's <code>.github/workflows</code> just now. If Slop Build is
                      already there, there's nothing to add.
                    </p>
                  )}
                  {!s.note && (
                    <p className="muted small">
                      Detected a <b>{PROJECT_LABEL[s.project_type]}</b> project.
                    </p>
                  )}
                  <p className="muted small">
                    <code>{s.file_path}</code>
                  </p>
                  <pre>
                    <code>{s.workflow}</code>
                  </pre>
                  {s.create_url && (
                    <p>
                      <ExternalLink href={s.create_url}>
                        <span className="btn slime">Add to my repository on GitHub</span>
                      </ExternalLink>{" "}
                      <span className="muted small">Opens GitHub with the file filled in; you just commit it.</span>
                    </p>
                  )}
                  {buildNow}
                </>
              )
            )}
          </>
        )}
      </section>
      {s?.supported && <SlopCheck setup={s} />}
    </>
  );
}

/** Slop Check: SLAPorSLOP's checks on the repository's own GitHub Actions, for releases built elsewhere. */
function SlopCheck({ setup: s }: { setup: SlopBuildSetupInfo }) {
  const existing = s.check_existing;
  return (
    <details id="slop-check" className="panel panel-pad help">
      <summary>
        {existing ? (
          <>
            <b>Slop Check is set up</b> <span className="muted">— SLAPorSLOP's checks run on your GitHub Actions</span>
          </>
        ) : (
          <>
            <b>Building somewhere else?</b>{" "}
            <span className="muted">Add Slop Check so SLAPorSLOP's checks run on your GitHub Actions</span>
          </>
        )}
      </summary>
      <p className="muted small">
        Whenever you publish a release, Slop Check runs SLAPorSLOP's malware scan of every file, its source check and (for
        Windows builds) Microsoft Defender on your repository's own GitHub Actions, so your release isn't waiting for
        SLAPorSLOP's servers. Slop Build does all of this for you. <Link to="/publish/guide#slop-check">More about it</Link>.
      </p>
      {existing ? (
        <>
          <p className="small">
            <b>Slop Check is set up:</b> <WorkflowLink file={existing} />.
          </p>
          <details>
            <summary className="small">See the suggested workflow</summary>
            <pre>
              <code>{s.check_workflow}</code>
            </pre>
          </details>
        </>
      ) : (
        <>
          {s.check_taken && (
            <p className="notice small">
              Your repository already has <WorkflowLink file={s.check_taken} />, and it isn't Slop Check, so this file goes at{" "}
              <code>{s.check_file_path}</code> instead.
            </p>
          )}
          <p className="muted small">
            <code>{s.check_file_path}</code>
          </p>
          <pre>
            <code>{s.check_workflow}</code>
          </pre>
          {s.check_create_url && (
            <p>
              <ExternalLink href={s.check_create_url}>
                <span className="btn">Add Slop Check to my repository</span>
              </ExternalLink>
            </p>
          )}
        </>
      )}
    </details>
  );
}

/** Every build needs its maker's signature (docs/SECURITY.md §5.3). */
function SigningHelp() {
  return (
    <details className="panel panel-pad help">
      <summary>
        <b>Or build and sign it yourself</b>{" "}
        <span className="muted">— every build needs your signature before it can be published</span>
      </summary>
      <p>
        Build your release files in GitHub Actions and add GitHub's attestation step. It signs each file with a short-lived
        certificate tied to your repository, workflow and commit, and records it in Sigstore's public log. There's no key for
        you to manage, and nobody who only has your SLAPorSLOP password can fake it.
      </p>
      <pre>
        <code>{WORKFLOW_SNIPPET}</code>
      </pre>
      <p className="muted small">
        SLAPorSLOP finds the attestation by each file's hash. The commit you send with the release (on the Releases tab) must
        be the one the workflow built. Slop Build does all of this for you. Only GitHub repositories are supported so far.
      </p>
    </details>
  );
}
