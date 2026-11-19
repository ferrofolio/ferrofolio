# SPDX-License-Identifier: AGPL-3.0-only
"""Parity fixtures for the routes that read the database.

harness.py creates the fixture databases and records the routes; each slice of
the port (settings.py, ...) defines the data it needs, the routes it records and
their cases. tests/parity/generate_db_route_fixtures.py runs them.
"""
