// SPDX-License-Identifier: AGPL-3.0-or-later
// Fixes offered between a community build and its original (crates/slop-api/src/offers.rs): the
// build's maker registers a pull request on the original's repository; the original's maker
// accepts or declines. Merging happens on the repository.

import { useState } from "react";
import { Link } from "react-router";

import { api, ApiError, type UpstreamOffer } from "../api";
import { formatDate } from "../format";
import { useLoad } from "../hooks";
import { ErrorState, ExternalLink } from "./ui";

const STATUS: Record<UpstreamOffer["status"], string> = {
  open: "WAITING",
  accepted: "ACCEPTED",
  declined: "DECLINED",
  withdrawn: "WITHDRAWN",
};

export function OffersPanel({ slug, originalSlug }: { slug: string; originalSlug: string | null }) {
  const isBranch = originalSlug !== null;
  const offers = useLoad(() => api.offers(slug), [slug]);
  const [error, setError] = useState<string>();
  const run = async (action: () => Promise<unknown>) => {
    setError(undefined);
    try {
      await action();
      offers.reload();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Something went wrong. Try again.");
    }
  };
  const received = offers.data?.filter((o) => o.direction === "received") ?? [];
  const sent = offers.data?.filter((o) => o.direction === "sent") ?? [];
  if (!isBranch && received.length === 0) return null;
  return (
    <section className="panel panel-pad" id="offers" aria-labelledby="offers-title">
      <h2 id="offers-title">
        {isBranch ? (
          <>
            Fixes for the original (<Link to={`/app/${originalSlug}`}>{originalSlug}</Link>)
          </>
        ) : (
          "Fixes offered to you"
        )}
      </h2>
      {offers.error && <ErrorState error={offers.error} onRetry={offers.reload} />}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      {received.length > 0 && (
        <ul className="offer-list">
          {received.map((o) => (
            <Received key={o.id} slug={slug} offer={o} run={run} />
          ))}
        </ul>
      )}
      {isBranch && (
        <>
          <p className="muted small">
            Made something the original would want? Open a pull request with your changes on its repository, then offer
            it here: its maker is told, and can accept or decline.
          </p>
          <OfferForm slug={slug} run={run} />
          {sent.length > 0 && (
            <ul className="offer-list">
              {sent.map((o) => (
                <li key={o.id}>
                  <span className="badge">{STATUS[o.status]}</span> <ExternalLink href={o.pull_request_url}>Pull request</ExternalLink>{" "}
                  <span className="faint small">offered {formatDate(o.created_at)}</span>
                  <p className="pre small">{o.message}</p>
                  {o.answer && <p className="small">Reply: {o.answer}</p>}
                  {o.status === "open" && (
                    <button type="button" className="btn small ghost" onClick={() => void run(() => api.withdrawOffer(slug, o.id))}>
                      Withdraw
                    </button>
                  )}
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  );
}

function Received({ slug, offer: o, run }: { slug: string; offer: UpstreamOffer; run: (a: () => Promise<unknown>) => Promise<void> }) {
  const [reply, setReply] = useState("");
  return (
    <li>
      <span className="badge">{STATUS[o.status]}</span> <Link to={`/app/${o.branch_slug}`}>{o.branch_title}</Link> by{" "}
      {o.offered_by} · <ExternalLink href={o.pull_request_url}>Pull request</ExternalLink>{" "}
      <span className="faint small">{formatDate(o.created_at)}</span>
      <p className="pre small">{o.message}</p>
      {o.status === "open" ? (
        <div className="form">
          <label>
            Reply (optional)
            <input value={reply} maxLength={2000} onChange={(e) => setReply(e.target.value)} />
          </label>
          <p className="faint small">Review the pull request on your repository and merge it there if you accept.</p>
          <div className="row-actions">
            <button type="button" className="btn small slime" onClick={() => void run(() => api.answerOffer(slug, o.id, "accepted", reply))}>
              Accept
            </button>
            <button type="button" className="btn small ghost" onClick={() => void run(() => api.answerOffer(slug, o.id, "declined", reply))}>
              Decline
            </button>
          </div>
        </div>
      ) : (
        o.answer && <p className="small">Your reply: {o.answer}</p>
      )}
    </li>
  );
}

function OfferForm({ slug, run }: { slug: string; run: (a: () => Promise<unknown>) => Promise<void> }) {
  const [url, setUrl] = useState("");
  const [message, setMessage] = useState("");
  return (
    <form
      className="form"
      onSubmit={(e) => {
        e.preventDefault();
        void run(async () => {
          await api.offerFix(slug, url.trim(), message);
          setUrl("");
          setMessage("");
        });
      }}
    >
      <label>
        Pull request on the original's repository
        <input type="url" value={url} required placeholder="https://github.com/owner/game/pull/12" onChange={(e) => setUrl(e.target.value)} />
      </label>
      <label>
        What it fixes or adds
        <textarea value={message} maxLength={2000} rows={3} required onChange={(e) => setMessage(e.target.value)} />
      </label>
      <div className="row-actions">
        <button className="btn small slime" disabled={!url.trim() || !message.trim()}>
          Offer it
        </button>
      </div>
    </form>
  );
}
