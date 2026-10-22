-- Commits may be as large as chat.rs allows (MAX_COMMIT_BODY, 2 MiB): one that adds many devices
-- at once is bigger than the 64 KiB the table first allowed, and failed with a server error.
-- Messages stay limited to 32 KiB by the API.
alter table chat_messages drop constraint chat_messages_body_check;
alter table chat_messages add constraint chat_messages_body_check check (length(body) <= 2097152);
