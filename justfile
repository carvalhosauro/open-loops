default: test

# instala hooks e valida toolchain
setup:
    @command -v lefthook >/dev/null || (echo "instale lefthook: https://lefthook.dev/installation/" && exit 1)
    lefthook install
    rustup show

test:
    cargo test

lint:
    cargo clippy --all-targets -- -D warnings

fmt:
    cargo fmt

# require: cargo install cargo-llvm-cov
cov:
    cargo llvm-cov --fail-under-lines 70

# require: cargo install git-cliff
changelog:
    git cliff -o CHANGELOG.md
