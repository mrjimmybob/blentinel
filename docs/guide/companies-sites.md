# Companies and sites

A **company** is a client you monitor. A **site** is one of its physical locations: headquarters, warehouse 1, a branch office. A company with several locations has one site per location, and each probe reports from one site, so the dashboard shows each company and, on its page, which site each probe is at.

## Where they come from

You create them in the UI, before adding probes:

- **Companies → Add company** (Owner and Admin): a display name and an **ID**, suggested from the name (`ACME Ltd` → `acme-ltd`) and editable. IDs are lowercase letters, digits and `-`.
- On the company's page, **Add site** (also Technicians, for their own companies): a name and an ID the same way. A company needs at least one site before it can have probes.

Then **Add probe** on the company's page (enabled once the company has a site) creates the enrollment code for probes at one of its sites ([install/probe.md](../install/probe.md)). The probe reports for that company and site, because the hub decided it, whatever the probe's own settings say. A probe can be renamed or moved to another site of the same company in its row on the company's page.

**A probe never changes company** (D-116). A probe registration belongs to the company it was enrolled in, for good. When a machine goes to another company:

1. **Revoke** its probe on the old company's page. This retires the registration: it can't report again, and it is kept, with all its history.
2. **Add probe** in the new company, and run `probe enroll <code>` (or `probe install --code <code>`) on the machine. The installed probe is reused; it makes a **new identity**, so the new company gets a new probe with no checks and no history, and builds its own from its first report.

The old company keeps everything the probe reported (results, incidents, availability, the monthly reports of those months) for as long as retention keeps it, and the new company sees none of it. The hub refuses to enroll an identity that belongs to another company, so the old registration must be retired first. Only **Remove probe** on the admin page deletes a probe's history.

**Not built yet:** until D-116 is implemented, the hub still moves an identity enrolled with another company's code, as D-47 and D-115 did.

IDs never change, because the history is stored under them. Everything else can be edited.

## Completing a record

**Companies** in the menu lists every company, with its sites and probes. Everyone sees only the companies in their scope ([users.md](users.md#which-companies-a-user-sees)). Open one to edit it (Owner, Admin, and Technicians for their own companies; Viewers and Clients can read, and Clients don't see notes or contacts):

- **Details:** display name (shown on the dashboard instead of the ID), contact name, e-mail and phone, notes. They're shown as saved; **Edit** opens the form, then **Save** or **Cancel**.
- **Sites:** a display name (e.g. "Warehouse 1" for `warehouse-1`), address and notes for each location.
- **Probes:** each probe with its site, state and version; rename it, move it to another site, or revoke it (retire it for good: its history stays). **Add probe**, and the codes still valid, with how many probes used each (with Revoke).

## Archiving

**Archive** (Owner and Admin) hides a company from the dashboard, for example:

- a company added by mistake: enroll its probes into the right company, then archive it;
- a former client whose history you want to keep.

Archiving deletes nothing, and the company's probes can still report **and alert**. It only hides the company. **Unarchive** brings it back. To delete data, use the admin page.

## Coming later

- A timezone per site, for maintenance windows in the site's local time (M3-04).
