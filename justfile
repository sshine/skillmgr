# Default recipe: list available commands
default:
    @just --list --unsorted

fmt:
    treefmt

fmt-check:
    treefmt --fail-on-change --no-cache

lint:
    cargo clippy --all-targets --all-features -- -D warnings

# Find dead Nix code
lint-nix:
    deadnix --fail nix/ flake.nix

test:
    cargo test --all-features

test-verbose:
    cargo test --all-features -- --nocapture

build:
    cargo build --release --all-features

doc *args='':
    cargo doc --no-deps --all-features {{args}}

readme_args := "--project-root crates/skillmgr-cli --input crates/skillmgr-cli/src/main.rs --template README.tpl"

# Regenerate README.md from README.tpl and the CLI docs
readme:
    cargo readme {{readme_args}} | mdformat - > README.md

# Check README.md is in sync with README.tpl and the CLI docs
readme-check:
    cargo readme {{readme_args}} | mdformat - | diff - README.md

lock-check:
    cargo metadata --locked --format-version 1 > /dev/null

# The CLI depends on promptui through a git URL, so only the dependency-free core
# crate is a candidate for crates.io packaging.
package-check:
    cargo package --locked -p skillmgr-core --no-verify

release_targets := "x86_64-unknown-linux-musl aarch64-unknown-linux-musl"

# Assert the release tag names the version cargo would publish
check-version version:
    @pkgid="$(cargo pkgid -p skillmgr-cli)"; crate="v${pkgid##*#}"; \
    if [ "$crate" != "{{version}}" ]; then \
        echo "tag {{version}} does not match crate version $crate" >&2; exit 1; \
    fi

# Build the static release artifacts and their checksums into dist/
dist version: (check-version version)
    rm -rf dist && mkdir -p dist
    for target in {{release_targets}}; do \
        cargo build --release --locked --target "$target" -p skillmgr-cli; \
        name="skillmgr-{{version}}-$target"; \
        tar -czf "dist/$name.tar.gz" -C "target/$target/release" skillmgr; \
        ( cd dist && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256" ); \
    done
    @ls -1 dist

# Print the SRI hashes nix/binary.nix pins, read from the published release.
# Local builds are not bit-identical to the ones CI publishes, so hashing dist/
# would give values that never match what fetchurl downloads.
release-hashes version:
    @for target in {{release_targets}}; do \
        url="https://github.com/sshine/skillmgr/releases/download/{{version}}/skillmgr-{{version}}-$target.tar.gz"; \
        hash="$(nix store prefetch-file --json --hash-type sha256 "$url" | jq -r .hash)"; \
        printf '"%s" = "%s";\n' "${target%%-*}-linux" "$hash"; \
    done

# Run CI checks locally
ci: fmt-check lock-check lint lint-nix test build doc readme-check package-check
    @echo "All CI checks passed!"

clean:
    cargo clean
