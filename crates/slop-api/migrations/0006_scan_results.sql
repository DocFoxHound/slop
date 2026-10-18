-- Scan results per release file (DESIGN §10). Results follow the design's scale; `skipped`
-- means the server runs without a scanner (test mode) and the file was not scanned.
alter table release_files add column scan_result text
    check (scan_result in ('passed', 'note', 'on_hold', 'rejected', 'blocked', 'skipped'));
alter table release_files add column scan_detail text;
