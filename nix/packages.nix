{ ... }:
{
  perSystem =
    { pkgs, lib, ... }:
    rec {
      checks.skillmgr = packages.default;

      packages.default = pkgs.callPackage ./_package.nix { };

      # The README advertises `nix run <url>#skillmgr`.
      packages.skillmgr = packages.default;

      apps.default = {
        type = "app";
        program = lib.getExe packages.default;
      };
    };
}
