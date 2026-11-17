-- Fingerprints of the programs and scripts inside release files (deploy/scanner/hashes.py), so
-- ClamAV's newer signatures can be checked against them every six hours (rescan.rs) instead of
-- downloading and scanning every live release again each week (decided 2026-11-17: with
-- thousands of apps, that costs too much). Most of ClamAV's signatures are fingerprints like
-- these: a whole file's MD5, SHA-1 or SHA-256 with its size, or one Windows program section's.
create table release_file_fingerprints (
    release_id  uuid not null,
    platform    text not null,
    file        text not null check (char_length(file) <= 1000),
    -- null for the whole file, else the section's place in the program.
    section     smallint check (section >= 0),
    size        bigint not null check (size >= 0),
    md5         text not null check (md5 ~ '^[0-9a-f]{32}$'),
    sha1        text not null check (sha1 ~ '^[0-9a-f]{40}$'),
    sha256      text not null check (sha256 ~ '^[0-9a-f]{64}$'),
    foreign key (release_id, platform) references release_files (release_id, platform) on delete cascade
);
create index release_file_fingerprints_file on release_file_fingerprints (release_id, platform);
comment on table release_file_fingerprints is
    'purpose: checking released programs against newer malware signatures without downloading them again';

-- Signatures that already matched a file: each acts once (a re-scan, or a hold), not every six
-- hours. A file whose re-scan staff kept, or whose finding they cleared, isn't raised again.
create table fingerprint_matches (
    release_id  uuid not null,
    platform    text not null,
    signature   text not null check (char_length(signature) <= 300),
    file        text not null check (char_length(file) <= 1000),
    found_at    timestamptz not null default now(),
    primary key (release_id, platform, signature),
    foreign key (release_id, platform) references release_files (release_id, platform) on delete cascade
);
comment on table fingerprint_matches is
    'purpose: acting once on each newer signature that matches a released program';

-- When the fingerprints were last checked, and against which signatures.
create table fingerprint_checks (
    id          boolean primary key default true check (id),
    signatures  text not null,
    checked_at  timestamptz not null default now()
);
comment on table fingerprint_checks is
    'purpose: checking fingerprints again only when they are due';
