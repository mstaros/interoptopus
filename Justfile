alias b := build
alias t := test
alias d := docs
alias doc := docs

# Runs all tests CI would perform before merging a PR.
[arg("verbose", long="verbose", short="v", value="--verbose")]
ci verbose="": (build verbose) (test verbose) test-dotnet lint

# Builds the workspace with all features.
[arg("verbose", long="verbose", short="v", value="--verbose")]
build verbose="":
    cargo build --all-features {{ verbose }}

# Run unit tests, check semantic correctness.
#
# The .NET reverse-interop plugins are built by `define_plugin!` during the test run itself,
# so a .NET 11 preview SDK must be on PATH. There is no separate plugin build step: an
# out-of-band one let the DLLs drift from the interop sources and surfaced as `ApiMismatch`.
[arg("verbose", long="verbose", short="v", value="--verbose")]
test verbose="" package="":
    cargo nextest run --all-features {{ verbose }}
    cargo test --doc --all-features

# Runs .NET tests.
test-dotnet:
    # Make sure the DLL + Interop files exist
    cargo build -p reference_project  --all-features
    cargo test --test mod reference_project::interop  --all-features
    cd crates/backend_csharp && dotnet test --project tests/reference_project/Tests/Tests.csproj

# Runs .NET benchmarks.
bench-dotnet:
    # Make sure the DLL + Interop files exist
    cargo build -p reference_project --release  --all-features
    cargo test --test mod reference_project::interop  --all-features
    dotnet run -c Release --project crates/backend_csharp/benches/dotnet/dotnet_benchmarks.csproj

# Run linters, check tidiness.
lint:
    cargo fmt --check
    cargo clippy -- -D warnings
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
    diff -q crates/core/README.md README.md # Make sure top-level README is up to date.

# Install all required tools, needs `binstall`, see https://github.com/cargo-bins/cargo-binstall
binstall-deps force="":
    cargo binstall cargo-insta --disable-telemetry --no-confirm --secure {{ force }}
    cargo binstall cargo-nextest --disable-telemetry --no-confirm --secure {{ force }}

# Opens cargo docs using nightly for doc feature bubbles.
docs open="":
    RUSTDOCFLAGS="--cfg docsrs" cargo +nightly doc --no-deps --all-features {{ open }}

# Updates the top-level README from the core crate's README (the source of truth).
update-readme:
    cp crates/core/README.md README.md

# Update UI snapshots (.snap and .stderr) files
update-snapshots:
    # find . -name "*.snap" -delete
    INSTA_UPDATE=always TRYBUILD=overwrite cargo nextest run --all-features

# Generate 8 random 128-bit IDs in hex format.
ids:
    for i in $(seq 1 8); do od -An -tx1 -N16 /dev/urandom | tr -d ' \n' | sed 's/^/0x/' | tr 'a-f' 'A-F'; echo; done

# Can be used by agents for the current task.
test-agent:
    # Agents: Feel free to update the test logic here for the task at hand.
    cargo build -p reference_project --all-features
    cargo test -p interoptopus_csharp --test mod reference_project::interop --all-features
    cd crates/backend_csharp && dotnet test --project tests/reference_project/Tests/Tests.csproj
