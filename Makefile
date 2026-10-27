.PHONY: build install

PREFIX ?= $(HOME)

build:
	cargo build --release --bin ktx
	mkdir -p bin
	cp target/release/ktx bin/ktx

install: build
	mkdir -p $(PREFIX)/bin
	cp bin/ktx $(PREFIX)/bin/ktx
