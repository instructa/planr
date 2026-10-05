# Planr bb plugin

`server.ts` owns project mappings, RPC forwarding, realtime invalidation and the read-only
`bb planr next|check|board` CLI. `host.ts` owns file access, the engine, Git and native watches
on the enrolled machine. `contract.ts` is the shared UI/server/host contract.

## Isolated development loop

Develop against an isolated local bb instance, never a bb server used for real work.
Put a `bb` CLI that targets that instance first on PATH (`npm run build` calls `bb plugin build`):

```sh
cd bb-plugin
npm run build
bb plugin types --check
npm run typecheck
npm test
(cd .. && node --test)
bb plugin dev
```

`npm run build` stages the single source `../skills/planr`
into ignored `dist/skills/planr`, then runs `bb plugin build`. The manifest ships that complete
skill, including scripts and assets; never commit a second copy. `prepare` also stages it at install.

With a remote, human actions use the detached `<git-toplevel>-planr-ui` worktree and compare
against both main and origin before committing and pushing. A dirty target in main is refused.
After a push, main fetches and fast-forwards only if clean; otherwise its files stay unchanged.
Nested planning roots retain their Git-relative paths.
With no remote, actions commit only the selected file in the main checkout, preserving other work;
a dirty selected file or stale SHA causes a conflict. All actions serialize per repository and use
the `Approved-via: planr-ui` trailer. Rejected pushes get one fetch, SHA check, rebase and retry.
The SDK harness tests cover writes and the watch → host signal → realtime publication path.

## Approval identity

bb builds that provide `bb.server.experimental_requestPrincipal()` supply request identity
(upstream bb does not). With a trusted identity listener configured,
only principals of kind `person` can approve. Configure `approvers` in bb plugin settings:
one `issuer:subject` per line; blank means any person. Identifiers match exactly, including case;
email and display name do not grant access. The first colon separates issuer from subject.
Invalid lines are logged, and a non-empty list with no valid entries refuses approval.
Enforced approvals add `Approved-by: issuer:subject` after `Approved-via: planr-ui`.

Without that API the plugin uses convention mode; a present API without an identity listener refuses actions.
`session_get` reports the request's identity and permission. Identity is captured before any await.
As recorded in the contract, agents on a device signed in as the user can carry that user's identity.
