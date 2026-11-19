-- The beta's invite code (2026-11-19). Anyone can sign up, make store pages and send releases
-- through the checks, but going public (publishing a release, putting an app in the store the
-- first time, asking for a private app to go public) needs an invite code while it's switched on.
-- One code for everyone, set by SLAPorSLOP staff and changed when too many people have it. It
-- isn't a secret: it only paces the beta. An account that entered it keeps access after it
-- changes. Off until staff switch it on.
create table publish_invite (
    id          boolean primary key default true check (id),
    required    boolean not null default false,
    code        text not null default '',
    updated_at  timestamptz not null default now(),
    updated_by  uuid references users (id) on delete set null
);
comment on table publish_invite is
    'purpose: whether going public needs the beta invite code, and the code';
comment on column publish_invite.updated_by is
    'purpose: accountability: the staff member who last changed the invite settings (also in the audit log)';
insert into publish_invite (id) values (true);

alter table users add column publish_invited_at timestamptz;
comment on column users.publish_invited_at is
    'purpose: when the account first entered the beta invite code, so it can publish while the code is required';

-- Makers already out there keep publishing: anyone with an app that has been in the store, or a
-- release that has been published.
update users u set publish_invited_at = now()
where exists (
    select 1 from apps a
    where a.maker_id = u.id
      and (a.status <> 'draft'
           or exists (select 1 from releases r
                      where r.app_id = a.id
                        and (r.published_at is not null or r.status in ('approved', 'revoked'))))
);
