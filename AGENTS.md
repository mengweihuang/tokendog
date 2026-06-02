# General Agent Guidelines

> If a `AGENTS.local.md` file exists alongside this file, read and respect it--
> it contains developer-specific overrides that supplement this shared guidance.

## Development environment

* Before any work, check local Python venv and activate if one exists.
* Don't install pip packages outside the local Python venv if one exists.
* Refer to the Makefile and use make to build (e.g., `make build`).

## Code standards

* Add doc comments to public APIs and complex logic.
* Never hard-code secrets; use environment variables.
* Handle business exceptions gracefully; propagate unexpected errors globally.
* Keep lines under 200 characters.

## Code changes

* Add tests and update docs for the changed code.
* Before creating commits, run `pre-commit run --all-files` to format.
* When creating commits, perform sign off on behalf of the author.
