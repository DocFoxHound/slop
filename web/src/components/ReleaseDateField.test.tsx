// SPDX-License-Identifier: AGPL-3.0-or-later
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it } from "vitest";

import { ReleaseDateField, releaseDateChanges, releaseDateValue, type ReleaseDateValue } from "./ReleaseDateField";

const saved = { stage: "beta" as const, release_date: "2026-12-05", release_date_precision: "day" as const };

describe("releaseDateChanges", () => {
  it("sends nothing when the date is as saved, so other changes save even once it has passed", () => {
    expect(releaseDateChanges(releaseDateValue(saved), saved)).toEqual({});
    expect(releaseDateChanges({ date: "", precision: "day" }, { ...saved, release_date: null, release_date_precision: null })).toEqual({});
    const month = { ...saved, release_date: "2026-12-01", release_date_precision: "month" as const };
    expect(releaseDateChanges({ date: "2026-12-01", precision: "month" }, month)).toEqual({});
  });

  it("sends a new date with its precision; a month as its first day", () => {
    expect(releaseDateChanges({ date: "2027-01-10", precision: "day" }, saved)).toEqual({
      release_date: "2027-01-10",
      release_date_precision: "day",
    });
    expect(releaseDateChanges({ date: "2026-12-05", precision: "month" }, saved)).toEqual({
      release_date: "2026-12-01",
      release_date_precision: "month",
    });
  });

  it("clears a saved date, and a released app sends none", () => {
    expect(releaseDateChanges({ date: "", precision: "day" }, saved)).toEqual({ release_date: null });
    expect(releaseDateChanges({ date: "2027-01-10", precision: "day" }, { ...saved, stage: "released" })).toEqual({});
    // A new app: nothing to clear.
    expect(releaseDateChanges({ date: "", precision: "day" }, { stage: "coming_soon", release_date: null, release_date_precision: null })).toEqual({});
  });
});

function Form({ start, passed = false }: { start: ReleaseDateValue; passed?: boolean }) {
  const [value, setValue] = useState(start);
  return (
    <>
      <ReleaseDateField value={value} onChange={setValue} passed={passed} now={Date.parse("2026-11-19T12:00:00Z")} />
      <output>{JSON.stringify(value)}</output>
    </>
  );
}

describe("ReleaseDateField", () => {
  afterEach(cleanup);

  it("shows what players will see, takes Month only, and clears", () => {
    render(<Form start={{ date: "2026-12-05", precision: "day" }} />);
    expect(screen.getByText(/Players see “Releases Dec 5”/)).toBeTruthy();
    fireEvent.click(screen.getByLabelText("Month only"));
    expect(screen.getByText(/Players see “Releases December 2026”/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Clear the date" }));
    expect(screen.getByRole("status", { hidden: true }).textContent).toBe('{"date":"","precision":"month"}');
    expect(screen.getByText(/No date: players see none/)).toBeTruthy();
  });

  it("tells its maker when the date has passed", () => {
    render(<Form start={{ date: "2026-10-01", precision: "day" }} passed />);
    expect(screen.getByRole("alert").textContent).toMatch(/has passed, so players no longer see it/);
  });
});
