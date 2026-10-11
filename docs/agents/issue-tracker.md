# Issue tracker: GitHub

Issues and specs live in this repository's GitHub Issues.
Use the authenticated `gh` CLI from the repository root.
Infer the repository from its Git remote.

## Operations

- Create: `gh issue create --title "..." --body "..."`
- Read: `gh issue view <number> --json number,title,body,labels,comments`
- List: `gh issue list --state open --json number,title,body,labels`
- Update: `gh issue edit <number> --title "..." --body "..."`
- Comment: `gh issue comment <number> --body "..."`
- Label: `gh issue edit <number> --add-label "..." --remove-label "..."`
- Close: `gh issue close <number> --comment "..."`

Use `--body-file` for multiline bodies when appropriate.

When a skill says "publish to the issue tracker", create a GitHub issue.
When it says "fetch the relevant ticket", read the issue and its comments.

## Pull requests as a triage surface

PRs as a request surface: no.

## Wayfinding

A map is an issue labelled `wayfinder:map`. Child tickets use
`wayfinder:<type>`, where type is research, prototype, grilling, or task.

Use native GitHub sub-issues and issue dependencies when supported by
the installed CLI and repository. Otherwise, list children in the map's
task list, add `Part of #<map>` to each child, and record dependencies
as `Blocked by: #<number>` in the child body.

The frontier is the first open child in map order with no open blockers
and no assignee. Claim with `gh issue edit <number> --add-assignee @me`.
Resolve by commenting with the result, closing the child, and adding
a result link to the map's Decisions-so-far.
