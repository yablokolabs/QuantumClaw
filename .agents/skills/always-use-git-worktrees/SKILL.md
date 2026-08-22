---
name: always-use-git-worktrees
description: Standing rule for this repository — ALWAYS isolate feature work in a git worktree under .worktrees/, and never run destructive branch operations in the main checkout. Load before any multi-step git work.
---

<required>
*CRITICAL* Standing user preference for this repository:

1. **Always use git worktrees for feature work.** Before starting any new
   branch-based work, create a worktree under `.worktrees/<branch>` (the
   directory is already gitignored) and do all work from inside it. See the
   `using-git-worktrees` skill for the full checklist (symlinking git-ignored
   content, setup, tests, staying inside the worktree).

2. **Never run destructive branch operations in the main checkout.**
   Specifically: no `git reset --hard`, no branch-switching that can clobber
   uncommitted or unpushed work, no `git checkout <branch>` followed by
   destructive operations, while the main checkout holds work. The main
   checkout stays on `main`, synced to `origin/main`.

3. If the main checkout is already on a feature branch with work in flight,
   move that work into a worktree before any further branch surgery.
</required>

# Why this rule exists

The incident that created it: a `git reset --hard origin/main` in the main
checkout silently dropped feature files (a benchmark script and the dwave-sqa
code) from the working tree. Nothing was lost — the commits were safe on a
pushed branch — but the working tree no longer matched the branch, causing
confusion ("file not found") until the branch was checked out again.

Worktrees make this class of mistake impossible: each branch has its own
directory, `main` stays put, and switching branches never touches the
feature branch's files.

# Mechanics

- Worktrees live in `.worktrees/` (already in `.gitignore`).
- Create: `git worktree add ".worktrees/$BRANCH" -b "$BRANCH"`
- Report the worktree path when starting work, and stay inside it for the
  session.
- Symlink shared git-ignored content (env files, data, caches) from the main
  checkout so nothing is lost on worktree cleanup.
