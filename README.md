# skillmgr

Manage agent skills from a declarative `skills.toml` registry. Vendored skills are
copied from upstream git repositories at a pinned commit; authored skills live in the
same managed directory; both are symlinked into `~/.agents/skills/`.

The point is to stop copying skills by hand. `skillmgr` records where each vendored
skill came from, keeps the copy reviewable in git, and reports how far behind upstream
both the repository and the skill's own directory are.

```
~/.agents/
├── managed-skills/
│   ├── skills.toml
│   └── skills/
│       ├── vendored/<name>/    # pinned upstream copies
│       └── authored/<name>/    # written here
└── skills/<name> -> ../managed-skills/skills/<source>/<name>
```

Run `skillmgr` with no arguments for a promptui dashboard. The same operations are
available as subcommands.

## Installation

```
nix run github:sshine/skillmgr
nix profile install github:sshine/skillmgr
cargo install --path crates/skillmgr-cli
```

You can download prebuilt [static binaries for x86_64 and aarch64 Linux][releases].

`git` must be on `PATH`; skillmgr drives it to fetch origins and walk history. The Nix
package wraps the binary so this is already taken care of.

### NixOS

The flake exports an overlay, so a system configuration can take skillmgr as an
ordinary attribute of `pkgs`:

```nix
{
  inputs = {
    nixpkgs.url = "https://nixos.org/channels/nixpkgs-unstable/nixexprs.tar.xz";

    skillmgr = {
      url = "github:sshine/skillmgr";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { nixpkgs, skillmgr, ... }:
    {
      nixosConfigurations.example = nixpkgs.lib.nixosSystem {
        modules = [
          (
            { pkgs, ... }:
            {
              nixpkgs.overlays = [ skillmgr.overlays.default ];
              environment.systemPackages = [ pkgs.skillmgr ];
            }
          )
        ];
      };
    };
}
```

For a prebuilt binary instead of a source build, use
`(skillmgr.packages.${system}).skillmgr-bin`. The release is pinned in
`nix/binary.nix`.

## Quick start

```console
$ skillmgr init
$ skillmgr add sshine/skills agentic-playwright-nix
$ skillmgr add git@github.com:sshine/skills.git   # a whole collection
$ skillmgr status
NAME                     ORIGIN    LINK    REV      BEHIND  PATH
agentic-playwright-nix   vendored  linked  84a1a07  12/3    sshine/skills:agentic-playwright-nix

$ skillmgr new my-skill --description "What this skill does and when to use it"
$ skillmgr update
```

`BEHIND` is `repo/path`: commits on the tracked branch since the pin, and how many of
those touch the skill's directory. `skillmgr update` moves the pin to the branch head;
`skillmgr sync` re-vendors at the existing pin. `sync --check` and `list --outdated`
exit non-zero, so they work in CI.

## Registry

```toml
schema = 1

[skills.agentic-playwright-nix]
source = "vendored"
repo = "sshine/skills"
path = "agentic-playwright-nix"
branch = "main"
rev = "84a1a07..."

[skills.blog-post]
source = "authored"
```

`source` names the directory below `skills/`. An entry with `repo`, `path`, `branch`
and `rev` is vendored; an entry with only `source` is authored. Vendored copies are
committed to the managed git repository; the origin cache is disposable.

A repository that contains `SKILL.md` files anywhere is a skills collection. Adding
one discovers and vendors every skill in it. A repository that cannot be reached, or
that contains no `SKILL.md` files, is kept as a repository declaration instead of
being rejected:

```toml
[skills.sshine-skills]
source = "vendored"
kind = "repository"
repo = "git@github.com:sshine/skills.git"
path = "."
status = "no SKILL.md files found"
```

Repository declarations show up as `repo` or `faulty`, never block operations on
healthy skills, and can be retried with `sync` or by adding the repository again.

## Origin cache

Upstream repositories are bare mirrors under
`$XDG_CACHE_HOME/skillmgr/origins/`, named by transport, host, transport user,
namespace and repository:

```
git+https/github.com/anonymous/sshine/skills.git
git+ssh/github.com/git/sshine/skills.git
git+file/localhost/anonymous/home/sshine/repos/foo.git
```

Every component is percent-encoded, so the path is reversible and unambiguous. The
cache is safe to delete; it is never the source of truth.

## TUI

The dashboard has one table, a detail pane, and every shortcut visible in the keybar:

```
a add   n new   u update (U all)   s sync (S all)   i install (I all)
x uninstall   o adopt   d remove   e edit   r refresh   / filter   ? help   q quit
```

Destructive actions ask for confirmation. Adding a repository returns immediately:
the fetch runs in the background with a network meter, and a failure stays in the list
as a `faulty` entry rather than closing the dialog with an error. Upstream checks also
run in the background, and the detail pane reads the selected skill's `SKILL.md`,
origin, pin, link state and behind counts.

## CLI

## skillmgr

Manage agent skills as a mixture of vendored upstream copies and skills authored
in the managed directory.

`skillmgr` keeps a managed home at `~/.agents/managed-skills/` with a
`skills.toml` registry and two conventional source directories:

```
skills/vendored/<name>/   copies pinned to an upstream git commit
skills/authored/<name>/   skills written here
```

Active skills are symlinked into `~/.agents/skills/`. Vendored skills can be
synced at their pinned revision or updated to the upstream branch head, with a
heuristic that reports both how many commits the repository is ahead and how
many of those commits touch the skill's directory.

### Usage

```
$ skillmgr --help
Manage agent skills as a mixture of vendored upstream copies and skills authored
in the managed directory.

Usage: skillmgr [OPTIONS] [COMMAND]

Commands:
  init        Create the managed directory and an empty skills.toml
  add         Vendor a skill from a git repository
  remove      Unregister skills, remove their links, and optionally delete files
  sync        Re-vendor skills at their pinned revisions
  update      Move pins to the branch head and re-vendor
  list        List registered and unregistered skills
  status      List skills with upstream behind counts
  install     Symlink registered skills into the active directory
  uninstall   Remove managed symlinks from the active directory
  new         Scaffold an authored skill
  register    Register an existing directory below skills/
  adopt       Move an existing active skill directory into skills/authored/
  edit        Open a skill's SKILL.md in $EDITOR
  completion  Print a shell completion script
  tui         Open the interactive dashboard
```

Run without a command in a terminal to open the dashboard. The TUI uses the same
operations as the subcommands and keeps every shortcut visible in its keybar.

### Registry

```toml
schema = 1

[skills.agentic-playwright-nix]
source = "vendored"
repo = "sshine/skills"
path = "agentic-playwright-nix"
branch = "main"
rev = "84a1a07..."

[skills.blog-post]
source = "authored"
```

`source` names the directory below `skills/`; `vendored` and `authored` are
conventions, not hardcoded behaviour. An entry with `repo`, `path`, `branch` and
`rev` is vendored. Anything else is authored.

A repository that contains `SKILL.md` files anywhere is a skills collection:
adding it discovers and vendors every skill. An entry with `repo` and `path` but
no complete pin is a repository declaration. It may be unreachable or contain no
skills; either way it stays in the registry as a faulty entry that a later
retry can resolve, and it never blocks operations on healthy skills.

### Origin cache

Upstream repositories are bare mirrors under
`$XDG_CACHE_HOME/skillmgr/origins/`, named by transport, host, transport user,
namespace and repository:

```
git+https/github.com/anonymous/sshine/skills.git
git+ssh/github.com/git/sshine/skills.git
```

The cache is disposable. Vendored copies live in the managed directory and are
the source of truth; deleting the cache never removes an installed skill.

### Installation

```
nix run github:sshine/skillmgr
nix profile install github:sshine/skillmgr
cargo install --path crates/skillmgr-cli
```

For NixOS, add the flake as an input and use `pkgs.skillmgr` from the overlay, or
the prebuilt `skillmgr-bin` package from the release.

`git` must be on `PATH`; the Nix package wraps the binary with it.

## License

MIT OR Apache-2.0.

[releases]: https://github.com/sshine/skillmgr/releases
