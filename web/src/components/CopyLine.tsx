// SPDX-License-Identifier: AGPL-3.0-or-later
// A line for the person to copy, usually into their AI assistant, and the lines SLAPorSLOP offers.

import { useState } from "react";

import { siteOrigin } from "../browsing";

declare const __SLOP_SITE_URL__: string;

/** This SLAPorSLOP site's address (the site a desktop app build is for). */
export function siteAddress(): string {
  return siteOrigin(window.location, typeof __SLOP_SITE_URL__ === "string" ? __SLOP_SITE_URL__ : "");
}

/** The line to hand an AI assistant to get started (public/start.md), for publishing or People. */
export function startPrompt(origin: string, goal: "publish" | "people"): string {
  return goal === "publish"
    ? `Read ${origin}/start.md and help me publish my project on SLAPorSLOP.`
    : `Read ${origin}/start.md and help me set up my People page on SLAPorSLOP.`;
}

/** A line to copy, with its own Copy button. */
export function CopyLine({ text, label }: { text: string; label: string }) {
  const [copied, setCopied] = useState(false);
  const copy = () => {
    void navigator.clipboard?.writeText(text).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    });
  };
  return (
    <div className="ai-prompt">
      <code>{text}</code>
      <button type="button" className="btn small" onClick={copy} aria-label={label}>
        {copied ? "Copied" : "Copy"}
      </button>
    </div>
  );
}
