-- GitHub's numeric IDs for an app's repository and its owner (repo_identity.rs). A name can be
-- taken over (a renamed account, a deleted repository registered again); the IDs can't, so
-- releases are only accepted from the repository the app was linked to.
alter table apps
    add column github_repo_id bigint,
    add column github_owner_id bigint;
