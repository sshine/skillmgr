# The build definition, shared by this flake's packages and by the overlay.
#
# Taking `pkgs` as an argument (rather than closing over this flake's own) is what
# lets the overlay build against the consumer's nixpkgs, so downstream can override
# and cross-compile it.
{
  lib,
  rustPlatform,
  makeWrapper,
  git,
  ...
}:
let
  workspace = lib.importTOML ../Cargo.toml;
  crate = lib.importTOML ../crates/skillmgr-cli/Cargo.toml;
in
rustPlatform.buildRustPackage {
  pname = "skillmgr";
  version = workspace.workspace.package.version;

  # Naming the inputs explicitly keeps target/ and .direnv/ out of the store, and
  # means an unrelated edit does not invalidate the build.
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../crates
      ../README.md
    ];
  };
  cargoLock = {
    lockFile = ../Cargo.lock;
    outputHashes = {
      # promptui-core comes from a git URL, so Nix needs the checkout's hash.
      "promptui-core-0.1.0" = "sha256-ZqTJ33QmspaOw7nPMVjfhrt13/I8J4SkQLyVFHy9pMU=";
    };
  };

  cargoBuildFlags = [
    "--package"
    crate.package.name
  ];

  nativeBuildInputs = [
    makeWrapper
    git
  ];

  # skillmgr drives git for every origin fetch; a bare `git` on the user's PATH is
  # not something a Nix-installed binary may assume exists.
  postInstall = ''
    wrapProgram $out/bin/skillmgr \
      --prefix PATH : ${lib.makeBinPath [ git ]}
  '';

  meta = {
    inherit (crate.package) description;
    mainProgram = "skillmgr";
    license = with lib.licenses; [
      mit
      asl20
    ];
  };
}
