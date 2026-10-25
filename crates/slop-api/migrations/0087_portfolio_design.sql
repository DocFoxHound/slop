-- A creative's own People page design (people/design.rs): the checked and rebuilt
-- .slop/page/index.html and style.css, read at the same commit as the portfolio and reviewed
-- with it. Null: no design, the standard layout.
alter table portfolio_sets add column design jsonb;
