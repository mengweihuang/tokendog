"""Command-line entry point for the router gateway."""

import argparse
import secrets
import sys

from router import Router


def _add_args(parser: argparse.ArgumentParser) -> None:
    """Add top-level arguments."""
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
        default=[],
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
    parser.add_argument(
        "--genkey",
        action="store_true",
        help="Generate an API key (sk- prefix, 32 hex chars) and exit.",
    )
    # ── K8s service discovery ──────────────────────────────────────────
    parser.add_argument(
        "--k8s-selector",
        nargs="+",
        default=[],
        help="K8s label selector for worker pods (key=value pairs). Enables K8s service discovery.",
    )
    parser.add_argument(
        "--k8s-namespace",
        default=None,
        help="K8s namespace to watch (omit for all namespaces).",
    )
    parser.add_argument(
        "--k8s-port",
        type=int,
        default=8000,
        help="Port workers listen on inside pods (default: 8000).",
    )
    parser.add_argument(
        "--k8s-check-interval",
        type=int,
        default=60,
        help="K8s reconciliation check interval in seconds (default: 60).",
    )
    parser.add_argument(
        "--k8s-prefill-selector",
        nargs="+",
        default=[],
        help="K8s label selector for prefill pods in PD mode (key=value pairs).",
    )
    parser.add_argument(
        "--k8s-decode-selector",
        nargs="+",
        default=[],
        help="K8s label selector for decode pods in PD mode (key=value pairs).",
    )


def _cmd_serve(args: argparse.Namespace) -> None:
    """Start the router gateway."""
    # Detect K8s mode: any k8s selector enables it.
    k8s_enabled = bool(args.k8s_selector or args.k8s_prefill_selector or args.k8s_decode_selector)

    if not k8s_enabled and not args.worker_urls:
        print(
            "error: either --worker-urls or --k8s-selector is required",
            file=sys.stderr,
        )
        sys.exit(2)

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

    k8s_selector: list[str] = []
    for u in args.k8s_selector:
        k8s_selector.extend(u.split(","))

    k8s_prefill_selector: list[str] = []
    for u in args.k8s_prefill_selector:
        k8s_prefill_selector.extend(u.split(","))

    k8s_decode_selector: list[str] = []
    for u in args.k8s_decode_selector:
        k8s_decode_selector.extend(u.split(","))

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
        k8s_selector=k8s_selector if k8s_selector else None,
        k8s_namespace=args.k8s_namespace,
        k8s_port=args.k8s_port,
        k8s_check_interval_secs=args.k8s_check_interval,
        k8s_prefill_selector=k8s_prefill_selector if k8s_prefill_selector else None,
        k8s_decode_selector=k8s_decode_selector if k8s_decode_selector else None,
    )
    gateway.serve()


def _cmd_genkey(args: argparse.Namespace) -> None:
    """Generate and print an API key."""
    key = "sk-" + secrets.token_hex(16)
    print(key)


def main() -> None:
    """Parse CLI arguments and start the gateway, or generate a key."""
    parser = argparse.ArgumentParser(
        description="LLM gateway for vLLM/SGLang inference engines."
    )
    _add_args(parser)

    args = parser.parse_args()

    if args.genkey:
        _cmd_genkey(args)
    else:
        _cmd_serve(args)


if __name__ == "__main__":
    main()
