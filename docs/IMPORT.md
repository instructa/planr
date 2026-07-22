# Planr Packages

```bash
planr project init "New Project"
planr export --include-plans --include-logs --template-name "API backend slice" --tag api --out planr-package.json
planr import planr-package.json --preview
planr import planr-package.json --confirm
```

Planr packages are local-first JSON files created by `planr export`. They carry graph items, links, contexts, optional logs, optional eval evidence refs, optional plan file snapshots, and review artifacts.

Imports are preview-first. Preview reports package metadata, create counts, and conflicting item ids before mutating the current project.

Eval evidence exports only when logs are included. Packages carry immutable eval suite snapshots, runs, case results, raw samples, comparisons, invalidations, and evidence refs so imported refs resolve to local eval records and comparison verdicts can be reproduced from restored run evidence. Import preflights the whole eval graph before inserting package rows: duplicate immutable ids inside the package must be byte/field-identical, suite/run/comparison/invalidation/ref dependencies must resolve, case sample ids must match nested samples, and eval refs must have `closure_authority: false`. Existing immutable eval ids are accepted only when their stored content exactly matches the package, and imported eval timestamps/provenance are preserved. Review refs must point at a real `work_type=review` item linked to the claimed item. Restoring audit provenance never closes work or changes map ownership.

Planr packages are local-first JSON. For encrypted sharing, review the JSON locally and encrypt the file with your team's standard tool, for example:

```bash
age -o planr-backup.json.age -r <recipient> planr-backup.json
gpg -c planr-backup.json
```

Planr does not require a hosted share service for V1.1.
