SHELL := /bin/bash
.DEFAULT_GOAL := dev

.PHONY: dev
dev:
	@set -em; \
	export HOST="$${HOST:-127.0.0.1}" PORT="$${PORT:-5173}"; \
	web_host="$$HOST"; case "$$web_host" in *:*) web_host="[$$web_host]" ;; esac; \
	log_dir="$${TMPDIR:-/tmp}"; \
	if [ -d /tmp/opencode ]; then log_dir=/tmp/opencode; fi; \
	web_log=$$(mktemp "$$log_dir/carapana-web.log.XXXXXX"); \
	pnpm dev >"$$web_log" 2>&1 & web_pid=$$!; \
	trap 'kill -TERM -- -$$web_pid 2>/dev/null || true; wait $$web_pid 2>/dev/null || true' EXIT; \
	trap 'exit 130' INT; trap 'exit 143' TERM; \
	printf 'Web: http://%s:%s · logs: %s\n' "$$web_host" "$$PORT" "$$web_log"; \
	cargo run --manifest-path prototypes/terminal/Cargo.toml --
