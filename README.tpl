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

[releases]: https://github.com/sshine/skillmgr/releases

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

{{readme}}

## License

MIT OR Apache-2.0.
