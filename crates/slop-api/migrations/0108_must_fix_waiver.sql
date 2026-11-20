-- A MUST FIX blocks Publish (owner decision 2026-11-20, publish.rs, fixes.rs): while a release
-- has `must` items, its maker can't publish it. Some are false alarms (a "possible secret" that
-- isn't one), so the maker can ask SLAPorSLOP staff about it (holds.rs), and a staff member
-- signed in themselves can allow publishing it anyway: recorded here, on that release only. A new
-- release is checked afresh and isn't covered.
alter table releases
    add column must_fix_waived_at timestamptz,
    add column must_fix_waived_by text;
comment on column releases.must_fix_waived_at is
    'purpose: when SLAPorSLOP staff allowed this release to be published although its checks found something that must be fixed';
comment on column releases.must_fix_waived_by is
    'purpose: accountability: which staff member (name and account ID) allowed publishing it anyway';
