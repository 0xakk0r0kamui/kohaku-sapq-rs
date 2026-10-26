check:
    cd crates && cargo check --all-targets --all-features
    cd crates && cargo clippy --all-targets --all-features -- -D warnings

test: check
    cd crates && cargo test --release --all-targets --all-features
    cd crates && cargo test --release --all-targets --all-features -- --ignored

changelog crate:
    #!/usr/bin/env bash
    set -euo pipefail
    dir=$(cd crates && cargo pkgid -p {{crate}} | sed -E 's|^path\+file://||; s|#.*||')
    git cliff --include-path "$(realpath --relative-to=. "$dir")/**" --tag-pattern '^{{crate}}-v[0-9]' --unreleased

# Dry-run publish
prepare crate:
    #!/usr/bin/env bash
    set -euo pipefail
    cd crates
    cargo clippy -p {{crate}} --all-targets --all-features -- -D warnings
    cargo test -p {{crate}} --doc
    # Semver checks need a published baseline, so they're skipped on a crate's first release.
    if git tag -l '{{crate}}-v*' | grep -q .; then
        cargo semver-checks -p {{crate}}
    fi
    cargo publish -p {{crate}} --dry-run

publish crate:
    #!/usr/bin/env bash
    set -euo pipefail
    cd crates
    tag="{{crate}}-v$(cargo pkgid -p {{crate}} | sed 's/.*@//')"
    cargo publish -p {{crate}}
    git tag -a "$tag" -m "Release $tag"
    git push origin "$tag"
