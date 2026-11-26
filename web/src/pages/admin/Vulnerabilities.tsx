// SPDX-License-Identifier: AGPL-3.0-or-later
// Known-vulnerable dependencies in the running server build (audit.json, ops.rs). Reached straight
// from the "N known-vulnerable dependencies" admin notification and from OpsAlerts in the queue.

import { Link } from "react-router";

import { api } from "../../api";
import { ErrorState, ExternalLink, Loading } from "../../components/ui";
import { formatDateTime } from "../../format";
import { useLoad } from "../../hooks";

export function VulnerabilitiesView() {
  const audit = useLoad(() => api.admin.opsAudit(), []);

  if (audit.error) return <ErrorState error={audit.error} onRetry={audit.reload} />;
  if (!audit.data) return <Loading />;

  const { available, checked_at, commit, vulnerabilities, accepted } = audit.data;
  const meta: string[] = [];
  if (checked_at) meta.push(`Checked ${formatDateTime(checked_at)}`);
  if (commit) meta.push(`commit ${commit.slice(0, 7)}`);
  if (accepted > 0) meta.push(`${accepted} accepted in deploy/audit-accepted.json`);

  return (
    <section aria-labelledby="ops-vulns-title">
      <p className="small">
        <Link to="/admin?tab=queue">← Review queue</Link>
      </p>
      <h2 className="section-title" id="ops-vulns-title">
        Known-vulnerable dependencies{" "}
        {available && vulnerabilities.length > 0 && (
          <span className="badge warn">{vulnerabilities.length}</span>
        )}
      </h2>
      {meta.length > 0 && <p className="muted small">{meta.join(" · ")}</p>}

      {!available ? (
        <div className="empty-state">No dependency audit has run on this server yet.</div>
      ) : vulnerabilities.length === 0 ? (
        <div className="empty-state">No known-vulnerable dependencies in the current build.</div>
      ) : (
        <>
          <table className="audit">
            <thead>
              <tr>
                <th scope="col">Package</th>
                <th scope="col">Ecosystem</th>
                <th scope="col">Advisory</th>
                <th scope="col">Summary</th>
              </tr>
            </thead>
            <tbody>
              {vulnerabilities.map((v) => (
                <tr key={`${v.package}@${v.version}:${v.id}`}>
                  <td>
                    <code>
                      {v.package} {v.version}
                    </code>
                  </td>
                  <td>{v.ecosystem || "—"}</td>
                  <td>
                    <ExternalLink href={`https://osv.dev/vulnerability/${encodeURIComponent(v.id)}`}>
                      {v.id}
                    </ExternalLink>
                  </td>
                  <td>{v.summary || "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <p className="muted small">
            Update them (<code>cargo update</code>, <code>npm update</code>) and deploy, or record reviewed advisories
            with a reason in <code>deploy/audit-accepted.json</code>.
          </p>
        </>
      )}
    </section>
  );
}
