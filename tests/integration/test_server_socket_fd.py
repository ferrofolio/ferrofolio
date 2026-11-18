# SPDX-License-Identifier: AGPL-3.0-only
"""The server accepts connections on a listening socket inherited from the gateway."""

import argparse
import http.client
import os
import signal
import socket
import subprocess
import sys
from pathlib import Path

import pytest

FINANZE_DIR = Path(__file__).parents[2] / "finanze"
TIMEOUT_SECONDS = 60

SCRIPT = """
import argparse
import asyncio
import sys

import uvicorn

from server import inherited_sockets, serve


async def app(scope, receive, send):
    await send({"type": "http.response.start", "status": 204, "headers": []})
    await send({"type": "http.response.body", "body": b""})


async def main(fd):
    sockets = inherited_sockets(argparse.Namespace(socket_fd=fd))
    config = uvicorn.Config(app, lifespan="off", log_config=None)
    await serve(uvicorn.Server(config), sockets)


asyncio.run(main(int(sys.argv[1])))
"""


# The in-process imports find finanze/ through pytest's pythonpath setting
# (pyproject.toml), so the import path stays as it is for later tests.
def _inherited_sockets(fd):
    from server import inherited_sockets

    return inherited_sockets(argparse.Namespace(socket_fd=fd))


def test_no_socket_without_a_descriptor():
    assert _inherited_sockets(None) is None


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX file descriptors")
def test_rejects_descriptors_that_are_not_listening_stream_sockets():
    datagram = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    unbound = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        for sock in (datagram, unbound):
            with pytest.raises(OSError, match="not a listening stream socket"):
                _inherited_sockets(os.dup(sock.fileno()))
    finally:
        datagram.close()
        unbound.close()


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX file descriptors")
def test_serves_requests_on_an_inherited_socket():
    # The listener closes when the block ends, also if the child cannot be started;
    # afterwards only the child holds the socket.
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        port = listener.getsockname()[1]
        fd = listener.fileno()
        process = subprocess.Popen(
            [sys.executable, "-c", SCRIPT, str(fd)],
            cwd=FINANZE_DIR,
            env={**os.environ, "PYTHONPATH": str(FINANZE_DIR)},
            pass_fds=[fd],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
    try:
        # The connection waits in the backlog until the child accepts it.
        connection = http.client.HTTPConnection(
            "127.0.0.1", port, timeout=TIMEOUT_SECONDS
        )
        try:
            connection.request("GET", "/")
            status = connection.getresponse().status
        except OSError as error:
            process.kill()
            _, stderr = process.communicate(timeout=TIMEOUT_SECONDS)
            pytest.fail(f"The server did not answer: {error}\n{stderr}")
        finally:
            connection.close()
        assert status == 204
    finally:
        if process.poll() is None:
            process.send_signal(signal.SIGTERM)
        try:
            process.communicate(timeout=TIMEOUT_SECONDS)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()


def _server_module():
    import server

    return server


class _StopInit(Exception):
    pass


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX file descriptors")
@pytest.mark.asyncio
async def test_startup_skips_the_port_check_for_an_inherited_socket(monkeypatch):
    server = _server_module()
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        port = listener.getsockname()[1]
        args = argparse.Namespace(
            socket_fd=listener.fileno(), port=port, host="127.0.0.1"
        )
        instance = server.FerrofolioServer(args)

        def check_port():
            raise AssertionError("the port of an inherited socket was probed")

        def stop(*_args, **_kwargs):
            raise _StopInit()

        monkeypatch.setattr(instance, "_check_port", check_port)
        # The first step after the port check; stopping there keeps the test small.
        monkeypatch.setattr(server, "DBClient", stop)
        with pytest.raises(_StopInit):
            await instance._init()


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX file descriptors")
@pytest.mark.asyncio
async def test_run_serves_the_inherited_socket(monkeypatch):
    server = _server_module()
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        # The server takes over its own descriptor of the listener.
        fd = os.dup(listener.fileno())
        args = argparse.Namespace(
            socket_fd=fd, port=listener.getsockname()[1], host="127.0.0.1"
        )
        instance = server.FerrofolioServer(args)
        served = {}

        async def init():
            served["initialised"] = True

        async def serve(_server, sockets=None):
            served["fds"] = [sock.fileno() for sock in sockets]

        monkeypatch.setattr(instance, "_init", init)
        monkeypatch.setattr(server, "serve", serve)
        await instance.run()

        assert served["initialised"]
        assert served["fds"] == [fd]
        # run() releases the socket it took over once the server has stopped.
        with pytest.raises(OSError):
            os.fstat(fd)


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX file descriptors")
@pytest.mark.asyncio
async def test_run_releases_the_inherited_socket_when_startup_fails(monkeypatch):
    server = _server_module()
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        fd = os.dup(listener.fileno())
        args = argparse.Namespace(
            socket_fd=fd, port=listener.getsockname()[1], host="127.0.0.1"
        )
        instance = server.FerrofolioServer(args)

        async def init():
            raise _StopInit()

        # The default error reporter does nothing.
        monkeypatch.setattr(instance, "_init", init)
        with pytest.raises(_StopInit):
            await instance.run()

        with pytest.raises(OSError):
            os.fstat(fd)
