.PHONY: all frontend

all: frontend
	cargo build

frontend: webui/node_modules/.install-stamp
	cd webui && npm run build

webui/node_modules/.install-stamp: webui/package.json webui/package-lock.json
	cd webui && npm ci
	touch $@
