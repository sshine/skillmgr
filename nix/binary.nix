# Prebuilt release binaries, for consumers who want skillmgr without a Rust toolchain
# entering their build closure.
#
# The pinned version deliberately trails Cargo.toml: it names the last tag whose
# artifacts are published, not whatever the working tree happens to claim. Bump it
# once a tag's Release workflow has finished; `just release-hashes <tag>` prints the
# values.
#
# The hashes are of the tarballs the release publishes, so they can be checked against
# the .sha256 assets rather than taken on faith. Systems absent from `hashes` do not get
# the package, which is what keeps it off Darwin, since the release builds only Linux.
{ ... }:
{
  perSystem =
    {
      system,
      pkgs,
      lib,
      ...
    }:
    let
      release = {
        version = "0.1.0";
        hashes = {
          # Filled by `just release-hashes v0.1.0` after the first release.
        };
      };

      triple = {
        "x86_64-linux" = "x86_64-unknown-linux-musl";
        "aarch64-linux" = "aarch64-unknown-linux-musl";
      };

      tag = "v${release.version}";

      tarball = pkgs.fetchurl {
        url = "https://github.com/sshine/skillmgr/releases/download/${tag}/skillmgr-${tag}-${triple.${system}}.tar.gz";
        hash = release.hashes.${system};
      };
    in
    lib.optionalAttrs (release.hashes ? ${system}) {
      packages.skillmgr-bin =
        pkgs.runCommand "skillmgr-bin-${release.version}"
          {
            nativeBuildInputs = [ pkgs.makeWrapper ];

            meta = {
              description = "Manage vendored and authored agent skills (prebuilt)";
              mainProgram = "skillmgr";
              license = with lib.licenses; [
                mit
                asl20
              ];
              platforms = lib.attrNames release.hashes;
              sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
            };
          }
          ''
            tar -xzf ${tarball}
            install -Dm755 skillmgr $out/bin/skillmgr

            wrapProgram $out/bin/skillmgr \
              --prefix PATH : ${lib.makeBinPath [ pkgs.git ]}
          '';
    };
}
