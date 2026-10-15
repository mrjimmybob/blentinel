## What & why

<!-- Roadmap item (e.g. M1-02) and/or proposal link -->

## Principle check ([PRINCIPLES.md](../docs/PRINCIPLES.md))

- [ ] FP-1: nothing requires the hub to initiate a connection to a probe
- [ ] FP-2: no new probe runtime/library/tool dependency; no files outside the probe directory
- [ ] FP-3: no trust decision relies on unauthenticated or probe-supplied data
- [ ] FP-4: probe still checks + buffers with the hub offline
- [ ] PP-1/PP-2: no new manual install/management step (file edit, key copy, rebuild)

## Definition of Done ([ENGINEERING.md](../docs/ENGINEERING.md#1-definition-of-done))

- [ ] Tests added/updated (fixing a known issue? remove its `#[ignore]`)
- [ ] `blentinelmake ci` passes locally; the CI gate passes here ([docs/ci.md](../docs/ci.md))
- [ ] Docs updated (list pages):
- [ ] `CHANGELOG.md` entry under *Unreleased*
- [ ] Migration included (if schema changed)
- [ ] `docs/ROADMAP.md` status updated
