-- The daily digest e-mail (ROADMAP M3-09, proposal 0005 §2.6, D-71): when,
-- in which time zone, to which e-mail channels, and the local date it last
-- went out (so a restart neither repeats nor skips it).
CREATE TABLE digest_settings (
    id         INTEGER  PRIMARY KEY CHECK (id = 1),
    hour       INTEGER  NOT NULL,              -- 0-23, local
    timezone   TEXT     NOT NULL,              -- IANA
    channels   TEXT     NOT NULL,              -- JSON array of channel IDs
    last_sent  TEXT,                           -- YYYY-MM-DD, local
    updated_by TEXT,
    updated_at DATETIME NOT NULL
);
