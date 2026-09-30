-- Statut `sending` : un envoi réclame sa ligne (pending → sending) avant de
-- partir, pour qu'un renvoi et la vague ne l'envoient pas deux fois.
-- SQLite ne sait pas modifier un CHECK : la table est reconstruite.

CREATE TABLE assignments_new (
    edition_id  INTEGER NOT NULL REFERENCES editions (id) ON DELETE CASCADE,
    giver_id    INTEGER NOT NULL REFERENCES players (id),
    receiver_id INTEGER NOT NULL REFERENCES players (id),
    mail_status TEXT NOT NULL CHECK (mail_status IN ('pending', 'sending', 'sent', 'failed', 'none')),
    mail_error  TEXT,
    PRIMARY KEY (edition_id, giver_id),
    UNIQUE (edition_id, receiver_id)
);

INSERT INTO assignments_new (edition_id, giver_id, receiver_id, mail_status, mail_error)
SELECT edition_id, giver_id, receiver_id, mail_status, mail_error FROM assignments;

DROP TABLE assignments;
ALTER TABLE assignments_new RENAME TO assignments;

CREATE INDEX idx_assignments_pending ON assignments (mail_status) WHERE mail_status = 'pending';
