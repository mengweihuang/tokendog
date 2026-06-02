ROUTER_PYTHON := python/router-py

.PHONY: all build build-router build-router-release build-wheel test clean

all: build

build: build-router build-wheel

build-router:
	cargo build -p router

build-router-release:
	cargo build --release -p router

# python3 -m build python/router-py --wheel --outdir dist
build-wheel:
	cd $(ROUTER_PYTHON) && maturin build --release

test:
	cargo test -p router

clean:
	cargo clean -p router
	cargo clean -p router-py
	rm -rf target/wheels
