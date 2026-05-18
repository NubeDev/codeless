# Codeless dev orchestration.
#
# Targets:
#   make start           launch backend + UI in the background, seed demo DB on first run
#   make stop            kill both via PID files
#   make restart         stop + start
#   make backend         launch only the backend (background, logs to .codeless-dev/backend.log)
#   make ui              launch only the UI dev server (background, logs to .codeless-dev/ui.log)
#   make backend-fg      run the backend in the foreground (Ctrl-C to stop)
#   make ui-fg           run the UI in the foreground
#   make demo-seed       force-seed the demo repo + mock job into the dev DB
#   make logs            tail both log files
#   make status          report which dev processes are running
#   make clean           stop + wipe .codeless-dev/ (db, pid files, logs)
#
# State lives in .codeless-dev/ so it does not collide with anything in
# the repo. The dev DB at .codeless-dev/codeless.db is reused across
# `make start` invocations; delete it (or run `make clean`) for a
# fresh seed.

SHELL := /bin/bash

DEV_DIR     := .codeless-dev
DB          := $(DEV_DIR)/codeless.db
BACKEND_PID := $(DEV_DIR)/backend.pid
BACKEND_LOG := $(DEV_DIR)/backend.log
UI_PID      := $(DEV_DIR)/ui.pid
UI_LOG      := $(DEV_DIR)/ui.log

# Bind addresses are the same defaults documented in README.md; keeping
# them parametric lets the operator override per-invocation, e.g.
# `make backend BACKEND_BIND=127.0.0.1:7780`.
BACKEND_BIND ?= 127.0.0.1:7777
UI_PORT      ?= 5173
FS_ROOT      ?= $(CURDIR)
# Derived so fuser can reference just the port number.
BACKEND_PORT  = $(word 2,$(subst :, ,$(BACKEND_BIND)))

CARGO_CMD := cargo run -p codeless-cli --
PNPM_CMD  := pnpm -C ui/codeless-ui

.PHONY: start stop kill restart backend ui backend-fg ui-fg demo-seed logs status clean help ci

help:
	@echo "codeless dev:"
	@echo "  make start       launch backend + UI (background)"
	@echo "  make stop        kill both"
	@echo "  make restart     stop + start"
	@echo "  make status      who is running"
	@echo "  make logs        tail backend + UI logs"
	@echo "  make ci          fmt --check + clippy -D warnings + test --workspace"

ci:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace
	@echo "  make backend     backend only (background)"
	@echo "  make ui          UI only (background)"
	@echo "  make backend-fg  backend in foreground"
	@echo "  make ui-fg       UI in foreground"
	@echo "  make demo-seed   seed demo repo + mock job"
	@echo "  make kill        force-kill backend + UI by process name"
	@echo "  make clean       stop + wipe $(DEV_DIR)/"

$(DEV_DIR):
	@mkdir -p $(DEV_DIR)

# Seed the demo DB only when it does not exist. Subsequent `make start`
# runs reuse whatever rows the operator has accumulated. Running this
# target directly (`make demo-seed`) bypasses the existence check.
$(DB): | $(DEV_DIR)
	@echo "seeding demo repo + mock job into $(DB)"
	@$(CARGO_CMD) --db $(DB) demo bootstrap >/dev/null

demo-seed: | $(DEV_DIR)
	@echo "seeding demo repo + mock job into $(DB)"
	@$(CARGO_CMD) --db $(DB) demo bootstrap

# Background launchers. `setsid` detaches the child from this shell's
# session so closing the terminal does not deliver SIGHUP. `disown`
# would be enough on bash but breaks under POSIX sh; setsid is the
# portable choice and avoids the `nohup ... &` log-file dance.
backend: | $(DEV_DIR) $(DB)
	@if [ -f $(BACKEND_PID) ] && kill -0 $$(cat $(BACKEND_PID)) 2>/dev/null; then \
	    echo "backend already running (pid $$(cat $(BACKEND_PID)))"; \
	    exit 0; \
	fi
	@echo "starting backend at http://$(BACKEND_BIND) (logs: $(BACKEND_LOG))"
	@setsid bash -c '$(CARGO_CMD) --db $(DB) serve \
	    --bind $(BACKEND_BIND) \
	    --fs-root "$(FS_ROOT)" \
	    --enable-claude \
	    >$(BACKEND_LOG) 2>&1 & echo $$! >$(BACKEND_PID)' < /dev/null
	@sleep 1
	@if ! kill -0 $$(cat $(BACKEND_PID)) 2>/dev/null; then \
	    echo "backend failed to start; last log lines:"; \
	    tail -20 $(BACKEND_LOG); \
	    rm -f $(BACKEND_PID); \
	    exit 1; \
	fi
	@echo "backend running (pid $$(cat $(BACKEND_PID)))"

ui: | $(DEV_DIR)
	@if [ -f $(UI_PID) ] && kill -0 $$(cat $(UI_PID)) 2>/dev/null; then \
	    echo "ui already running (pid $$(cat $(UI_PID)))"; \
	    exit 0; \
	fi
	@if [ ! -d ui/codeless-ui/node_modules ]; then \
	    echo "installing UI deps (first run)"; \
	    $(PNPM_CMD) install; \
	fi
	@echo "starting UI at http://127.0.0.1:$(UI_PORT) (logs: $(UI_LOG))"
	@setsid bash -c '$(PNPM_CMD) dev --port $(UI_PORT) \
	    >$(UI_LOG) 2>&1 & echo $$! >$(UI_PID)' < /dev/null
	@sleep 1
	@if ! kill -0 $$(cat $(UI_PID)) 2>/dev/null; then \
	    echo "ui failed to start; last log lines:"; \
	    tail -20 $(UI_LOG); \
	    rm -f $(UI_PID); \
	    exit 1; \
	fi
	@echo "ui running (pid $$(cat $(UI_PID)))"

start: backend ui
	@echo ""
	@echo "ready: open http://127.0.0.1:$(UI_PORT)"
	@echo "logs:  make logs"
	@echo "stop:  make stop"

# Foreground variants for when you want to watch a single component in
# isolation (e.g. attaching a debugger, running with RUST_LOG=debug).
# They never touch the PID files so `make stop` will not interfere.
backend-fg: | $(DEV_DIR) $(DB)
	$(CARGO_CMD) --db $(DB) serve --bind $(BACKEND_BIND) --fs-root "$(FS_ROOT)" --enable-claude

ui-fg:
	@if [ ! -d ui/codeless-ui/node_modules ]; then $(PNPM_CMD) install; fi
	$(PNPM_CMD) dev --port $(UI_PORT)

# Kill by PID. `setsid` made each child its own session leader, so
# negating the PID delivers SIGTERM to the whole process group, taking
# down vite's worker and any subprocess `codeless serve` spawned.
stop:
	@stopped=0; \
	for pair in "backend:$(BACKEND_PID)" "ui:$(UI_PID)"; do \
	    name=$${pair%%:*}; pidfile=$${pair##*:}; \
	    if [ -f $$pidfile ]; then \
	        pid=$$(cat $$pidfile); \
	        if kill -0 $$pid 2>/dev/null; then \
	            kill -TERM -$$pid 2>/dev/null || kill -TERM $$pid 2>/dev/null || true; \
	            sleep 0.5; \
	            kill -0 $$pid 2>/dev/null && kill -KILL -$$pid 2>/dev/null; \
	            echo "stopped $$name (pid $$pid)"; \
	            stopped=1; \
	        else \
	            echo "$$name pid $$pid was stale"; \
	        fi; \
	        rm -f $$pidfile; \
	    fi; \
	done; \
	if [ $$stopped -eq 0 ]; then echo "nothing to stop"; fi

# Force-kill by port. Using fuser avoids the pkill self-match bug where
# the pattern string appears verbatim in the spawning shell's cmdline.
kill:
	@fuser -k $(BACKEND_PORT)/tcp 2>/dev/null \
	    && echo "killed backend (port $(BACKEND_PORT))" \
	    || echo "backend not running (port $(BACKEND_PORT) clear)"
	@fuser -k $(UI_PORT)/tcp 2>/dev/null \
	    && echo "killed ui (port $(UI_PORT))" \
	    || echo "ui not running (port $(UI_PORT) clear)"
	@rm -f $(BACKEND_PID) $(UI_PID)

restart: stop start

status:
	@for pair in "backend:$(BACKEND_PID):$(BACKEND_BIND)" "ui:$(UI_PID):127.0.0.1:$(UI_PORT)"; do \
	    name=$${pair%%:*}; rest=$${pair#*:}; pidfile=$${rest%%:*}; addr=$${rest#*:}; \
	    if [ -f $$pidfile ] && kill -0 $$(cat $$pidfile) 2>/dev/null; then \
	        echo "$$name: running (pid $$(cat $$pidfile), $$addr)"; \
	    else \
	        echo "$$name: stopped"; \
	    fi; \
	done

logs:
	@touch $(BACKEND_LOG) $(UI_LOG)
	@tail -F $(BACKEND_LOG) $(UI_LOG)

clean: stop
	@rm -rf $(DEV_DIR)
	@echo "removed $(DEV_DIR)/"
