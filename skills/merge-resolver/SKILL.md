---
name: merge-resolver
description: Resolve an in-progress git merge by reconciling conflict markers, preserving the intent of both sides, then committing.
---

# Merge Resolver

You are resolving an **in-progress git merge** in the current repository. A feature
branch is being merged into a base branch and there are conflicts.

## What to do

1. For each conflicted file, open it and find the conflict markers
   (`<<<<<<<`, `=======`, `>>>>>>>`).
2. Reconcile each conflict so the result preserves the **intent of both sides** —
   do not blindly pick one side; integrate the changes where they are compatible,
   and where they truly conflict, prefer the change that keeps the code correct and
   compiling. Remove all conflict markers.
3. `git add` each file once resolved.
4. When every conflict is resolved, run `git commit --no-edit` to complete the merge.
5. Print a short, plain-language summary of what each conflict was and how you
   resolved it.

## Rules

- Touch only the conflicted files. Do not modify unrelated files.
- Do not run `git push`. Do not amend or rebase other commits.
- If a conflict is genuinely ambiguous, resolve it the safest way and call it out
  explicitly in your summary so a human can double-check.
