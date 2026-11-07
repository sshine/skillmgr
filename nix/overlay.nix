{ ... }:
{
  flake.overlays.default = final: _prev: {
    skillmgr = final.callPackage ./_package.nix { };
  };
}
