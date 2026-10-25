// SPDX-License-Identifier: AGPL-3.0-or-later
// Pieces every admin screen shares: running an action, and the reason forms.

import { useState } from "react";

import { ApiError } from "../../api";

/** Runs an admin action, showing its error (e.g. "approve a release first") if refused. */
export function useAction(onDone: () => void) {
  const [busy, setBusy] = useState<string>();
  const [error, setError] = useState<string>();
  const [notice, setNotice] = useState<string>();
  const run = async (key: string, action: () => Promise<unknown>) => {
    setBusy(key);
    setError(undefined);
    setNotice(undefined);
    try {
      const result = await action();
      // Sensitive actions may wait for another staff member (staff.rs).
      if (result && typeof result === "object" && "pending_action" in result) {
        setNotice(
          "Recorded. It happens once another staff member approves it under Review queue → Second approvals.",
        );
      }
      onDone();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(undefined);
    }
  };
  return { busy, error, notice, run };
}

export function ReasonButton({
  label,
  placeholder,
  danger,
  disabled,
  startOpen,
  onCancel,
  onSubmit,
}: {
  label: string;
  placeholder: string;
  danger?: boolean;
  disabled?: boolean;
  /** Open on the reason already (chosen from a menu). */
  startOpen?: boolean;
  /** Cancel was pressed (or Escape in the reason). */
  onCancel?: () => void;
  onSubmit: (reason: string) => void;
}) {
  const [open, setOpen] = useState(startOpen ?? false);
  const cancel = () => {
    setOpen(false);
    onCancel?.();
  };
  const [reason, setReason] = useState("");
  if (!open) {
    return (
      <button
        className={`btn small ${danger ? "ghost danger" : ""}`}
        disabled={disabled}
        onClick={() => setOpen(true)}
      >
        {label}
      </button>
    );
  }
  return (
    <form
      className="reason-form"
      onSubmit={(e) => {
        e.preventDefault();
        if (reason.trim()) onSubmit(reason.trim());
      }}
    >
      <input
        aria-label={`${label}: reason`}
        autoFocus
        value={reason}
        placeholder={placeholder}
        onChange={(e) => setReason(e.target.value)}
        onKeyDown={(e) => e.key === "Escape" && onCancel && cancel()}
      />
      <button
        className={`btn small ${danger ? "danger" : ""}`}
        disabled={disabled || !reason.trim()}
      >
        {label}
      </button>
      <button type="button" className="btn small ghost" onClick={cancel}>
        Cancel
      </button>
    </form>
  );
}

/** Picks a Content Policy rule for a decision. */
export function RuleSelect({
  rules,
  value,
  autoFocus,
  onChange,
}: {
  rules: Array<[string, string]>;
  value: string;
  autoFocus?: boolean;
  onChange: (v: string) => void;
}) {
  return (
    <label>
      The rule it breaks
      <select value={value} required autoFocus={autoFocus} onChange={(e) => onChange(e.target.value)}>
        <option value="" disabled>
          Choose a Content Policy rule
        </option>
        {rules.map(([code, title]) => (
          <option key={code} value={code}>
            {title}
          </option>
        ))}
      </select>
    </label>
  );
}

/** Taking an app out of the store from the Apps tab: the maker gets a statement of reasons. */
export function DelistButton({
  rules,
  disabled,
  startOpen,
  onCancel,
  onSubmit,
}: {
  rules: Array<[string, string]>;
  disabled: boolean;
  /** Open on the form already (chosen from a menu). */
  startOpen?: boolean;
  onCancel?: () => void;
  onSubmit: (rule: string, reason: string) => void;
}) {
  const [open, setOpen] = useState(startOpen ?? false);
  const [rule, setRule] = useState("");
  const [reason, setReason] = useState("");
  if (!open) {
    return (
      <button
        className="btn small ghost danger"
        disabled={disabled}
        onClick={() => setOpen(true)}
      >
        Take out of the store
      </button>
    );
  }
  return (
    <form
      className="form decide-form"
      onSubmit={(e) => {
        e.preventDefault();
        onSubmit(rule, reason.trim());
      }}
    >
      <RuleSelect rules={rules} value={rule} autoFocus={startOpen} onChange={setRule} />
      <label>
        The facts, as the maker will read them
        <textarea
          value={reason}
          rows={2}
          required
          onChange={(e) => setReason(e.target.value)}
        />
      </label>
      <div className="row-actions">
        <button
          className="btn small danger"
          disabled={disabled || !rule || !reason.trim()}
        >
          Take out of the store
        </button>
        <button
          type="button"
          className="btn small ghost"
          onClick={() => {
            setOpen(false);
            onCancel?.();
          }}
        >
          Cancel
        </button>
      </div>
    </form>
  );
}

/** The outcome of an action: refused, or waiting for a second approval. */
export function ActionMessages({ error, notice }: { error?: string; notice?: string }) {
  return (
    <>
      {notice && (
        <p className="notice" role="status">
          {notice}
        </p>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </>
  );
}
