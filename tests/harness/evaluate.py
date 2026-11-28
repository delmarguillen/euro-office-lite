# evaluate.py INSPECT PORT FILE: evaluates the JavaScript of FILE in the page
# of the app, through the inspector on PORT and the Inspector class of INSPECT
# (ci/eo-inspect.py), and prints its value. Exit status: 0, 1 failed, 3 no page.
import importlib.util
import socket
import sys
import time

# The server sometimes names no page to a connection made right after another
# one ended: seconds a connection waits for the page, and tries.
CONNECT_SECONDS = 2
CONNECT_TRIES = 8
# Seconds the page gets to answer. The expression is evaluated once: only
# the connection is tried again, so a `do` never runs twice.
EVALUATE_SECONDS = 30


# Loaded by its path; no __pycache__ is left next to it.
def load_inspector(path):
    sys.dont_write_bytecode = True
    spec = importlib.util.spec_from_file_location('eo_inspect', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


# A connection and the page it was told about, or None.
def connect(eo, port):
    try:
        inspector = eo.Inspector('127.0.0.1', port, eo.DEFAULT_PATH, CONNECT_SECONDS)
    except (OSError, RuntimeError):
        return None
    try:
        return inspector, inspector.wait_target()
    except (RuntimeError, socket.timeout):
        inspector.close()
        return None


def connect_with_retries(eo, port):
    for _ in range(CONNECT_TRIES):
        connected = connect(eo, port)
        if connected:
            return connected
        time.sleep(0.5)
    return None


def main(inspect_path, port, file_path):
    eo = load_inspector(inspect_path)
    with open(file_path, encoding='utf-8') as source:
        expression = source.read()
    connected = connect_with_retries(eo, int(port))
    if not connected:
        print(f'the inspector on port {port} named no page', file=sys.stderr)
        return 3
    inspector, target = connected
    inspector.sock.settimeout(EVALUATE_SECONDS)
    try:
        print(eo.render(inspector.evaluate(target, expression)))
        return 0
    except (RuntimeError, socket.timeout) as error:
        print(f'the evaluation failed: {error}', file=sys.stderr)
        return 1
    finally:
        inspector.close()


sys.exit(main(*sys.argv[1:4]))
