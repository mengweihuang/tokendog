"""vLLM PD separation example: sequential two-stage processing.

Starts the router gateway in vLLM Prefill-Decode mode. Inference requests
(/v1/chat/completions, /v1/completions, etc.) go through a two-stage pipeline:
1. Prefill stage: max_tokens=1 request to prefill worker, extracts kv_transfer_params
2. Decode stage: original request + kv_transfer_params to decode worker, streams response

Other requests (health, models, etc.) are forwarded directly to decode workers.

Prerequisites:
    Two vLLM instances configured for PD disaggregation:
    - Prefill worker(s) at --prefill-urls
    - Decode worker(s) at --decode-urls

Usage:
    python examples/pd_vllm_gateway.py

    # Or with custom URLs:
    python examples/pd_vllm_gateway.py \
        --prefill-urls http://10.0.0.1:8000 http://10.0.0.2:8000 \
        --decode-urls http://10.0.0.3:8000 http://10.0.0.4:8000

    # Then in another terminal:
    curl -X POST http://localhost:30000/v1/chat/completions \
        -H "Content-Type: application/json" \
        -d '{"model": "qwen", "messages": [{"role": "user", "content": "hello"}]}'
"""

import argparse

from router import Router


def main() -> None:
    parser = argparse.ArgumentParser(
        description="vLLM PD separation gateway example"
    )
    parser.add_argument("--host", default="0.0.0.0")
    parser.add_argument("--port", type=int, default=30000)
    parser.add_argument(
        "--prefill-urls",
        nargs="+",
        default=["http://127.0.0.1:8100"],
    )
    parser.add_argument(
        "--decode-urls",
        nargs="+",
        default=["http://127.0.0.1:8200"],
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
        pd_mode="vllm",
        prefill_urls=prefill_urls,
        decode_urls=decode_urls,
    )

    print(f"vLLM PD gateway on {gateway.host}:{gateway.port}")
    print(f"  Prefill workers: {gateway.prefill_urls}")
    print(f"  Decode workers:  {gateway.decode_urls}")
    print(f"  Policy: {gateway.policy}")
    print(f"  PD mode: {gateway.pd_mode}")
    print()
    print("Inference requests → two-stage pipeline (prefill → decode)")
    print("Other requests → forwarded to decode workers")
    print()

    gateway.serve()


if __name__ == "__main__":
    main()
