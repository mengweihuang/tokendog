"""SGLang PD separation example: concurrent dual dispatch.

Starts the router gateway in SGLang Prefill-Decode mode. Inference requests
(/v1/chat/completions, /v1/completions, etc.) are sent simultaneously to both
prefill and decode workers with bootstrap_host, bootstrap_port, and bootstrap_room
injected into each request body. The decode response is returned to the client.

SGLang handles KV cache transfer natively using the bootstrap parameters — no
external connector is needed.

Prerequisites:
    Two SGLang instances configured for PD disaggregation:
    - Prefill worker(s) with a bootstrap server at --prefill-urls
    - Decode worker(s) at --decode-urls

Usage:
    python examples/pd_sglang_gateway.py

    # Or with custom URLs:
    python examples/pd_sglang_gateway.py \
        --prefill-urls http://10.0.0.1:30000 \
        --decode-urls http://10.0.0.2:30000

    # Then in another terminal:
    curl -X POST http://localhost:30000/v1/chat/completions \
        -H "Content-Type: application/json" \
        -d '{"model": "qwen", "messages": [{"role": "user", "content": "hello"}], "stream": true}'
"""

import argparse

from router import Router


def main() -> None:
    parser = argparse.ArgumentParser(
        description="SGLang PD separation gateway example"
    )
    parser.add_argument("--host", default="0.0.0.0")
    parser.add_argument("--port", type=int, default=30000)
    parser.add_argument(
        "--prefill-urls",
        nargs="+",
        default=["http://127.0.0.1:30001"],
    )
    parser.add_argument(
        "--decode-urls",
        nargs="+",
        default=["http://127.0.0.1:30002"],
    )
    parser.add_argument("--policy", default="least-loaded")
    parser.add_argument(
        "--log-level", default="info", choices=["error", "warn", "info", "debug"]
    )
    args = parser.parse_args()

    prefill_urls: list[str] = []
    for u in args.prefill_urls:
        prefill_urls.extend(u.split(","))

    decode_urls: list[str] = []
    for u in args.decode_urls:
        decode_urls.extend(u.split(","))

    gateway = Router(
        worker_urls=decode_urls,  # fallback for non-inference paths
        host=args.host,
        port=args.port,
        log_level=args.log_level,
        policy=args.policy,
        pd_mode="sglang",
        prefill_urls=prefill_urls,
        decode_urls=decode_urls,
    )

    print(f"SGLang PD gateway on {gateway.host}:{gateway.port}")
    print(f"  Prefill workers: {gateway.prefill_urls}")
    print(f"  Decode workers:  {gateway.decode_urls}")
    print(f"  Policy: {gateway.policy}")
    print(f"  PD mode: {gateway.pd_mode}")
    print()
    print("Inference requests → concurrent dual dispatch (prefill ⊕ decode)")
    print("Other requests → forwarded to decode workers")
    print()

    gateway.serve()


if __name__ == "__main__":
    main()
