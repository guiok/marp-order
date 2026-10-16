# Without DECK, marp-order reopens the files of the last session.
DECK ?=
UNAME := $(shell uname -s)
# One build folder per system: the project may sit on a shared drive used from Linux and
# macOS, and a binary built on one cannot run on the other.
TARGET_DIR := target/$(UNAME)-$(shell uname -m)
BIN  := $(TARGET_DIR)/release/marp-order
SRC  := Cargo.toml $(wildcard src/*)

.PHONY: run build test install deps help

run: $(BIN) ## run it, reopening the last session (or DECK=presentation.md)
	$(BIN) $(if $(DECK),"$(DECK)")

build: $(BIN) ## build the release binary

$(BIN): $(SRC)
	cargo build --release --target-dir $(TARGET_DIR)

test: ## run the parser, server and interface tests
	cargo test --target-dir $(TARGET_DIR)
	node --test tests/ui_logic.test.js

install: $(BIN) ## copy the binary to ~/.local/bin
	install -m 755 $(BIN) $(HOME)/.local/bin/marp-order

deps: ## install node and marp-cli, needed for thumbnails and the tests (Rust itself: https://rustup.rs)
	@command -v node >/dev/null 2>&1 || { \
		echo "Installing Node.js..."; \
		if [ "$(UNAME)" = "Darwin" ]; then \
			command -v brew >/dev/null 2>&1 || { echo "Homebrew not found; install it from https://brew.sh"; exit 1; }; \
			brew install node; \
		elif command -v apt-get >/dev/null 2>&1; then sudo apt-get update && sudo apt-get install -y nodejs npm; \
		elif command -v dnf >/dev/null 2>&1; then sudo dnf install -y nodejs npm; \
		elif command -v pacman >/dev/null 2>&1; then sudo pacman -S --noconfirm nodejs npm; \
		else echo "No supported package manager found; install Node.js manually."; exit 1; \
		fi; \
	}
	npm install -g @marp-team/marp-cli

help: ## list the targets
	@grep -E '^[a-z]+:.*##' $(MAKEFILE_LIST) | sed -E 's/:.*## /\t/'
