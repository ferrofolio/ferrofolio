# SPDX-License-Identifier: AGPL-3.0-only
"""Command line options of the Python server."""

import pytest

from args import app_args, public_port


def test_public_port_defaults_to_the_listening_port():
    args = app_args().parse_args(["--port", "7600"])

    assert args.public_port is None
    assert public_port(args) == 7600


def test_public_port_is_used_behind_the_gateway():
    args = app_args().parse_args(["--port", "41000", "--public-port", "7592"])

    assert args.port == 41000
    assert public_port(args) == 7592


@pytest.mark.parametrize("port", ["0", "65536", "-1"])
def test_public_port_must_be_a_tcp_port(port):
    with pytest.raises(SystemExit):
        app_args().parse_args(["--public-port", port])


def test_socket_descriptor_cannot_be_negative():
    with pytest.raises(SystemExit):
        app_args().parse_args(["--socket-fd", "-1"])
    assert app_args().parse_args(["--socket-fd", "3"]).socket_fd == 3
