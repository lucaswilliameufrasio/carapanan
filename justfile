# Prototypes remain mock-only while Delivery 1 builds the engineering foundation.
dev:
    pnpm dev

tui:
    cargo run --manifest-path prototypes/terminal/Cargo.toml --

cli scenario="approval":
    cargo run --manifest-path prototypes/terminal/Cargo.toml -- run --scenario {{scenario}}

verify:
    make verify
