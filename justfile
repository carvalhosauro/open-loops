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
# local preview only; release changelog is updated by release-plz on Release PR merge
changelog:
    git cliff -o CHANGELOG.md

# require: cargo install --git https://github.com/asciinema/agg
# render the asciinema cast to the README demo GIF
demo-gif:
    agg docs/demo.cast docs/demo.gif

# deterministic stress benchmark (use `just stress --heavy` for the big scales)
stress *ARGS:
    bash scripts/stress/bench.sh {{ARGS}}

# black-box behavior regression (CI-able; nonzero exit on any failure)
regress *ARGS:
    bash scripts/stress/regress.sh {{ARGS}}
