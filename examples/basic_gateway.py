"""Basic example: start router gateway with Python bindings.

This script starts the gateway on port 30000 with two backend workers.
Requests are load-balanced via round-robin.

Usage:
    python examples/basic_gateway.py

    # Then in another terminal:
    curl http://localhost:30000/health
    curl -X POST http://localhost:30000/v1/chat/completions \
        -H "Content-Type: application/json" \
        -d '{"model": "qwen", "messages": [{"role": "user", "content": "hello"}]}'
"""

from router import Router


def main():
    workers = [
        "http://192.168.1.10:8000",
        "http://192.168.1.20:8000",
    ]

    gateway = Router(
        worker_urls=workers,
        host="0.0.0.0",
        port=30000,
        request_timeout_secs=300,
        log_level="info",
    )

    print(f"Starting gateway on {gateway.host}:{gateway.port}")
    print(f"Workers: {gateway.worker_urls}")
    print(f"Log level: {gateway.log_level}")
    print()

    gateway.serve()


if __name__ == "__main__":
    main()
