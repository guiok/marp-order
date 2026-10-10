# Without DECK, marp-order reopens the files of the last session.
DECK ?=
BIN  := target/release/marp-order
SRC  := Cargo.toml $(wildcard src/*)

.PHONY: run build test install help

run: $(BIN) ## run it, reopening the last session (or DECK=presentation.md)
	$(BIN) $(if $(DECK),"$(DECK)")

build: $(BIN) ## build the release binary

$(BIN): $(SRC)
	cargo build --release

test: ## run the parser, server and interface tests
	cargo test
	node --test tests/ui_logic.test.js

install: $(BIN) ## copy the binary to ~/.local/bin
	install -m 755 $(BIN) $(HOME)/.local/bin/marp-order

help: ## list the targets
	@grep -E '^[a-z]+:.*##' $(MAKEFILE_LIST) | sed -E 's/:.*## /\t/'
