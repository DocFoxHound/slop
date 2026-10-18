// The licence field on the Publish form: popular open-source licences, "Other" for any SPDX ID,
// and detection from the linked repository (GitHub's licence API, via Slop).

import { useEffect, useRef, useState } from "react";

import { api } from "../api";

/** Popular OSI-approved licences, by SPDX ID. */
export const POPULAR_LICENSES: Array<{ id: string; name: string }> = [
  { id: "MIT", name: "MIT License" },
  { id: "Apache-2.0", name: "Apache License 2.0" },
  { id: "GPL-3.0-only", name: "GNU GPL v3.0 only" },
  { id: "GPL-3.0-or-later", name: "GNU GPL v3.0 or later" },
  { id: "GPL-2.0-only", name: "GNU GPL v2.0 only" },
  { id: "GPL-2.0-or-later", name: "GNU GPL v2.0 or later" },
  { id: "AGPL-3.0-only", name: "GNU AGPL v3.0 only" },
  { id: "AGPL-3.0-or-later", name: "GNU AGPL v3.0 or later" },
  { id: "LGPL-3.0-only", name: "GNU LGPL v3.0 only" },
  { id: "LGPL-3.0-or-later", name: "GNU LGPL v3.0 or later" },
  { id: "LGPL-2.1-only", name: "GNU LGPL v2.1 only" },
  { id: "LGPL-2.1-or-later", name: "GNU LGPL v2.1 or later" },
  { id: "MPL-2.0", name: "Mozilla Public License 2.0" },
  { id: "BSD-3-Clause", name: "BSD 3-Clause License" },
  { id: "BSD-2-Clause", name: "BSD 2-Clause License" },
  { id: "0BSD", name: "BSD Zero Clause License" },
  { id: "ISC", name: "ISC License" },
  { id: "MIT-0", name: "MIT No Attribution" },
  { id: "Unlicense", name: "The Unlicense" },
  { id: "Zlib", name: "zlib License" },
  { id: "BSL-1.0", name: "Boost Software License 1.0" },
  { id: "EPL-2.0", name: "Eclipse Public License 2.0" },
  { id: "EUPL-1.2", name: "European Union Public License 1.2" },
  { id: "Artistic-2.0", name: "Artistic License 2.0" },
  { id: "OFL-1.1", name: "SIL Open Font License 1.1" },
];

/** GitHub reports these without "-only" / "-or-later", which only the licence notice says. */
const AMBIGUOUS = new Set(["GPL-2.0", "GPL-3.0", "LGPL-2.1", "LGPL-3.0", "AGPL-3.0"]);

const OTHER = "__other";

function isKnown(id: string) {
  return POPULAR_LICENSES.some((l) => l.id === id);
}

function looksLikeRepo(url: string) {
  return /^https:\/\/[^/\s]+\/[^/\s]+\/[^/\s]+/.test(url.trim());
}

export function LicensePicker({
  repoUrl,
  value,
  onChange,
  error,
}: {
  repoUrl: string;
  value: string;
  onChange: (spdx: string) => void;
  error?: string;
}) {
  const [other, setOther] = useState(value !== "" && !isKnown(value));
  // Once the maker picks a licence themselves, detection no longer overrides it.
  const chosen = useRef(value !== "");
  const [hint, setHint] = useState<string>();

  useEffect(() => {
    if (!looksLikeRepo(repoUrl)) {
      setHint(undefined);
      return;
    }
    let current = true;
    const timer = setTimeout(() => {
      setHint("Checking your repository's licence…");
      api.detectLicense(repoUrl.trim()).then(
        (found) => {
          if (!current) return;
          const spdx = found.spdx;
          if (spdx && AMBIGUOUS.has(spdx)) {
            setHint(
              `Found ${found.name ?? spdx}. Choose "only" or "or later" to match the notice in your code.`,
            );
            return;
          }
          if (spdx) {
            setHint(`Detected from your repository: ${found.name ?? spdx}.`);
            if (!chosen.current) {
              setOther(!isKnown(spdx));
              onChange(spdx);
            }
            return;
          }
          setHint(found.note ? `No licence detected: ${found.note}.` : "No licence detected.");
        },
        () => current && setHint(undefined),
      );
    }, 600);
    return () => {
      current = false;
      clearTimeout(timer);
    };
    // onChange is a fresh closure each render; detection should only follow the URL.
  }, [repoUrl]);

  const select = (next: string) => {
    chosen.current = true;
    if (next === OTHER) {
      setOther(true);
      if (isKnown(value)) onChange("");
    } else {
      setOther(false);
      onChange(next);
    }
  };

  return (
    <div className="license-picker">
      <label htmlFor="license-select">Licence</label>
      <select
        id="license-select"
        value={other ? OTHER : value}
        required={!other}
        onChange={(e) => select(e.target.value)}
      >
        <option value="" disabled>
          Choose a licence
        </option>
        {POPULAR_LICENSES.map((l) => (
          <option key={l.id} value={l.id}>
            {l.name} ({l.id})
          </option>
        ))}
        <option value={OTHER}>Other…</option>
      </select>
      {other && (
        <input
          aria-label="Other licence (SPDX ID)"
          value={value}
          placeholder="SPDX ID, e.g. CDDL-1.0"
          maxLength={64}
          required
          onChange={(e) => {
            chosen.current = true;
            onChange(e.target.value);
          }}
        />
      )}
      <span className="hint">
        {hint ?? "Must be an OSI-approved open-source licence."}{" "}
        {other && (
          <a href="https://spdx.org/licenses/" target="_blank" rel="noopener noreferrer">
            SPDX IDs
          </a>
        )}
      </span>
      {error && (
        <span className="field-error" role="alert">
          {error}
        </span>
      )}
    </div>
  );
}
