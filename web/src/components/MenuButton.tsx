// SPDX-License-Identifier: AGPL-3.0-or-later
// A menu button: a small button (by default three dots, "⋯") that opens a menu of actions, links and choices,
// as the WAI-ARIA menu button pattern says. Enter, Space or ↓ opens it on the first item, ↑ on the
// last; ↑ ↓ Home End and a first letter move between items; Escape, Tab or a click outside close
// it, and focus goes back to the button. A disabled item stays in the menu, with its reason under
// it, so a keyboard or screen reader user learns why too.
//
//   <MenuButton label="Actions for Harbour Lights" sections={[{ items: [{ label: "Open", onSelect }] }]} />

import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { Link } from "react-router";

export type MenuEntry =
  | {
      kind?: "action";
      label: string;
      onSelect: () => void;
      danger?: boolean;
      /** Why it can't be chosen now, shown under it. */
      disabled?: string;
      /** The choice opens something that takes focus (a form): don't put focus back on the button. */
      keepsFocus?: boolean;
    }
  | { kind: "link"; label: string; to: string }
  | { kind: "radio"; label: string; checked: boolean; onSelect: () => void; disabled?: string };

/** Three dots in a row, crisper than the "⋯" character in most fonts. */
const DOTS = (
  <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor" focusable="false">
    <circle cx="3" cy="8" r="1.6" />
    <circle cx="8" cy="8" r="1.6" />
    <circle cx="13" cy="8" r="1.6" />
  </svg>
);

export interface MenuSection {
  /** Names the group for screen readers, and is shown above it. */
  label?: string;
  items: MenuEntry[];
}

export function MenuButton({
  label,
  sections,
  icon = DOTS,
  disabled,
}: {
  /** The button's accessible name, e.g. "Actions for Harbour Lights". */
  label: string;
  sections: MenuSection[];
  icon?: ReactNode;
  disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  /** Which item to focus once the menu is open. */
  const [start, setStart] = useState<"first" | "last">("first");
  const id = useId();
  const root = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);

  const items = () => Array.from(menu.current?.querySelectorAll<HTMLElement>('[role^="menuitem"]') ?? []);

  useEffect(() => {
    if (!open) return;
    const all = items();
    (start === "first" ? all[0] : all[all.length - 1])?.focus();
    const outside = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", outside);
    return () => document.removeEventListener("mousedown", outside);
  }, [open]);

  const show = (where: "first" | "last") => {
    setStart(where);
    setOpen(true);
  };
  const close = (refocus = true) => {
    setOpen(false);
    if (refocus) button.current?.focus();
  };

  const onMenuKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const all = items();
    const at = all.indexOf(document.activeElement as HTMLElement);
    const go = (i: number) => {
      e.preventDefault();
      all[(i + all.length) % all.length]?.focus();
    };
    if (e.key === "ArrowDown") go(at + 1);
    else if (e.key === "ArrowUp") go(at - 1);
    else if (e.key === "Home") go(0);
    else if (e.key === "End") go(all.length - 1);
    else if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "Tab") close(false);
    else if (e.key.length === 1 && /\S/.test(e.key)) {
      // The next item starting with that letter.
      const letter = e.key.toLowerCase();
      for (let n = 1; n <= all.length; n++) {
        const i = (at + n) % all.length;
        if (all[i].textContent?.trim().toLowerCase().startsWith(letter)) {
          go(i);
          break;
        }
      }
    }
  };

  const choose = (entry: { onSelect: () => void; disabled?: string; keepsFocus?: boolean }) => {
    if (entry.disabled) return;
    close(!entry.keepsFocus);
    entry.onSelect();
  };

  return (
    <div className="menu-button" ref={root}>
      <button
        ref={button}
        type="button"
        className={`btn small ghost menu-button-btn${open ? " on" : ""}`}
        aria-label={label}
        title={label}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        disabled={disabled}
        onClick={() => (open ? close() : show("first"))}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            show(e.key === "ArrowDown" ? "first" : "last");
          }
        }}
      >
        <span aria-hidden="true">{icon}</span>
      </button>
      {open && (
        <div className="menu-button-menu panel" role="menu" id={id} aria-label={label} ref={menu} onKeyDown={onMenuKey}>
          {sections
            .filter((s) => s.items.length > 0)
            .map((section, si) => (
              <div key={si} role="group" aria-label={section.label} className="menu-button-group">
                {si > 0 && <div role="separator" className="menu-button-sep" />}
                {section.label && (
                  <div className="menu-button-label" aria-hidden="true">
                    {section.label}
                  </div>
                )}
                {section.items.map((entry, i) => (
                  <Entry key={i} entry={entry} onChoose={choose} onFollow={() => close(false)} />
                ))}
              </div>
            ))}
        </div>
      )}
    </div>
  );
}

function Entry({
  entry,
  onChoose,
  onFollow,
}: {
  entry: MenuEntry;
  onChoose: (e: { onSelect: () => void; disabled?: string; keepsFocus?: boolean }) => void;
  onFollow: () => void;
}) {
  const noteId = useId();
  if (entry.kind === "link") {
    return (
      <Link role="menuitem" tabIndex={-1} className="menu-button-item" to={entry.to} onClick={onFollow}>
        {entry.label}
      </Link>
    );
  }
  const radio = entry.kind === "radio";
  return (
    <>
      <button
        type="button"
        role={radio ? "menuitemradio" : "menuitem"}
        aria-checked={radio ? entry.checked : undefined}
        aria-disabled={entry.disabled ? true : undefined}
        aria-describedby={entry.disabled ? noteId : undefined}
        tabIndex={-1}
        className={`menu-button-item${!radio && entry.danger ? " danger" : ""}`}
        onClick={() => onChoose(entry)}
      >
        {radio && (
          <span className="menu-button-check" aria-hidden="true">
            {entry.checked ? "●" : ""}
          </span>
        )}
        {entry.label}
      </button>
      {entry.disabled && (
        <p className="menu-button-note" id={noteId}>
          {entry.disabled}
        </p>
      )}
    </>
  );
}
