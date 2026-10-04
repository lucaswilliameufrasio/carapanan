SHELL := /bin/bash
.DEFAULT_GOAL := dev

.PHONY: dev
dev:
	@set -em; \
	log_dir="$${TMPDIR:-/tmp}"; \
	if [ -d /tmp/opencode ]; then log_dir=/tmp/opencode; fi; \
	web_log=$$(mktemp "$$log_dir/carapana-web.log.XXXXXX"); \
	pnpm dev >"$$web_log" 2>&1 & web_pid=$$!; \
	trap 'kill -TERM -- -$$web_pid 2>/dev/null || true; wait $$web_pid 2>/dev/null || true' EXIT; \
	trap 'exit 130' INT; trap 'exit 143' TERM; \
	printf 'Web: http://127.0.0.1:5173 · logs: %s\n' "$$web_log"; \
	cargo run --manifest-path prototypes/terminal/Cargo.toml --
