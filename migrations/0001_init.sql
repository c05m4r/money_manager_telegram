-- Copyright (C) 2026 Marcos Gabriel Miller
-- Bot-owned tables. teloxide's SqliteStorage creates `teloxide_dialogues` on its own.

CREATE TABLE IF NOT EXISTS sessions (
    telegram_user_id INTEGER PRIMARY KEY,
    chat_id          INTEGER NOT NULL,
    user_uuid        TEXT NOT NULL,
    username         TEXT NOT NULL,
    role             TEXT NOT NULL,
    jwt              TEXT,
    jwt_expires_at   INTEGER,
    -- nonce (12 bytes) || AES-256-GCM(identifier \0 password); NULL when STORE_CREDENTIALS=false
    credentials      BLOB,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS short_refs (
    telegram_user_id INTEGER NOT NULL,
    kind             TEXT NOT NULL,
    position         INTEGER NOT NULL,
    target           TEXT NOT NULL,
    PRIMARY KEY (telegram_user_id, kind, position)
);

CREATE TABLE IF NOT EXISTS list_state (
    telegram_user_id INTEGER NOT NULL,
    kind             TEXT NOT NULL,
    filters_json     TEXT NOT NULL,
    PRIMARY KEY (telegram_user_id, kind)
);

CREATE TABLE IF NOT EXISTS login_failures (
    telegram_user_id INTEGER NOT NULL,
    failed_at        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_login_failures_user ON login_failures (telegram_user_id, failed_at);
