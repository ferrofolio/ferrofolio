# SPDX-License-Identifier: AGPL-3.0-only
"""Regression checks for the temporary suspension of E2E CI execution."""

import json
from pathlib import Path

import pytest
from strictyaml import Bool, dirty_load

ROOT = Path(__file__).resolve().parents[2]


@pytest.fixture
def ci_workflow():
    # GitHub workflows use flow sequences, which StrictYAML disables by default.
    return dirty_load(
        (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8"),
        allow_flow_style=True,
    )


def test_e2e_is_disabled_at_job_level(ci_workflow):
    # A step-level condition would still provision the runner and install browsers.
    condition = ci_workflow["jobs"]["e2e"]["if"]
    condition.revalidate(Bool())
    assert condition.data is False


def test_disabled_e2e_retains_required_check_identity(ci_workflow):
    job = ci_workflow["jobs"]["e2e"].data
    assert job["name"] == "E2E Tests"
    assert job["needs"] == "changes"
    # Removing the job instead of skipping it can leave a required check pending.
    assert job["steps"]


def test_e2e_test_command_remains_available_for_reenabling(ci_workflow):
    steps = ci_workflow["jobs"]["e2e"]["steps"].data
    test_steps = [step for step in steps if step.get("name") == "Run E2E tests"]
    assert len(test_steps) == 1
    assert test_steps[0]["working-directory"] == "e2e/standard"
    assert test_steps[0]["run"] == "pnpm test"
    package = json.loads(
        (ROOT / "e2e/standard/package.json").read_text(encoding="utf-8")
    )
    assert package["scripts"]["test"] == "playwright test"


@pytest.mark.parametrize("job_id", ["test", "format-check", "frontend-test"])
def test_e2e_suspension_does_not_disable_other_checks(ci_workflow, job_id):
    condition = ci_workflow["jobs"][job_id]["if"].data
    # Keep running checks when change detection fails, even during the suspension.
    assert condition.startswith("${{")
    assert "!cancelled()" in condition
    assert "needs.changes.result != 'success'" in condition
