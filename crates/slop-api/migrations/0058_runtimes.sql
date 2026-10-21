-- Shared runtimes a release needs (slop-manifest runtimes.rs): signed into its manifest, so the
-- Slop app installs missing ones from the vendor before the game starts.
alter table releases add column runtimes text[] not null default '{}';
comment on column releases.runtimes is 'purpose: the approved shared runtimes (Visual C++, .NET Desktop, DirectX) this release needs, for the Slop app to install';
