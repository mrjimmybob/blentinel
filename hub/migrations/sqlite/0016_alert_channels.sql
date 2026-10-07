-- Alert channels and routing, managed in the UI (ROADMAP M3-03, proposal
-- 0005 §2.6, D-04, D-66). Secrets are sealed with the hub's data key
-- (hub_data.key), never stored in the clear.

-- The hub's mail server and its address for links (at most one row).
CREATE TABLE alert_settings (
    id                   INTEGER  PRIMARY KEY CHECK (id = 1),
    hub_url              TEXT     NOT NULL,
    smtp_server          TEXT     NOT NULL,
    smtp_port            INTEGER  NOT NULL,
    smtp_security        TEXT     NOT NULL,      -- starttls | tls | none
    smtp_username        TEXT     NOT NULL,
    smtp_from            TEXT     NOT NULL,
    smtp_password_sealed TEXT,                   -- NULL: no password
    updated_by           TEXT,
    updated_at           DATETIME NOT NULL
);

-- Where alerts can go.
CREATE TABLE alert_channels (
    id               INTEGER  PRIMARY KEY AUTOINCREMENT,
    name             TEXT     NOT NULL,
    kind             TEXT     NOT NULL,          -- email | webhook | teams | slack | discord | telegram
    enabled          INTEGER  NOT NULL,
    target           TEXT     NOT NULL,          -- addresses, a webhook URL, a chat ID; '' otherwise
    secret_sealed    TEXT,                       -- signing key, chat webhook URL, bot token
    created_at       DATETIME NOT NULL,
    last_delivery_at DATETIME,
    last_error       TEXT                        -- NULL: the last delivery worked
);

-- Which alerts go where, in order (model::RouteRule as JSON).
CREATE TABLE alert_routes (
    position INTEGER PRIMARY KEY,
    rule     TEXT    NOT NULL
);
