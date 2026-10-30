// SPDX-License-Identifier: AGPL-3.0-or-later
// "How ratings work" (METRICS §2): the weighting formula is published, so players and makers
// can check what a rating means. The numbers here match crates/slop-api/src/votes.rs.

import { Link } from "react-router";

import { useDocumentTitle } from "../hooks";
import { InlineIcon } from "../components/StoreIcons";

const PLAYTIME: Array<[string, string]> = [
  ["Under 2 minutes", "0.1"],
  ["5 minutes", "0.28"],
  ["30 minutes", "0.54"],
  ["1 hour", "0.64"],
  ["3 hours", "0.81"],
  ["10 hours or more", "1"],
];

const DAYS: Array<[string, string]> = [
  ["1 day", "0.29"],
  ["3 days", "0.58"],
  ["5 days", "0.75"],
  ["10 days or more", "1"],
];

const LABELS: Array<[string, string]> = [
  ["95–100%", "Overwhelmingly Slaps"],
  ["80–94%", "Really Slaps"],
  ["70–79%", "Mostly Slaps"],
  ["40–69%", "Mixed"],
  ["20–39%", "Mostly Slop"],
  ["0–19%", "Overwhelmingly Slop"],
];

export function HowRatings() {
  useDocumentTitle("How ratings work");
  return (
    <div className="page narrow prose">
      <h1>How ratings work</h1>
      <p className="lead">
        Every app on SLAPorSLOP is rated <InlineIcon name="up" /> <b>Slaps</b> or <InlineIcon name="down" /> <b>Slop</b> by the people who play it. A vote from someone who played
        for hours counts for more than one from someone who tried it for a minute.
      </p>

      <h2>Who can vote</h2>
      <ul>
        <li>Anyone signed in who has launched the app through SLAPorSLOP. You can't rate what you haven't tried.</li>
        <li>
          Apps you can play: Beta and released apps. Coming soon apps have nothing to play yet, so they collect Interested marks
          instead. A vote given while the app was in Beta is marked <b>Rated in Beta</b>.
        </li>
        <li>One vote per account. You can change or withdraw it whenever you like.</li>
      </ul>

      <h2>Weighting by playtime</h2>
      <p>Each vote's weight comes from how much you'd played when you voted:</p>
      <pre>
        <code>weight = min(1, log10(1 + minutes) / log10(601))</code>
      </pre>
      <p>So ten hours or more counts fully, and less than two minutes counts 0.1.</p>
      <table className="kv-table">
        <caption className="sr-only">Vote weight by playtime</caption>
        <thead>
          <tr>
            <th scope="col">Played</th>
            <th scope="col">Vote counts</th>
          </tr>
        </thead>
        <tbody>
          {PLAYTIME.map(([p, w]) => (
            <tr key={p}>
              <td>{p}</td>
              <td>{w}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <p>
        Tools and games that don't report playtime use the number of separate days you launched them instead, with ten days or
        more counting fully: <code>min(1, log10(1 + days) / log10(11))</code>.
      </p>
      <table className="kv-table">
        <caption className="sr-only">Vote weight by days launched</caption>
        <thead>
          <tr>
            <th scope="col">Launched on</th>
            <th scope="col">Vote counts</th>
          </tr>
        </thead>
        <tbody>
          {DAYS.map(([d, w]) => (
            <tr key={d}>
              <td>{d}</td>
              <td>{w}</td>
            </tr>
          ))}
        </tbody>
      </table>

      <h2>What you see</h2>
      <ul>
        <li>
          <b>% Slaps, weighted:</b> the weighted share of Slaps votes. It's the number shown first.
        </li>
        <li>
          <b>Unweighted:</b> one account, one vote. It's shown beside the weighted one, so you can see if they disagree.
        </li>
        <li>
          <b>No rating below 5 votes.</b> A handful of votes says too little.
        </li>
        <li>
          <b>Over time:</b> each app page charts its rating month by month, weighted or unweighted.
        </li>
      </ul>
      <table className="kv-table">
        <caption className="sr-only">Rating labels</caption>
        <thead>
          <tr>
            <th scope="col">Weighted % Slaps</th>
            <th scope="col">Label</th>
          </tr>
        </thead>
        <tbody>
          {LABELS.map(([range, label]) => (
            <tr key={range}>
              <td>{range}</td>
              <td>{label}</td>
            </tr>
          ))}
        </tbody>
      </table>

      <h2>The store's lists</h2>
      <ul>
        <li>
          <b>Most Slaps:</b> the best weighted rating overall, among apps with at least 5 votes.
        </li>
        <li>
          <b>Most Slaps right now:</b> the same, counting only votes cast or changed in the last 30 days.
        </li>
        <li>
          <b>Fresh slop:</b> the most weighted Slop votes in the last 30 days. It's for fun, so please be kind in reviews.
        </li>
        <li>
          <b>New and barely seen:</b> apps released in the last 60 days that are in few libraries yet, in an order that
          changes every day, so every new app gets its turn.
        </li>
        <li>
          <b>Glow-ups:</b> apps whose % Slaps rose by 15 points or more since 60 days ago, showing before and after,
          so updates that fix things get noticed.
        </li>
        <li>
          <b>Discover:</b> signed in with recommendations based on your activity on, it starts with your For you picks (from
          your library, Interested list, playtime and votes), then rows for the genres you play most, like most (Slaps votes and
          Interested), keep most in your library and have donated to, leaving out what you already have. Mixed in for everyone:
          all-time favourites (at least 70% Slaps, 5 or more votes), up-and-coming releases (from the last 60 days, most added and
          played this week), and the most popular this week and this month. The rows' order changes once a day. Each row says
          what it's based on. Signed out or switched off, it's the same for everyone.
        </li>
      </ul>
      <p>
        <b>No paid placement.</b> Nobody can pay to rank higher, be featured or be recommended. Donations and hiring
        creatives never change where an app appears for anyone else (Content Policy, "No paid placement"). Your own donations
        only shape your own Discover page, and only while recommendations based on your activity are on.
      </p>

      <h2>Apps that hired creatives</h2>
      <p>
        When an app credits work from someone hired through People, its page also shows the rating from votes since the first
        release with that work. That never changes the app's main rating.
      </p>

      <h2>Your votes and privacy</h2>
      <ul>
        <li>Your current votes and reviews show on your profile, unless you make it private on your account page.</li>
        <li>The history of your changes is never shown to anyone else.</li>
        <li>Makers see totals, for example by version and platform, never who voted.</li>
        <li>No one can buy votes or ratings: SLAPorSLOP sells nothing, and donations never affect them.</li>
      </ul>
      <p>
        <Link to="/">Back to the store</Link>
      </p>
    </div>
  );
}
