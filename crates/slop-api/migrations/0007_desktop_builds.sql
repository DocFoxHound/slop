-- Builds of the Slop desktop app itself, for its auto-updater (SECURITY §7: self-updates are
-- signed and verified). CI uploads each build; the server signs it with the updater key.
create table desktop_builds (
    platform            text not null check (platform in ('windows', 'mac', 'linux')),
    version             text not null,
    file_name           text not null,
    sha256              text not null,
    size_bytes          bigint not null,
    -- Tauri updater signature (base64 minisign signature box).
    signature           text not null,
    commit_sha          text not null,
    created_at          timestamptz not null default now(),
    primary key (platform, version)
);
