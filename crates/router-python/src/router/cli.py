"""Command-line entry point for the router gateway."""

import argparse
import secrets
import sys

from router import Router


def _add_serve_args(parser: argparse.ArgumentParser) -> None:
    """Add serve subcommand arguments."""
    parser.add_argument(
        "--host",
        default="0.0.0.0",
        help="Bind address (default: 0.0.0.0)",
    )
    parser.add_argument(
        "--port",
        type=int,
        default=30000,
        help="Bind port (default: 30000)",
    )
    parser.add_argument(
        "--worker-urls",
        nargs="+",
        required=True,
        help="Worker URL(s). Accepts space-separated and/or comma-separated values.",
    )
    parser.add_argument(
        "--request-timeout-secs",
        type=int,
        default=300,
        help="Timeout in seconds for upstream requests (default: 300)",
    )
    parser.add_argument(
        "--log-level",
        default="info",
        choices=["error", "warn", "info", "debug"],
        help="Log level (default: info)",
    )
    parser.add_argument(
        "--policy",
        default="least-loaded",
        choices=[
            "least-loaded",
            "power-of-two",
            "random",
            "round-robin",
            "session-affinity",
            "prefix-affinity",
            "load-cache-aware",
        ],
        help="Load-balancing policy (default: least-loaded)",
    )
    parser.add_argument(
        "--pd-mode",
        default=None,
        choices=["vllm", "sglang"],
        help="Prefill-Decode separation mode: vllm or sglang (default: None, disabled)",
    )
    parser.add_argument(
        "--prefill-urls",
        nargs="+",
        default=[],
        help="Prefill worker URL(s) for PD mode. Accepts space-separated and/or comma-separated values.",
    )
    parser.add_argument(
        "--decode-urls",
        nargs="+",
        default=[],
        help="Decode worker URL(s) for PD mode. Accepts space-separated and/or comma-separated values.",
    )
    parser.add_argument(
        "--data-plane-api-keys",
        nargs="+",
        default=[],
        help="API key(s) for data plane Bearer token authentication. Accepts space-separated and/or comma-separated values.",
    )


def _cmd_serve(args: argparse.Namespace) -> None:
    """Start the router gateway."""
    worker_urls: list[str] = []
    for u in args.worker_urls:
        worker_urls.extend(u.split(","))

    prefill_urls: list[str] = []
    for u in args.prefill_urls:
        prefill_urls.extend(u.split(","))

    decode_urls: list[str] = []
    for u in args.decode_urls:
        decode_urls.extend(u.split(","))

    data_plane_api_keys: list[str] = []
    for u in args.data_plane_api_keys:
        data_plane_api_keys.extend(u.split(","))

    gateway = Router(
        worker_urls=worker_urls,
        host=args.host,
        port=args.port,
        request_timeout_secs=args.request_timeout_secs,
        log_level=args.log_level,
        policy=args.policy,
        pd_mode=args.pd_mode,
        prefill_urls=prefill_urls if prefill_urls else None,
        decode_urls=decode_urls if decode_urls else None,
        data_plane_api_keys=data_plane_api_keys if data_plane_api_keys else None,
    )
    gateway.serve()


def _cmd_genkey(args: argparse.Namespace) -> None:
    """Generate and print an API key."""
    key = "sk-" + secrets.token_hex(16)
    print(key)


def main(default_command: str | None = None) -> None:
    """Parse CLI arguments and dispatch to the appropriate subcommand."""
    parser = argparse.ArgumentParser(
        description="LLM gateway for vLLM/SGLang inference engines."
    )
    subparsers = parser.add_subparsers(dest="command", title="commands")
    subparsers.required = False

    serve_parser = subparsers.add_parser("serve", help="Start the gateway server")
    _add_serve_args(serve_parser)

    subparsers.add_parser("genkey", help="Generate an API key (sk- prefix, 32 hex chars)")

    args = parser.parse_args()

    command = args.command or default_command

    if command == "genkey":
        _cmd_genkey(args)
    elif command == "serve":
        _cmd_serve(args)
    else:
        parser.print_help()
        sys.exit(1)


if __name__ == "__main__":
    main()
