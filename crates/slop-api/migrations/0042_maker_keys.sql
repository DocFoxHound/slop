-- Makers' own signing keys, for builds they make and attach by hand (SECURITY §5.3), built
-- 2026-10-20. An Ed25519 SSH key: files are signed with `ssh-keygen -Y sign -n slop-release`.
create table maker_keys (
    id                  uuid primary key,
    user_id             uuid not null references users (id) on delete cascade,
    -- `ssh-ed25519 <base64>`, without the comment.
    public_key          text not null,
    fingerprint         text not null,
    name                text not null check (char_length(name) between 1 and 60),
    created_at          timestamptz not null default now(),
    -- Usable only from here: 72 hours after it's added (SECURITY §5.3), so a stolen session
    -- can't add a key and sign a release with it before the maker hears about it.
    active_from         timestamptz not null,
    revoked_at          timestamptz,
    unique (user_id, fingerprint)
);

comment on table maker_keys is 'purpose: checking that hand-attached release files were signed by their maker (public keys only)';

-- The maker's signature on a file, when it has no attestation: the key and the signature.
alter table release_files add column maker_signature jsonb;
