// SPDX-License-Identifier: AGPL-3.0-or-later
// The tabs at the top of Publish: Your apps (/publish) and Stats (/publish/stats).

import { Link } from "react-router";

export type PublishTab = "apps" | "stats";

const TABS: Array<[PublishTab, string, string]> = [
  ["apps", "Your apps", "/publish"],
  ["stats", "Stats", "/publish/stats"],
];

export function PublishTabs({ on }: { on: PublishTab }) {
  return (
    <nav className="maker-tabs" aria-label="Publish">
      {TABS.map(([key, label, to]) => (
        <Link key={key} to={to} className={on === key ? "on" : undefined} aria-current={on === key ? "page" : undefined}>
          {label}
        </Link>
      ))}
    </nav>
  );
}
