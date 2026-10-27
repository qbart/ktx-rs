.PHONY: build install publish publish-macros publish-ktx

PREFIX ?= $(HOME)

# ./bin/ktx with its man pages in ./share/man/man1 (where `ktx help` looks: <exe>/../share/man/man1).
build:
	cargo build --release --bin ktx
	mkdir -p bin share/man/man1
	cp target/release/ktx bin/ktx
	cp man/*.1 share/man/man1/

install: build
	mkdir -p $(PREFIX)/bin $(PREFIX)/share/man/man1
	cp bin/ktx $(PREFIX)/bin/ktx
	cp man/*.1 $(PREFIX)/share/man/man1/

# crates.io: ktx-rs depends on ktx-rs-macros, so the macros crate goes first.
publish:
	$(MAKE) publish-macros
	$(MAKE) publish-ktx

publish-macros:
	cargo publish -p ktx-rs-macros

publish-ktx:
	cargo publish -p ktx-rs
