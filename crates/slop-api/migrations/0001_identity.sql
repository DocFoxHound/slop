-- Identity: country policies, accounts, sign-in identities and sessions.
--
-- Record of processing (LEGAL-GUIDANCE PRIV-1): every column that holds personal data has a
-- `purpose:` comment. Update docs/RECORD-OF-PROCESSING.md in the same change as any new field.

-- Age and legal rules per country (LEGAL-GUIDANCE AGE-1, AGE-2). Feature code reads these rows
-- instead of hard-coding country rules.
create table country_policies (
    country_code        char(2) primary key check (country_code ~ '^[A-Z]{2}$'),
    name                text not null,
    signups_allowed     boolean not null,
    min_age             smallint not null check (min_age >= 16),
    under18_view_default boolean not null,
    payments_allowed    boolean not null
);

comment on table country_policies is
    'Per-country age and legal policy (AGE-1). Launch regions: US, EU, UK (DESIGN §17).';

insert into country_policies (country_code, name, signups_allowed, min_age, under18_view_default, payments_allowed)
values
    ('US', 'United States', true, 16, false, true),
    ('GB', 'United Kingdom', true, 16, true, true),
    ('AT', 'Austria', true, 16, false, true),
    ('BE', 'Belgium', true, 16, false, true),
    ('BG', 'Bulgaria', true, 16, false, true),
    ('HR', 'Croatia', true, 16, false, true),
    ('CY', 'Cyprus', true, 16, false, true),
    ('CZ', 'Czechia', true, 16, false, true),
    ('DK', 'Denmark', true, 16, false, true),
    ('EE', 'Estonia', true, 16, false, true),
    ('FI', 'Finland', true, 16, false, true),
    ('FR', 'France', true, 16, false, true),
    ('DE', 'Germany', true, 16, false, true),
    ('GR', 'Greece', true, 16, false, true),
    ('HU', 'Hungary', true, 16, false, true),
    ('IE', 'Ireland', true, 16, false, true),
    ('IT', 'Italy', true, 16, false, true),
    ('LV', 'Latvia', true, 16, false, true),
    ('LT', 'Lithuania', true, 16, false, true),
    ('LU', 'Luxembourg', true, 16, false, true),
    ('MT', 'Malta', true, 16, false, true),
    ('NL', 'Netherlands', true, 16, false, true),
    ('PL', 'Poland', true, 16, false, true),
    ('PT', 'Portugal', true, 16, false, true),
    ('RO', 'Romania', true, 16, false, true),
    ('SK', 'Slovakia', true, 16, false, true),
    ('SI', 'Slovenia', true, 16, false, true),
    ('ES', 'Spain', true, 16, false, true),
    ('SE', 'Sweden', true, 16, false, true);

create table users (
    id                  uuid primary key,
    display_name        text not null check (char_length(display_name) between 1 and 40),
    email               text,
    country_code        char(2) not null references country_policies (country_code),
    age_16_confirmed_at timestamptz not null,
    created_at          timestamptz not null default now()
);

comment on column users.display_name is 'purpose: shown on the profile, store pages, reviews and votes';
comment on column users.email is 'purpose: account and product emails; from the sign-in provider, verified only';
comment on column users.country_code is 'purpose: choosing the age and legal policy (AGE-1); stated by the user';
comment on column users.age_16_confirmed_at is 'purpose: proof the user confirmed they are 16+ at sign-up; no date of birth is stored';

-- One row per sign-in provider account. Accounts are never linked or merged because an email
-- matches (SECURITY §4.1); a provider is linked only from a signed-in session.
create table identities (
    provider            text not null check (provider in ('github')),
    provider_subject    text not null,
    user_id             uuid not null references users (id) on delete cascade,
    created_at          timestamptz not null default now(),
    last_sign_in_at     timestamptz not null default now(),
    primary key (provider, provider_subject)
);

create index identities_user_id on identities (user_id);

comment on column identities.provider_subject is 'purpose: sign-in; the provider''s stable account ID';

-- In-flight OAuth sign-ins. Single use, short-lived. Keyed by a hash of the `state` value, which
-- is also held in a cookie on the browser that started the flow.
create table oauth_flows (
    state_hash          bytea primary key,
    provider            text not null,
    pkce_verifier       text not null,
    return_to           text not null,
    expires_at          timestamptz not null
);

-- A provider sign-in with no Slop account yet. The user finishes sign-up (16+ confirmation and
-- country) before an account exists.
create table pending_signups (
    token_hash          bytea primary key,
    provider            text not null,
    provider_subject    text not null,
    suggested_name      text not null,
    email               text,
    return_to           text not null,
    expires_at          timestamptz not null
);

comment on column pending_signups.email is 'purpose: carried into the new account; deleted when the row expires';

create table sessions (
    id                  uuid primary key,
    user_id             uuid not null references users (id) on delete cascade,
    token_hash          bytea not null unique,
    user_agent          text,
    created_at          timestamptz not null default now(),
    last_seen_at        timestamptz not null default now(),
    expires_at          timestamptz not null,
    revoked_at          timestamptz
);

create index sessions_user_id on sessions (user_id);

comment on column sessions.user_agent is 'purpose: lets the user recognise the session on the devices page (SECURITY §4.2)';
