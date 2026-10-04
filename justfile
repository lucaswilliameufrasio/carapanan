# Delivery 0 only: no daemon, provider, database or agent execution.
dev:
    pnpm dev

tui:
    cargo run --manifest-path prototypes/terminal/Cargo.toml --

cli scenario="approval":
    cargo run --manifest-path prototypes/terminal/Cargo.toml -- run --scenario {{scenario}}

verify:
    pnpm format:check
    pnpm lint
    pnpm check
    pnpm test
    pnpm build
    pnpm test:e2e
    cargo fmt --manifest-path prototypes/terminal/Cargo.toml --check
    cargo clippy --manifest-path prototypes/terminal/Cargo.toml --all-targets -- -D warnings
    cargo test --manifest-path prototypes/terminal/Cargo.toml
    cargo build --manifest-path prototypes/terminal/Cargo.toml
