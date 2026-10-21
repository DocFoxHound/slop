-- Known-bad links (bad_links.rs, SECURITY §11): public lists of malware and phishing addresses,
-- refreshed daily, checked against store pages and creative profiles. Matches become abuse
-- signals for a person; nothing is removed automatically.
create table bad_links (
    url         text primary key,
    host        text not null,
    source      text not null,
    listed_at   timestamptz not null default now()
);
create index bad_links_host on bad_links (host);
comment on table bad_links is 'purpose: addresses public lists name as malware or phishing, to point out links to them on Slop pages; no personal data';
