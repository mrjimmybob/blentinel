# Companies and sites

A **company** is a client you monitor. A **site** is one of its physical locations: headquarters, warehouse 1, a branch office. A company with several locations has one site per location, and each probe reports from one site, so the dashboard shows each company and, on its page, which site each probe is at.

## Where they come from

Each probe says which company and site it belongs to, in its `blentinel_probe.toml`:

```toml
[agent]
company_id = "acme"
site = "warehouse-1"
```

The first report that names a company or site the hub doesn't know **registers** it: the hub creates the record and marks it **New**. There is nothing to create beforehand, and reporting never stops for an unknown name. (From M2 on, the hub will assign probes to companies itself when they enroll; ROADMAP M2-01.)

The `company_id` and `site` a probe sends are the records' **IDs**. They never change, because the history is stored under them. Everything else can be edited.

## Completing a record

**Companies** in the menu lists every company, with its sites, probes, and a **New** badge for those nobody has looked at yet. Everyone sees only the companies in their scope ([users.md](users.md#which-companies-a-user-sees)). Open one to edit it (Owner, Admin, and Technicians for their own companies; Viewers and Clients can read, and Clients don't see notes or contacts):

- **Details:** display name (shown on the dashboard instead of the ID), contact name, e-mail and phone, notes. Saving clears the **New** badge.
- **Sites:** a display name (e.g. "Warehouse 1" for `warehouse-1`), address and notes for each location.

Later reports never overwrite what you entered.

## Archiving

**Archive** (Owner and Admin) hides a company from the dashboard, for example:

- a typo in a probe's config (`acme` and `acmee`): fix the probe's `company_id`, then archive the typo;
- a former client whose history you want to keep.

Archiving deletes nothing, and the company's probes can still report **and alert**. It only hides the company. **Unarchive** brings it back. To delete data, use the admin page.

## Coming later

- A timezone per site, for maintenance windows in the site's local time (M3-04).
