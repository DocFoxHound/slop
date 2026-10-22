-- An official team's GitHub organization is unique only once verified (security review,
-- 2026-10-22): before, whoever named an organization first held it, and its real owners couldn't
-- make their team. Unverified ones show as not verified and never get the badge.
drop index teams_one_per_org;
create unique index teams_one_verified_per_org on teams (lower(github_org))
    where kind = 'official' and verified_at is not null;
