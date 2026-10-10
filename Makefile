# Sem DECK, o marp-order reabre os arquivos da última sessão.
DECK ?=
BIN  := target/release/marp-order
SRC  := Cargo.toml $(wildcard src/*)

.PHONY: abrir build test install help

abrir: $(BIN) ## abre os arquivos da última sessão (ou DECK=apresentacao.md)
	$(BIN) $(if $(DECK),"$(DECK)")

build: $(BIN) ## compila o binário de release

$(BIN): $(SRC)
	cargo build --release

test: ## roda os testes do parser/writer
	cargo test

install: $(BIN) ## copia o binário para ~/.local/bin
	install -m 755 $(BIN) $(HOME)/.local/bin/marp-order

help: ## lista os alvos
	@grep -E '^[a-z]+:.*##' $(MAKEFILE_LIST) | sed -E 's/:.*## /\t/'
