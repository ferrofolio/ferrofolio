"""
Verify that all AI client identity strings use the Ferrofolio brand.
"""

from infrastructure.client.ai.claude_agent.claude_agent_client import (
    APPLICATION_NAME as CLAUDE_APPLICATION_NAME,
)
from infrastructure.client.ai.github_copilot.copilot_runtime import (
    APPLICATION_NAME as COPILOT_APPLICATION_NAME,
)


def test_claude_agent_client_app_name_is_ferrofolio():
    """The Claude agent client should identify itself as Ferrofolio."""
    assert CLAUDE_APPLICATION_NAME == "Ferrofolio"


def test_copilot_runtime_app_name_is_ferrofolio():
    """The GitHub Copilot runtime should identify itself as Ferrofolio."""
    assert COPILOT_APPLICATION_NAME == "Ferrofolio"
