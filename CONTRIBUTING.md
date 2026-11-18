# Contributing

Two workflows run on every pull request and on every push to `main`:

| Workflow | File | What it proves |
|---|---|---|
| `docs` | `.github/workflows/docs.yml` | The `docs/` corpus satisfies the governance contract (`scripts/check_docs.py`). |
| `workspace` | `.github/workflows/workspace.yml` | The pure-Rust workspace formats, lints and tests on Linux, macOS and Windows. |

`main` is protected and every check those two workflows produce is required. The
settings live in GitHub, not in this repository, so a pull request cannot change
them: read this section before you touch a job name, a workflow file or the
matrix.

## 1. Current settings

Snapshot, 2026-11-18. GitHub is the authority; re-read it with the commands in
§5 before relying on this table.

| Setting | Value |
|---|---|
| Required checks | `check`, `gates (ubuntu-latest)`, `gates (macos-latest)`, `gates (windows-latest)` |
| Require branches to be up to date before merging (`strict`) | enabled |
| Include administrators (`enforce_admins`) | enabled |
| Force pushes | disabled |
| Deleting `main` | disabled |
| Required signatures, linear history, conversation resolution | not enabled |

`enforce_admins` means a maintainer cannot merge past a red or missing required
check. It does not prevent a maintainer from editing the protection settings
themselves — §6 and §7 are the escape hatches, and they are the only ones.

**What these settings do not do.** "Require a pull request before merging" is
*not* enabled (`required_pull_request_reviews` is null) and `restrictions` is
not set, so a maintainer who can push to the repository can push straight to
`main`; required status checks gate pull-request merges only, and a direct push
never consults them. Treat that as a known hole, not as a guarantee: if the
intent becomes "every change to `main` passes both workflows", enable the
pull-request requirement in the same PUT body (§6) and then expect direct
pushes to be rejected.

## 2. Which job produces which check

A check's context name is the job's `name:` when it has one, otherwise its job
id. It is not prefixed with the workflow name, and it is not the file name.

| Required context | Job | Definition |
|---|---|---|
| `check` | id `check`, no `name:` | `.github/workflows/docs.yml` |
| `gates (<os>)` | id `gates`, `name: gates (${{ matrix.os }})`, matrix `os: [ubuntu-latest, macos-latest, windows-latest]` | `.github/workflows/workspace.yml` |

A matrix job produces one check per matrix value expanded into `name:`. Adding a
value produces a new context; removing a value stops producing one.

## 3. The invariant

> Every required context must be produced by a job that runs on every pull
> request.

Three consequences, each with a failure mode the repository has to avoid:

1. **No `paths:` filters on a workflow that produces a required check.** A
   filtered-out workflow is not "skipped and green": GitHub never creates the
   check at all, the pull request shows `Expected — Waiting for status to be
   reported`, and it waits forever. Both workflows deliberately run on every
   event for this reason; the comments at the top of each file say so.
2. **Check names must be unique across workflows.** Two jobs named `check` in
   different workflows produce two check runs with the same context, and the
   required context then depends on which run finished last. Keep `check`
   (the docs job) in mind when naming anything new.
3. **Do not satisfy a required check with a job that never runs.** The two
   ways a job can not run behave differently, and neither one is the gate the
   review is for:
   - `if:` that evaluates false still creates the job, and GitHub reports it as
     skipped and counts it as satisfied — the pull request can merge although
     nothing ran. This is the dangerous one, because nothing looks stuck.
   - a matrix `exclude:` creates no job at all, so the context is never
     reported and the pull request waits forever.

   Treat either outcome on a required context as a configuration bug.

## 4. Changing a check name or a matrix value

The repository and the protection settings cannot change atomically: protection
is changed over the API and the workflow is changed by merging a pull request.
What matters is the direction of the two changes, so that no intermediate state
leaves `main` blocked.

| Change | Order |
|---|---|
| **Add** a job or a matrix value | Merge the workflow change first — the new check appears but is not yet required — then add it to the required contexts (§6). |
| **Remove** a job or a matrix value | Remove the context from protection first (§6), then merge the change that stops producing it. Reversed, the pull request can never satisfy the context it is deleting. |
| **Rename** a job | 1. Add the new job next to the old one and merge. 2. Set protection to require both names. 3. Remove the old name from the required list, keeping the new name and every `gates (...)` context. 4. Merge the pull request that deletes the old job. Step 3 must precede step 4 for the same reason as the Remove row: the deletion pull request is tested at its merge commit, where the old job no longer exists, so the old context would never be reported. |
| **Split** one job into two, or revert a change that both added and removed a context | Apply both rows above to the two directions separately: every name the change stops producing is removed from protection *before* the merge, every name it starts producing is added *after* it has run on `main`. A single pull request can therefore be blocked by one direction unless the protection change lands first. |

Do not merge a change that stops producing a required context until its context
is gone from protection. If it happens anyway, §7 unblocks `main`.

`strict` also means a pull request whose checks passed must be brought up to
date and re-run after `main` moves. That is expected; the newest run of a
context is the one that counts.

Pull requests opened from a fork by a first-time contributor do not run the
workflows until a maintainer approves them, so their required contexts stay
pending while nothing is wrong with the configuration. That is the one
"Waiting for status to be reported" state §7 is *not* the answer to: approve
the run instead.

## 5. Verifying that settings and workflows agree

Run both and diff the two lists. They must be equal.

```bash
# 1. contexts required on main
gh api repos/YougLin-dev/zenui/branches/main/protection \
  --jq '.required_status_checks.contexts[]'

# 2. check names the workflows actually produce on main
for wf in docs.yml workspace.yml; do
  id=$(gh run list --workflow "$wf" --branch main --limit 1 \
        --json databaseId --jq '.[0].databaseId')
  gh api "repos/YougLin-dev/zenui/actions/runs/$id/jobs" --jq '.jobs[].name'
done
```

A name in list 1 but not list 2 blocks merges. A name in list 2 but not list 1
is a check that runs but gates nothing — fix it before it is relied on. The two
lists are *supposed* to differ in one window: while the Remove row of §4 is
half-done, the context has already been dropped from protection and the job is
still running. Check which side of that window you are on before "fixing" it.

## 6. Applying protection

`PUT` replaces the whole protection object. Send the full body every time, and
treat `contexts` as **derived, never copied**: build it from §5 command 2 — the
names the workflows actually produce on `main` — plus the name you are
deliberately adding. A literal list pasted from an older revision is how a
context that no job produces gets re-installed later.

```bash
gh api -X PUT repos/YougLin-dev/zenui/branches/main/protection --input - <<'JSON'
{
  "required_status_checks": {
    "strict": true,
    "contexts": [
      "check",
      "gates (ubuntu-latest)",
      "gates (macos-latest)",
      "gates (windows-latest)"
    ]
  },
  "enforce_admins": true,
  "required_pull_request_reviews": null,
  "restrictions": null,
  "required_linear_history": false,
  "allow_force_pushes": false,
  "allow_deletions": false,
  "required_conversation_resolution": false
}
JSON
```

The array above is the 2026-11-18 state, shown so the shape is unambiguous. It
is not the value to send: replace it with the derived list. Keeping
`required_pull_request_reviews: null` is what leaves direct pushes possible
(§1); replace it with the pull-request requirement if that changes.

Requires admin on the repository. Re-read §5 afterwards; the apply and the read
back are one operation, and only the read back counts as confirmation.

## 7. Recovering a blocked `main`

Symptom: a pull request is mergeable-ready but sits on `Expected — Waiting for
status to be reported`, and no run of that name exists. Rule out a fork pull
request waiting for approval first (§4), then:

```bash
# a. drop only the required checks; other protection stays in place
gh api -X DELETE repos/YougLin-dev/zenui/branches/main/protection/required_status_checks

# b. remove protection from main entirely; the branch and its history are untouched
gh api -X DELETE repos/YougLin-dev/zenui/branches/main/protection
```

Use (a) when the workflow is right and only the context list is wrong, (b) when
protection itself is wrong. Both need admin. Neither rewrites history.

Restoring, in order — do not skip the read back at the end:

1. Merge the workflow fix first. Until it is on `main` and has run there, there
   is nothing to derive the context list from.
2. Run §5 command 2 against `main` and collect the names it produces.
3. `PUT` the full body from §6 with `contexts` set to exactly that list.
4. Run §5 in full and require list 1 == list 2. That equality, not the `PUT`
   response, is the confirmation that `main` is unblocked.

## 8. Why this is documented here and not enforced by CI

Branch protection is repository state, not a tracked file, so nothing in the
repository can assert it: a pull request that changes a job name cannot fail a
check because protection was not updated in the same commit. The repository can
only carry the procedure (§4), the read-back commands (§5) and the recovery
steps (§7). The gate is the review: a change to `.github/workflows/` that alters
a check name, a matrix value or the triggers is a change to `main`'s merge
policy, and is reviewed as one.
