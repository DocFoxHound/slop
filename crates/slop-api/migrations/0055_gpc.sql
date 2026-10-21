-- Global Privacy Control (LEGAL-GUIDANCE PRIV-10): a browser saying "don't sell or share my
-- data". Slop never sells or shares data; the one optional use, recommendations based on the
-- person's activity, is off by default once their browser has sent the signal, unless they
-- turned it on themselves.
alter table users add column gpc_at timestamptz;
comment on column users.gpc_at is 'purpose: when this account''s browser first sent a Global Privacy Control signal, to honour it as an opt-out and to show we did';
