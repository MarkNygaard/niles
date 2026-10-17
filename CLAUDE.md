@AGENTS.md

# Claude Code notes

Everything above comes from `AGENTS.md`, which every agent reads. What follows is only for Claude Code.

## Attribution

Commits end with:

```
Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

PR bodies end with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.

## Local files

- `ROADMAP.md` is Mark's local working note: gitignored, and auto-loaded when present. Update it after each merge.
- `CLAUDE.local.md` holds local-only notes, gitignored, loaded beside this file.

## Shell habits

These avoid permission prompts on Windows:

- In Bash, no heredocs, no shell loops and no `$(...)`.
- Write scripts, commit messages and PR bodies to a file with the Write tool, then pass the path: `git commit -F <file>`, `gh pr create --body-file <file>`.
- Watch CI with `gh pr checks <number> --watch`.
- PRs opened from Mark's account can't get his approval (GitHub doesn't let an author approve their own PR), so they merge through his admin bypass once CI is green: `gh pr merge <number> --squash --delete-branch --admin`. Only when he has asked for the merge.

## The installed app

The PWA installed on an iPhone keeps its old JavaScript after a deploy. Before debugging a UI error reported from the phone, ask for the app to be closed or hard-refreshed.
