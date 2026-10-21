// SPDX-License-Identifier: AGPL-3.0-or-later
// The simulation sandbox's "view as": pick a simulated user and see the store, "For you" and
// ratings as they would (crates/slop-api/src/sim.rs). Only on a server running the sandbox.

import { useState } from "react";
import { useNavigate } from "react-router";

import { api, ApiError, type SimUser } from "../api";
import { ErrorState, Loading } from "../components/ui";
import { useDocumentTitle, useLoad } from "../hooks";
import { useSession } from "../session";
import { NotFound } from "./Listing";

export function Sim() {
  useDocumentTitle("Simulated users");
  const server = useLoad(() => api.server(), []);
  const users = useLoad(() => (server.data?.simulation ? api.simUsers() : Promise.resolve([])), [server.data?.simulation]);
  const { refresh } = useSession();
  const navigate = useNavigate();
  const [filter, setFilter] = useState("");
  const [error, setError] = useState<string>();

  if (!server.data) return <Loading />;
  if (!server.data.simulation) return <NotFound />;
  if (users.error) return <ErrorState error={users.error} onRetry={users.reload} />;
  if (!users.data) return <Loading />;

  const viewAs = async (u: SimUser) => {
    try {
      await api.simLogin(u.id);
      await refresh();
      navigate(u.personas.startsWith("People:") ? "/people/jobs" : "/");
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };
  const shown = users.data.filter((u) => `${u.personas} ${u.display_name}`.toLowerCase().includes(filter.toLowerCase()));
  // "roguelike fan:1,developer:0.35" → "roguelike fan + a little developer"
  const describe = (p: string) =>
    p.startsWith("People:")
      ? p
      : p
      .split(",")
      .map((part) => {
        const [name, w] = part.split(":");
        return Number(w) < 0.5 ? `a little ${name}` : Number(w) < 1 ? `some ${name}` : name;
      })
      .join(" + ");

  return (
    <div className="page">
      <h1 className="page-title">View as a simulated user</h1>
      <p className="muted">
        Each simulated user has a hidden taste (one or two personas) that decided what they added, played and voted for.
        Pick one to see the store, <b>For you</b> and ratings as they do. UK accounts start with personal recommendations
        off, as real ones do. To try People, view as a <b>People: maker</b> (hiring) or a <b>People: creative</b> (working): you
        land on their jobs, in every state from inquiry to problem.
      </p>
      <div className="filters">
        <label>
          Filter{" "}
          <input value={filter} placeholder="e.g. developer, cozy, GB" onChange={(e) => setFilter(e.target.value)} />
        </label>
      </div>
      {error && <p className="form-error">{error}</p>}
      <ul className="release-list">
        {shown.map((u) => (
          <li key={u.id} className="panel panel-pad title-row spread">
            <div>
              <b>{describe(u.personas)}</b>
              <div className="muted small">
                {u.personas.startsWith("People:")
                  ? `${u.display_name} · ${u.country_code} · ${u.library} jobs`
                  : `${u.display_name} · ${u.country_code} · ${u.library} in library · ${u.votes} votes`}
              </div>
            </div>
            <button className="btn small slime" onClick={() => void viewAs(u)}>
              View as
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
