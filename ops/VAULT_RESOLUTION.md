# Phase 4E: Vault Configuration and Variable Resolution

## Overview

This document explains how the probe enrollment code (`probe_staging_code`) is stored and accessed by both the control host and staging environment, reusing the existing Vault structure.

## Architecture

### Secret Storage

- **Location:** `host_vars/atlas/vault.yml`
- **File:** Encrypted with Ansible Vault, same password and backup process as `deploy_agent_github_token`
- **Variable name:** `vault_probe_staging_code: BLT1-…` (distinct from the global `probe_staging_code` to avoid recursion)
- **Added alongside:** existing secrets like `deploy_agent_github_token`
- **Access:** Decrypted by Ansible when `--ask-vault-pass` is used during playbook runs

### Variable Resolution (Distinct Vault Variable)

The non-encrypted `group_vars/all.yml` file uses a distinct Vault variable name (`vault_probe_staging_code`) to avoid recursive resolution:

```yaml
probe_staging_code: "{{ hostvars.get('atlas', {}).get('vault_probe_staging_code', '') }}"
```

**Why distinct naming matters:**

- **Vault variable:** `vault_probe_staging_code` is stored encrypted in `host_vars/atlas/vault.yml`
- **Global variable:** `probe_staging_code` is defined in `group_vars/all.yml` and references the Vault variable
- **Avoids recursion:** If we named both `probe_staging_code`, Ansible might resolve the global definition instead of the Vault value, causing circular references

**How it works:**

1. **Inventory loading phase:** `group_vars/all.yml` is loaded by Ansible, but the Jinja2 expression is **not evaluated yet** (lazy).
2. **Vault decryption phase:** When `--ask-vault-pass` is used, Ansible decrypts all encrypted vars (including `host_vars/atlas/vault.yml`).
3. **Task execution phase:** When the probe role runs as a task and accesses `probe_staging_code`, the Jinja2 expression is evaluated at that moment.
4. **hostvars resolution:** By task execution time, `hostvars['atlas']` contains all decrypted variables from `atlas`, including `vault_probe_staging_code` from its Vault.
5. **Return value:** The expression retrieves `vault_probe_staging_code` from atlas and returns it as `probe_staging_code`.

### Playbook Hosts

- **staging.yml:** Runs on `staging` group → inherits `probe_staging_code` from `group_vars/all` → probe role runs
- **lab-control.yml (or atlas.yml):** Runs on `lab_control` (or `prod_control`) group → inherits `probe_staging_code` from `group_vars/all` → probe role runs

Both hosts can access the same secret stored in a single Vault file.

## No Duplication

- **Single source of truth:** `probe_staging_code` is encrypted in only one Vault file (`host_vars/{control_host}/vault.yml`)
- **No plaintext copies:** Never stored in `hosts.yml`, `.yml` files not in Vault, or Git
- **One password:** The operator uses the same Vault password for all secrets

## Verification (Without Modifying Live Infrastructure)

### Test Case 1: Vault Variable Present

When `vault_probe_staging_code` is present in `host_vars/atlas/vault.yml`:

```bash
# Dry-run with verbose output to see variable resolution
ansible-playbook ops/staging.yml \
  --ask-vault-pass --ask-become-pass \
  --diff --check \
  -vvv 2>&1 | grep -i "probe_staging_code\|vault_probe_staging_code"
```

**Expected behavior:**
- `probe_staging_code` resolves to the decrypted value from `vault_probe_staging_code`
- Both `atlas` and `blentinel-staging` hosts see the same code
- Probe role skips installation only if code is explicitly empty (should NOT skip if Vault has a value)

### Test Case 2: Vault Variable Absent

When `vault_probe_staging_code` is not set in the Vault:

```bash
# Override with empty value to simulate missing Vault variable
ansible-playbook ops/staging.yml \
  --ask-vault-pass --ask-become-pass \
  --diff --check \
  -e "vault_probe_staging_code=" \
  -vvv 2>&1 | grep -i "probe_staging_code"
```

**Expected behavior:**
- `probe_staging_code` resolves to empty string (the `default('')` fallback)
- Probe role skips installation with "Skip probe installation (no enrollment code)" message
- **No recursion errors** or warnings about undefined variables

### Additional Verification

1. **Syntax check:** The probe role's variable references are idempotent and safe:
   - Line 5: `when: not probe_staging_code or probe_staging_code | length == 0` → skips if not set
   - Line 92: `cmd: "sudo /tmp/blentinel-probe install --code {{ probe_staging_code }}"` → uses the resolved value
   - No direct Vault file access; all decryption handled by Ansible

2. **Role defaults:** Probe role defaults `probe_staging_code` to empty string, so the chain is:
   - Vault has `vault_probe_staging_code` → `group_vars/all.yml` exposes as `probe_staging_code`
   - Vault does not have it → `group_vars/all.yml` falls back to empty string
   - Both cases skip recursion by using distinct variable names

3. **Variable precedence:** Verify Ansible's resolution order:
   ```bash
   ansible localhost -m debug -a var=probe_staging_code -i ops/inventory.example/hosts.yml
   ```

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
