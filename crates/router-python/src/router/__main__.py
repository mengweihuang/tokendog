"""Enable ``python -m router`` to start the gateway."""

from router.cli import main

main(default_command="serve")
