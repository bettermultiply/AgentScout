.PHONY: all frontend pi-deps install uninstall purge

BINDIR ?= $(HOME)/.local/bin
DATADIR ?= $(HOME)/.local/share/ascout
CONFIGDIR ?= $(HOME)/.config/ascout
PI_ROOT ?= $(shell if test -d "$(abspath ../pi)"; then printf '%s' "$(abspath ../pi)"; fi)

all: frontend pi-deps
	cargo build

frontend: webui/node_modules/.install-stamp
	cd webui && npm run build

webui/node_modules/.install-stamp: webui/package.json webui/package-lock.json
	cd webui && npm ci
	touch $@

pi-deps: instrumentation/pi/node_modules/.install-stamp

instrumentation/pi/node_modules/.install-stamp: instrumentation/pi/package.json instrumentation/pi/package-lock.json
	npm ci --prefix instrumentation/pi
	touch $@

install: frontend pi-deps
	@set -eu; \
	if test -z "$(PI_ROOT)" || test ! -d "$(PI_ROOT)"; then \
		echo "Pi source root is required for installation. Run PI_ROOT=/path/to/pi make install." >&2; \
		exit 1; \
	fi
	cargo build --release
	@set -eu; \
	mkdir -p "$(BINDIR)" "$(DATADIR)/policies" "$(DATADIR)/runs" "$(DATADIR)/instrumentation/pi" "$(DATADIR)/instrumentation/core" "$(CONFIGDIR)"; \
	install -m 755 target/release/ascout-control-plane "$(BINDIR)/ascout-control-plane"; \
	install -m 755 target/release/ascout-runtime-adapter "$(BINDIR)/ascout-runtime-adapter"; \
	install -m 755 target/release/ascout-fs-holder "$(BINDIR)/ascout-fs-holder"; \
	ln -sfn ascout-control-plane "$(BINDIR)/ascout"; \
	cp -a assets/policy/. "$(DATADIR)/policies/"; \
	cp -a instrumentation/pi/. "$(DATADIR)/instrumentation/pi/"; \
	cp -a instrumentation/core/. "$(DATADIR)/instrumentation/core/"; \
	if test -e "$(CONFIGDIR)/config.toml"; then echo "Keeping existing config $(CONFIGDIR)/config.toml"; else \
		sed -e 's|^policy_dir = .*|policy_dir = "$(DATADIR)/policies"|' \
		    -e 's|^runs_dir = .*|runs_dir = "$(DATADIR)/runs"|' \
		    -e 's|^log_path = .*|log_path = "$(DATADIR)/ascout.log"|' \
		    -e 's|^# pi_root = .*|pi_root = "$(PI_ROOT)"|' \
		    -e 's|^pi_root = .*|pi_root = "$(PI_ROOT)"|' config.example.toml > "$(CONFIGDIR)/config.toml"; \
		echo "Created $(CONFIGDIR)/config.toml"; fi

check-active-runs:
	@set -eu; \
	runs="$(DATADIR)/runs"; \
	if test -f "$(CONFIGDIR)/config.toml"; then configured=$$(sed -n 's|^[[:space:]]*runs_dir[[:space:]]*=[[:space:]]*"\([^"]*\)".*|\1|p' "$(CONFIGDIR)/config.toml" | head -n1); \
		test -z "$$configured" || runs="$$configured"; fi; \
	case "$$runs" in ~/*) runs="$(HOME)/$${runs#~/}";; esac; \
	if test -d "$$runs"; then for lock in "$$runs"/run-*/run.lock "$$runs"/session-*/run.lock; do \
		test -e "$$lock" || continue; \
		if flock -n "$$lock" -c true 2>/dev/null; then :; else echo "Active AScout run holds $$lock" >&2; exit 1; fi; \
	done; fi

uninstall: check-active-runs
	@rm -f "$(BINDIR)/ascout" "$(BINDIR)/ascout-control-plane" "$(BINDIR)/ascout-runtime-adapter" "$(BINDIR)/ascout-fs-holder"; \
	rm -rf "$(DATADIR)/policies" "$(DATADIR)/instrumentation"; \
	echo "Removed AScout binaries and installed policies. Configuration and runs were kept."

purge: check-active-runs
	@$(MAKE) uninstall BINDIR="$(BINDIR)" DATADIR="$(DATADIR)" CONFIGDIR="$(CONFIGDIR)"; \
	rm -rf "$(CONFIGDIR)" "$(DATADIR)"; \
	echo "Removed AScout configuration, logs, policies, and runs."
