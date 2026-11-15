---
name: manage-skills
description: >-
  Keep agent skills in skillmgr's managed repository and use skillmgr to create,
  find, install, update, and remove them. Use this skill whenever the user asks
  to create a skill, edit an existing skill, find or discover a skill, install
  or add a skill from a repository, decide where a skill should live, or manage
  the skills on this machine, even when another skill such as skill-creator or
  find-skills is handling the main workflow. Prefers the skillmgr managed
  repository at ~/.agents/managed-skills/ when it exists.
metadata:
  version: "1.0"
---

# Manage skills

`skillmgr` is the source of truth for skills on this machine. It keeps a
managed git repository at `~/.agents/managed-skills/` and symlinks active
skills from `~/.agents/skills/`.

```
~/.agents/
├── managed-skills/
│   ├── skills.toml
│   └── skills/
│       ├── vendored/<name>/    # pinned upstream copies
│       └── authored/<name>/    # skills written here
└── skills/<name> -> ../managed-skills/skills/<source>/<name>
```

Check that the managed repository exists before deciding where anything goes:

```console
$ test -f ~/.agents/managed-skills/skills.toml && echo managed
$ skillmgr list
```

This skill adds skillmgr's storage, installation, and provenance rules to
whatever skill is driving the task. When another skill such as `skill-creator`
or `find-skills` applies, follow its process for the actual work and use the
rules here for where the skill lives and how it is installed. Do not duplicate
or replace that skill's workflow.

## Working with other skills

- **skill-creator**: keep its interview, drafting, testing, and evaluation
  loop. When it is time to write the skill, create or edit the copy in the
  managed repository. Do not let it write the final skill into a harness
  directory or leave it unmanaged.
- **find-skills or another discovery skill**: keep its search, quality checks,
  and user approval steps. When the user agrees to install a skill, install it
  with `skillmgr add` rather than a competing package manager.
- **A harness's skill installer**: if it already created a real directory in a
  harness directory, adopt or vendor that skill into skillmgr and remove the
  duplicate, so the machine has one owner for the skill.

## Finding a skill

Search the managed repository first, because a suitable skill may already be
installed and linked:

```console
$ skillmgr list
$ skillmgr list <query>
```

If nothing suitable exists, use the discovery skill the user has. For example,
`find-skills` searches the open ecosystem with `npx skills find`. Verify
quality and let the user choose before installing.

When the user chooses a skill, install it with skillmgr instead of
`npx skills add`. Skillmgr records the upstream repository, the path, and a
pinned revision, and makes updates reviewable:

```console
$ skillmgr add <owner/repo> <path/to/skill>   # one skill
$ skillmgr add <owner/repo>                   # every SKILL.md in the repo
$ skillmgr add anthropics/skills skills/skill-creator
```

If the repository contains many skills, pass the directory for the one the
user wants. The path is relative to the repository root. Adding the whole
repository vendors every skill it discovers, which is usually more than the
user asked for.

## Adding a local skill

Use the skillmgr operation that matches where the skill currently lives:

```console
$ skillmgr add <owner/repo> <path>   # already upstream; vendor it
$ skillmgr adopt <name>              # a real directory in ~/.agents/skills
$ skillmgr register <name>           # a directory already below skills/
```

`skillmgr adopt` moves an existing real skill directory into
`skills/authored/`, registers it, and replaces the directory with a link.
`skillmgr register` only adds a registry entry for a directory that is already
inside the managed repository.

Do not create real skill directories directly in `~/.agents/skills/` or in a
harness directory such as `~/.config/opencode/skills/`, `~/.claude/skills/`,
`.opencode/skills/`, or `.claude/skills/`. Those directories hold symlinks
managed by skillmgr. A real directory there collides with the link and
silently forks the skill from its source.

## Where a new skill belongs

If the managed repository exists, create the skill there. Use the manager so
the registry and link stay in sync:

```console
$ skillmgr new <name> --description "<what it does and when to use it>"
$ skillmgr edit <name>
```

This writes `~/.agents/managed-skills/skills/authored/<name>/SKILL.md`,
registers the skill in `skills.toml`, and links it at
`~/.agents/skills/<name>`. If a skill creator needs a concrete directory, give
it:

```
~/.agents/managed-skills/skills/authored/<name>/SKILL.md
```

Prefer `skillmgr new` over creating that directory by hand. If the directory
already exists and contains a `SKILL.md`, use `skillmgr register <name>`
instead.

### When there is no managed repository

If `~/.agents/managed-skills/skills.toml` does not exist, ask the user where
the skill should live. Offer these options in order:

1. **Managed skill (recommended)** — run `skillmgr init` (or the `skillmgr`
   TUI and initialize it), then `skillmgr new <name>`. The skill lives in the
   managed repository and is available everywhere.
2. **Vendored skill** — for a skill that already exists upstream, use
   `skillmgr add owner/repo path/to/skill` instead of copying it by hand.
3. **Project-local skill** — only when the user explicitly wants the skill in
   one repository. Use that harness's project directory, such as
   `.opencode/skills/<name>/` or `.claude/skills/<name>/`.
4. **Unmanaged global skill** — the harness's global directory. Use this only
   if the user declines skillmgr; explain that updates and provenance are then
   manual.

## Skill conventions

- Use a lowercase hyphenated name, at most 64 characters, and make the
  frontmatter `name` match the directory name.
- Put a `description` in the frontmatter that says both what the skill does and
  when an agent should use it.
- Write instructions in `SKILL.md`. Put longer references or scripts in
  sibling files inside the skill directory.
- Do not edit a vendored skill directly. Change the upstream repository and run
  `skillmgr update`, or pin a different revision with `skillmgr update --to`.

## Maintaining existing skills

```console
$ skillmgr list                 # local inventory, linked state
$ skillmgr status               # also check how far upstream is behind
$ skillmgr update [name...]     # move pins to the branch head and re-vendor
$ skillmgr sync [name...]       # re-vendor at the existing pin
$ skillmgr install [name...]    # restore symlinks in ~/.agents/skills
$ skillmgr remove <name>        # unregister and remove the link
```

Run `skillmgr` with no arguments to use the interactive dashboard. It exposes
the same operations with every shortcut in the keybar.
