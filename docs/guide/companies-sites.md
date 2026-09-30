# Companies and sites

A **company** is a client you monitor. A **site** is one of its physical locations: headquarters, warehouse 1, a branch office. A company with several locations has one site per location, and each probe reports from one site, so the dashboard shows each company and, on its page, which site each probe is at.

## Where they come from

You create them in the UI, before adding probes:

- **Companies → Add company** (Owner and Admin): a display name and an **ID**, suggested from the name (`ACME Ltd` → `acme-ltd`) and editable. IDs are lowercase letters, digits and `-`.
- On the company's page, **Add site** (also Technicians, for their own companies): a name and an ID the same way. A company needs at least one site before it can have probes.

Then **Add probe** on the company's page (enabled once the company has a site) creates the enrollment code for probes at one of its sites ([install/probe.md](../install/probe.md)). The probe reports for that company and site, because the hub decided it, whatever the probe's own settings say. A probe can be renamed or moved to another site of the same company in its row on the company's page; to move it to another company, enroll it again with a code from that company.

IDs never change, because the history is stored under them. Everything else can be edited.

## Completing a record

**Companies** in the menu lists every company, with its sites and probes. Everyone sees only the companies in their scope ([users.md](users.md#which-companies-a-user-sees)). Open one to edit it (Owner, Admin, and Technicians for their own companies; Viewers and Clients can read, and Clients don't see notes or contacts):

- **Details:** display name (shown on the dashboard instead of the ID), contact name, e-mail and phone, notes. They're shown as saved; **Edit** opens the form, then **Save** or **Cancel**.
- **Sites:** a display name (e.g. "Warehouse 1" for `warehouse-1`), address and notes for each location.
- **Probes:** each probe with its site, state and version; rename it, move it to another site, or revoke it. **Add probe**, and the codes still valid, with how many probes used each (with Revoke).

## Archiving

**Archive** (Owner and Admin) hides a company from the dashboard, for example:

- a company added by mistake: enroll its probes into the right company, then archive it;
- a former client whose history you want to keep.

Archiving deletes nothing, and the company's probes can still report **and alert**. It only hides the company. **Unarchive** brings it back. To delete data, use the admin page.

## Coming later

- A timezone per site, for maintenance windows in the site's local time (M3-04).
