-- Per-account storage budgets for chat (security review, 2026-10-22): what each account sent in
-- the last day, looked up by sender.
create index chat_messages_sender on chat_messages (sender_user_id, created_at);
create index chat_attachments_uploader on chat_attachments (uploader_id, created_at);
