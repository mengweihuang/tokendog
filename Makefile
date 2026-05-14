ROUTER_PYTHON := crates/router-python

.PHONY: all build build-router build-router-release build-wheel test clean

all: build

build: build-router build-wheel

build-router:
	cargo build -p router

build-router-release:
	cargo build --release -p router

build-wheel:
	cd $(ROUTER_PYTHON) && maturin build --release

test:
	cargo test -p router

clean:
	cargo clean -p router
	cargo clean -p router-python
	rm -rf ./target/wheels
