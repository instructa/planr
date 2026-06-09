# Import Existing Planr Data

```bash
planr project init "Imported Project"
planr import /path/to/repo-or-planr-dir
planr map show --json
```

Import reads project packs, product plans, build plans, status scopes, and review artifacts when present. Originals are not deleted or rewritten.

Use JSON export for backups and reusable templates:

```bash
planr export --include-plans --include-logs --template-name "API backend slice" --tag api --out planr-backup.json
planr import planr-backup.json --preview
planr import planr-backup.json --confirm
```

JSON imports are preview-first. Preview reports compatibility metadata, create counts, and conflicting item ids before mutating the current project.

Planr packages are local-first JSON. For encrypted sharing, review the JSON locally and encrypt the file with your team's standard tool, for example:

```bash
age -o planr-backup.json.age -r <recipient> planr-backup.json
gpg -c planr-backup.json
```

Planr does not require a hosted share service for V1.1.
