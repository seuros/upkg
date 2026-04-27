set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]

export RUSTFLAGS := "-D warnings"

fmt *ARGS:
    cargo fmt --all {{ARGS}}

check:
    cargo check --workspace --all-targets

clippy *ARGS:
    cargo clippy --workspace --all-targets -- {{ARGS}}

test *ARGS:
    cargo test --workspace {{ARGS}}

qq: fmt check clippy test

qa: qq

run *ARGS:
    cargo run -p upkg -- {{ARGS}}

clean:
    cargo clean
