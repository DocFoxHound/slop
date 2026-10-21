// SPDX-License-Identifier: AGPL-3.0-or-later
// Slop's source code (README "License"): the AGPL asks a network service to offer its users the
// source of the version they're using.

import { api } from "../api";
import { useDocumentTitle, useLoad } from "../hooks";
import { ErrorState, Loading } from "../components/ui";

export function Source() {
  useDocumentTitle("Source code");
  const source = useLoad(() => api.source(), []);
  return (
    <div className="page narrow doc">
      <h1>Source code</h1>
      <p>
        Slop is open source. Its server, website and desktop app are under the{" "}
        <a href="https://www.gnu.org/licenses/agpl-3.0.html">GNU Affero General Public License, version 3 or later</a>:
        you may use, study, change and share them, and anyone who runs a changed version for others must offer them its
        source too. The code makers and other stores build on (the engine snippets, Slop Build's recipes and the
        manifest checker) is under the Apache 2.0 or MIT licence, so using it never changes your own licence.
      </p>
      {source.error && <ErrorState error={source.error} onRetry={source.reload} />}
      {!source.data ? (
        !source.error && <Loading />
      ) : source.data.url ? (
        <>
          <p>
            <a href={source.data.url}>Slop's source code</a>
          </p>
          {source.data.version_url && source.data.commit && (
            <p>
              This site runs commit{" "}
              <a href={source.data.version_url}>
                <code>{source.data.commit.slice(0, 12)}</code>
              </a>
              .
            </p>
          )}
        </>
      ) : (
        <p>The source code isn't public yet. It will be linked here when it is.</p>
      )}
    </div>
  );
}
