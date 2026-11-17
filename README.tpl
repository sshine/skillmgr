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

### NixOS / Nix system-manager

The flake exports an overlay, so a NixOS configuration can take skillmgr as an
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

The flake works with [system-manager] too. The example below shows both how to
install skillmgr with system-manager and how to use the prebuilt `skillmgr-bin`
package, so no Rust toolchain enters the build closure:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    system-manager = {
      url = "github:numtide/system-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    skillmgr = {
      url = "github:sshine/skillmgr";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      system-manager,
      skillmgr,
      ...
    }:
    {
      systemConfigs.default = system-manager.lib.makeSystemConfig {
        modules = [
          (
            { pkgs, ... }:
            {
              nixpkgs.hostPlatform = "x86_64-linux";
              environment.systemPackages = [
                skillmgr.packages.${pkgs.stdenv.hostPlatform.system}.skillmgr-bin
              ];
            }
          )
        ];
      };
    };
}
```

[system-manager]: https://github.com/numtide/system-manager

## Quick start

```console
$ skillmgr init
$ skillmgr add sshine/skills agentic-playwright-nix
$ skillmgr add git@github.com:sshine/skills.git --link   # a whole collection
$ skillmgr status
NAME                     ORIGIN    LINK    REV      BEHIND  PATH
agentic-playwright-nix   vendored  linked  84a1a07  12/3    sshine/skills:agentic-playwright-nix

$ skillmgr new my-skill --description "What this skill does and when to use it"
$ skillmgr update
```

Adding vendors skills without linking them. Pass `--link` to link the new skills
immediately, or run `skillmgr install` later. This keeps a whole upstream
collection out of the active directory until you choose what to use.

`BEHIND` is `repo/path`: commits on the tracked branch since the pin, and how many of
those touch the skill's directory. `skillmgr update` moves the pin to the branch head;
`skillmgr sync` re-vendors at the existing pin. `sync --check` and `list --outdated`
exit non-zero, so they work in CI.

`skillmgr upstreams` groups the registry by source repository:

```console
$ skillmgr upstreams
REPO                                    SKILLS  LINKED
git@github.com:anthropics/skills.git    16      2
git@github.com:sshine/skills.git        7       1
```

`skillmgr upstreams unlink <repo>` removes the links for every skill from that
repository in one step, while keeping the vendored files and registry entries.
A shorthand such as `sshine/skills` matches the recorded SSH URL.

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
x uninstall   o adopt   d remove   e edit   r refresh   / filter   v upstreams   ? help   q quit
```

Destructive actions ask for confirmation. Adding a repository returns immediately:
the fetch runs in the background with a network meter, and a failure stays in the list
as a `faulty` entry rather than closing the dialog with an error. The add form's link
toggle starts off, so a new repository is vendored without flooding the active
directory; turn it on with Space to link the new skills.

Press `v` for the upstream overview. It lists every source repository with its skill
and link counts. Pressing Enter unlinks every skill from the selected upstream, while
leaving its vendored files and registry entries in place. Upstream checks also run in
the background, and the detail pane reads the selected skill's `SKILL.md`, origin,
pin, link state and behind counts.

On first run, initializing the managed directory also offers to vendor skillmgr's
own `manage-skills` skill from `sshine/skillmgr`. That skill teaches agents to
create new skills in the managed repository, to look for existing skills there
first, and to install discovered skills with `skillmgr add` instead of copying
them into a harness directory.

Optional setup questions are asked once. Answers live in
`$XDG_STATE_HOME/skillmgr/state.toml` by default
(`~/.local/state/skillmgr/state.toml`), keyed by managed root. A question is
asked again only when it is new or its version changes. Use `--state-dir` to
put this file elsewhere.

## CLI

{{readme}}

## License

MIT OR Apache-2.0.
