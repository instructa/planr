# Planr Packages

```bash
planr project init "New Project"
planr export --include-plans --include-logs --template-name "API backend slice" --tag api --out planr-package.json
planr import planr-package.json --preview
planr import planr-package.json --confirm
```

Planr packages are local-first JSON files created by `planr export`. They carry graph items, links, contexts, optional logs, optional plan file snapshots, and review artifacts.

Imports are preview-first. Preview reports package metadata, create counts, and conflicting item ids before mutating the current project.

Planr packages are local-first JSON. For encrypted sharing, review the JSON locally and encrypt the file with your team's standard tool, for example:

```bash
age -o planr-backup.json.age -r <recipient> planr-backup.json
gpg -c planr-backup.json
```

Planr does not require a hosted share service for V1.1.
