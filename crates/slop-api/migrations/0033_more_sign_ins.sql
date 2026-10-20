-- Microsoft and Discord sign-in (DESIGN §4), built 2026-10-20.
alter table identities drop constraint identities_provider_check;
alter table identities add constraint identities_provider_check
    check (provider in ('github', 'google', 'gitlab', 'bitbucket', 'microsoft', 'discord'));
