# Monthly availability reports

Each company has a **monthly availability report** (M6-05): how available its checks were, site by site, compared with the month before, its incidents with their cause and how long they lasted, and the maintenance done. It's for you and for your client: every role that sees the company sees its report, a Client too.

## Opening one

**Reports** in the menu lists the companies you see; click one. Its company page links to it too (**Monthly report**, next to *Company details*). The report opens on last month. Choose another month and click **Show**. The current month shows the figures so far.

- **Print** opens the browser's print dialog. The menu and the buttons are left out. Choose *Save as PDF* there for a PDF.
- **Download CSV** gives one row per check, for a spreadsheet: site, probe, check, kind, availability and the month before (in percent, 3 decimals), the counts of Up, Warning, Down and Unknown results and of results during maintenance, incidents, downtime in minutes, and whether the check was deleted since. Names that start with `=`, `+`, `-` or `@` get a `'` in front, so a spreadsheet never runs them as formulas.
- **Target** (optional, e.g. `99.9`): rows below it are marked in red.

## What it says

| | |
|---|---|
| **Availability** | The Up and Warning results of those that were Up, Warning or Down. Unknown results (a check behind one that was down) and results during maintenance don't count. The same rule as each check's history. |
| **Change** | Against the month before, in percentage points. |
| **Incidents** | The incidents that opened in the month (an incident that started the month before is listed, but not counted again). |
| **Downtime** | How long the checks were in an incident during the month, added up. An incident still open counts until now. A probe that stopped reporting is listed among the incidents but adds no downtime to its checks. |
| **Cause** | What the hub saw when the incident opened, e.g. `Down: Request timed out`. Never the notes people wrote. |
| **Maintenance** | The openings of the company's maintenance windows in the month, in each window's time zone. Windows deleted since aren't listed; the results during maintenance are still counted in each row. |

Every site's figures cover **its own month, in its time zone** (set on the company's page): March in Madrid starts at midnight in Madrid. The company's figure adds up its sites. Checks of probes that are no longer enrolled are under **Probes no longer enrolled** (in UTC), and checks deleted since are marked *deleted*: their history stays.

The figures come from the hourly history summaries ([data-retention](data-retention.md)), kept 2 years by default. A month older than that is read from the daily summaries, which are in UTC days, so a site's month is only roughly its local month, and the report says so. Changing a site's time zone changes the months it reports, back to its first month.

## Not yet

Scheduled e-mail of the report to a company's contacts is planned (ROADMAP, M6-05 schedule).
