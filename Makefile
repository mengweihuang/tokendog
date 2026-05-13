CARGO := cargo
MATURIN := maturin
CRATES := crates
ROUTER_PYTHON := crates/router-python

.PHONY: all build build-router build-router-release build-wheel test clean

all: build

build: build-router build-wheel

build-router:
	cd $(CRATES) && $(CARGO) build -p router

build-router-release:
	cd $(CRATES) && $(CARGO) build --release -p router

build-wheel:
	cd $(ROUTER_PYTHON) && $(MATURIN) build --release

test:
	cd $(CRATES) && $(CARGO) test -p router

clean:
	cd $(CRATES) && $(CARGO) clean -p router
	cd $(ROUTER_PYTHON) && $(CARGO) clean -p router-python
	rm -rf $(ROUTER_PYTHON)/target/wheels
