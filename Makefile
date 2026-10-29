.PHONY: build install wasm test-wasm js test-js test-js-browser webgl publish publish-macros publish-ktx

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

# wasm32: the library builds without threads or SIMD; the GPU loader features are no-ops there.
wasm:
	cargo build --lib --target wasm32-unknown-unknown
	cargo build --lib --all-features --target wasm32-unknown-unknown

# Runs the test suite under wasmtime (WASI preview 1); WASMTIME overrides the runner binary.
WASMTIME ?= wasmtime
test-wasm:
	CARGO_TARGET_WASM32_WASIP1_RUNNER="$(WASMTIME) run --dir=. --dir=$(CURDIR) --dir=/tmp --" \
		cargo test --release --target wasm32-wasip1 $(ARGS)

# JS binding (bindings/js/dist): needs wasm-bindgen-cli matching the wasm-bindgen crate version.
js:
	bindings/js/build.sh msc msc_basis_transcoder MSC_TRANSCODER
	bindings/js/build.sh read libktx_read createKtxReadModule "gl.js libktx_common.js" module_create_compat.js
	bindings/js/build.sh write libktx createKtxModule "gl.js libktx_common.js" module_create_compat.js
	cp bindings/js/dist/*.js bindings/js/dist/*.wasm bindings/js/tests/webgl/

test-js: js
	node --test bindings/js/tests/*.test.js

# glUpload on real WebGL 2: headless Chrome through chromedriver (CHROMEDRIVER=, CHROME= paths;
# Chrome for Testing's chrome-headless-shell works).
# Also drives the tests/webgl demo pages (llt-three needs unpkg.com, skipped when offline).
test-js-browser: js
	node bindings/js/tests/browser/run.js

# Serves the tests/webgl demo pages on http://localhost:8000/ (upstream tests/webgl).
webgl: js
	cd bindings/js/tests/webgl && python3 -m http.server 8000

# crates.io: ktx-rs depends on ktx-rs-macros, so the macros crate goes first.
publish:
	$(MAKE) publish-macros
	$(MAKE) publish-ktx

publish-macros:
	cargo publish -p ktx-rs-macros

publish-ktx:
	cargo publish -p ktx-rs
