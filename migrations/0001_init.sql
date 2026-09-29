CREATE TABLE players (
    id            INTEGER PRIMARY KEY,
    email         TEXT NOT NULL UNIQUE,
    sso_user_id   INTEGER UNIQUE,
    pseudo        TEXT NOT NULL,
    has_access    INTEGER NOT NULL DEFAULT 0,
    updated_at    TEXT NOT NULL
);

CREATE TABLE editions (
    id               INTEGER PRIMARY KEY,
    year             INTEGER NOT NULL UNIQUE,
    budget_eur       INTEGER NOT NULL CHECK (budget_eur > 0),
    deadline         TEXT NOT NULL,
    status           TEXT NOT NULL CHECK (status IN ('open', 'drawn', 'failed')),
    failure_reason   TEXT,
    drawn_at         TEXT,
    reminder_sent_at TEXT,
    announced_at     TEXT,
    imported         INTEGER NOT NULL DEFAULT 0,
    created_at       TEXT NOT NULL
);

CREATE TABLE registrations (
    edition_id        INTEGER NOT NULL REFERENCES editions (id) ON DELETE CASCADE,
    player_id         INTEGER NOT NULL REFERENCES players (id),
    participating     INTEGER NOT NULL,
    answered_by_admin INTEGER NOT NULL DEFAULT 0,
    updated_at        TEXT NOT NULL,
    PRIMARY KEY (edition_id, player_id)
);

CREATE TABLE assignments (
    edition_id  INTEGER NOT NULL REFERENCES editions (id) ON DELETE CASCADE,
    giver_id    INTEGER NOT NULL REFERENCES players (id),
    receiver_id INTEGER NOT NULL REFERENCES players (id),
    mail_status TEXT NOT NULL CHECK (mail_status IN ('pending', 'sent', 'failed', 'none')),
    mail_error  TEXT,
    PRIMARY KEY (edition_id, giver_id),
    UNIQUE (edition_id, receiver_id)
);

CREATE TABLE gifts (
    id          INTEGER PRIMARY KEY,
    player_id   INTEGER NOT NULL REFERENCES players (id),
    year        INTEGER NOT NULL,
    description TEXT NOT NULL,
    photo_path  TEXT,
    created_at  TEXT NOT NULL
);

CREATE INDEX idx_gifts_player ON gifts (player_id, year DESC);
CREATE INDEX idx_assignments_pending ON assignments (mail_status) WHERE mail_status = 'pending';
