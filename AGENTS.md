# General Agent Guidelines

> If a `AGENTS.local.md` file exists alongside this file, read and respect it--
> it contains developer-specific overrides that supplement this shared guidance.

## Development environment

* Before any work, check local Python venv and activate if one exists.
* Don't install pip packages outside the local Python venv if one exists.
* Rust workspace root is at `crates/Cargo.toml` -- run all cargo commands from the `crates/` directory (e.g. `cd crates && cargo build`).

## Code changes

* Add tests and update docs for the changed code.
* Before creating commits, run `pre-commit run --all-files` to format.
* When creating commits, perform sign off on behalf of the author.
