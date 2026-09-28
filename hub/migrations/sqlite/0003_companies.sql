-- Companies and their sites as records (ROADMAP M1-03, proposal 0002 §4.1, D-44).
--
-- The keys are the text probes send (`company_id`, `site` in each report), so
-- reports, heartbeats and alert keys keep working unchanged. A report naming
-- an unknown company or site registers it (reviewed_at NULL = "new"); admins
-- then give it a name, contacts and notes, or archive it.

CREATE TABLE companies (
    id            TEXT     PRIMARY KEY,   -- the company_id probes send; immutable
    name          TEXT     NOT NULL,      -- display name
    notes         TEXT,
    contact_name  TEXT,
    contact_email TEXT,
    contact_phone TEXT,
    created_at    DATETIME NOT NULL,
    reviewed_at   DATETIME,               -- NULL: registered by a report, never edited
    archived_at   DATETIME                -- hidden from the dashboard; reports still accepted
);

-- A physical location of a company (headquarters, warehouse, …).
CREATE TABLE sites (
    company_id  TEXT     NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
    id          TEXT     NOT NULL,        -- the site probes send; immutable
    name        TEXT     NOT NULL,        -- display name
    address     TEXT,
    notes       TEXT,
    created_at  DATETIME NOT NULL,
    reviewed_at DATETIME,
    PRIMARY KEY (company_id, id)
);

-- Existing data: every company and site already seen in a report.
INSERT INTO companies (id, name, created_at)
SELECT company_id, company_id, MIN(timestamp) FROM reports GROUP BY company_id;

INSERT INTO sites (company_id, id, name, created_at)
SELECT company_id, site, site, MIN(timestamp) FROM reports
WHERE site <> '' GROUP BY company_id, site;
