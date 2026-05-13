"""Command-line entry point for the router gateway."""

import argparse

from router import Router


def main() -> None:
    """Parse CLI arguments and start the router gateway."""
    parser = argparse.ArgumentParser(
        description="LLM gateway for vLLM/SGLang inference engines."
    )
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

    args = parser.parse_args()

    # Support both space-separated and comma-separated worker URLs,
    # matching the behavior of the Rust CLI binary.
    worker_urls: list[str] = []
    for u in args.worker_urls:
        worker_urls.extend(u.split(","))

    gateway = Router(
        worker_urls=worker_urls,
        host=args.host,
        port=args.port,
        request_timeout_secs=args.request_timeout_secs,
        log_level=args.log_level,
        policy=args.policy,
    )
    gateway.serve()


if __name__ == "__main__":
    main()
