# Phase 4E: Vault Configuration and Variable Resolution

## Overview

This document explains how the probe enrollment code (`probe_staging_code`) is stored and accessed by both the control host and staging environment, reusing the existing Vault structure.

## Architecture

### Secret Storage

- **Location:** `host_vars/{control_host}/vault.yml` (e.g., `atlas` or `lab-control`)
- **File:** Encrypted with Ansible Vault, same password and backup process as `deploy_agent_github_token`
- **Content:** `probe_staging_code: BLT1-…` (added alongside existing secrets)
- **Access:** Decrypted by Ansible when `--ask-vault-pass` is used during playbook runs

### Variable Resolution (Lazy Evaluation)

The non-encrypted `group_vars/all.yml` file uses Jinja2's lazy evaluation to reference the encrypted secret:

```yaml
probe_staging_code: |
  {{
    hostvars.get('atlas', {}).get('probe_staging_code', '') or
    hostvars.get('lab-control', {}).get('probe_staging_code', '')
  }}
```

**How it works:**

1. **Inventory loading phase:** `group_vars/all.yml` is loaded by Ansible, but the Jinja2 expression is **not evaluated yet** (lazy).
2. **Vault decryption phase:** When `--ask-vault-pass` is used, Ansible decrypts all encrypted vars (including `host_vars/atlas/vault.yml`).
3. **Task execution phase:** When the probe role runs as a task and accesses `probe_staging_code`, the Jinja2 expression is evaluated at that moment.
4. **hostvars resolution:** By task execution time, `hostvars['atlas']` contains all decrypted variables from `atlas`, including `probe_staging_code` from its Vault.
5. **Return value:** The expression returns the secret from whichever control host exists (`atlas` or `lab-control`).

### Playbook Hosts

- **staging.yml:** Runs on `staging` group → inherits `probe_staging_code` from `group_vars/all` → probe role runs
- **lab-control.yml (or atlas.yml):** Runs on `lab_control` (or `prod_control`) group → inherits `probe_staging_code` from `group_vars/all` → probe role runs

Both hosts can access the same secret stored in a single Vault file.

## No Duplication

- **Single source of truth:** `probe_staging_code` is encrypted in only one Vault file (`host_vars/{control_host}/vault.yml`)
- **No plaintext copies:** Never stored in `hosts.yml`, `.yml` files not in Vault, or Git
- **One password:** The operator uses the same Vault password for all secrets

## Verification (Without Modifying Live Infrastructure)

To verify variable resolution works correctly:

1. **Syntax check:** The probe role's variable references are idempotent and safe:
   - Line 5: `when: not probe_staging_code or probe_staging_code | length == 0` → skips if not set
   - Line 92: `cmd: "sudo /tmp/blentinel-probe install --code {{ probe_staging_code }}"` → uses the resolved value
   - No direct Vault file access; all decryption handled by Ansible

2. **Dry-run on staging (optional):**
   ```bash
   # Verify variable loading without making changes
   ansible-playbook ops/staging.yml \
     --ask-vault-pass --ask-become-pass \
     --diff --check \
     -e "probe_staging_code=TEST-CODE-FOR-VERIFICATION"
   ```
   This overrides the Vault value for testing and shows what would happen.

3. **Play facts:** After running or during a dry-run, Ansible logs the hostvars resolution. The control host's variables (including `probe_staging_code`) appear in logs with `no_log: true` stripped if viewing raw output.

## Migration From Previous Approach

If Phase 4E was previously set up with `group_vars/all/vault.yml`:

1. **Do NOT delete** the old Vault file immediately.
2. **Add to existing Vault:** Copy `probe_staging_code: BLT1-…` into `host_vars/{control_host}/vault.yml`.
3. **Update group_vars:** Replace `group_vars/all/vault.yml` contents with the non-encrypted `group_vars/all.yml` (hostvars reference).
4. **Test:** Run playbooks with `--ask-vault-pass` to verify variable resolution.
5. **Cleanup:** After verification, remove the old `group_vars/all/vault.yml` file.

## Related Decisions

- **D-109+:** Phase 4E infrastructure and secret protection strategy
- **D-107:** Deployment agent and control host Vault (Phase 4D, already deployed)

## Files

- `ops/group_vars/all.yml` — Hostvars reference (non-encrypted)
- `ops/group_vars/staging.yml` — Staging-specific config with probe settings
- `ops/lab-control.yml` — Control host playbook (includes probe role)
- `ops/staging.yml` — Staging playbook (includes probe role)
- `ops/roles/probe/defaults/main.yml` — Probe role defaults (empty probe_staging_code as fallback)
- `docs/ops.md` — Installation and setup instructions
