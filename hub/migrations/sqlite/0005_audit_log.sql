-- Who did what (ROADMAP M1-06, proposal 0001 §4.2, D-46). Append-only.
CREATE TABLE audit_log (
    id          INTEGER  PRIMARY KEY AUTOINCREMENT,
    at          DATETIME NOT NULL,
    user_id     TEXT,             -- NULL: the hub itself or the server console
    username    TEXT,             -- as it was then (users can be renamed or deleted later)
    actor_ip    TEXT,
    action      TEXT     NOT NULL, -- e.g. 'user.create', 'silence.delete', 'login.failed'
    target_type TEXT,             -- 'user', 'company', 'site', 'probe', 'silence', 'archive'
    target_id   TEXT,
    company_id  TEXT,             -- the company concerned, if any
    details     TEXT              -- JSON
);

CREATE INDEX idx_audit_at ON audit_log(at);
CREATE INDEX idx_audit_user ON audit_log(user_id);
