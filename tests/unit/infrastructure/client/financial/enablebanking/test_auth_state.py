import pytest
from infrastructure.client.financial.enablebanking.auth_state import (
    InvalidAuthState,
    build_auth_state,
    is_allowed_completion_url,
    parse_auth_state,
)

ENTITY_ID = "0b6c2d0e-3f0a-4d8e-9a54-3f6f1c2a9b10"
COMPLETION_URL = "https://my-host.example/#/entities"


class TestRoundTrip:
    def test_without_completion_url(self):
        state = build_auth_state(ENTITY_ID, None)

        assert parse_auth_state(state) == (ENTITY_ID, None)

    def test_with_completion_url(self):
        state = build_auth_state(ENTITY_ID, COMPLETION_URL)

        assert parse_auth_state(state) == (ENTITY_ID, COMPLETION_URL)

    def test_with_deep_link(self):
        deep_link = "ferrofolio://external/complete"

        state = build_auth_state(ENTITY_ID, deep_link)

        assert parse_auth_state(state) == (ENTITY_ID, deep_link)

    def test_state_contains_only_url_safe_characters(self):
        state = build_auth_state(ENTITY_ID, COMPLETION_URL + "?a=b&c=d")

        assert all(c.isalnum() or c in "-_~" for c in state)


class TestRejection:
    @pytest.mark.parametrize("state", [None, "", ENTITY_ID, f"{ENTITY_ID}~", "a~b~c~d"])
    def test_rejects_missing_or_malformed_state(self, state):
        with pytest.raises(InvalidAuthState):
            parse_auth_state(state)

    def test_rejects_unsigned_state_in_the_previous_format(self):
        state = build_auth_state(ENTITY_ID, COMPLETION_URL)
        unsigned = state.rsplit("~", 1)[0]

        with pytest.raises(InvalidAuthState):
            parse_auth_state(unsigned)

    def test_rejects_replaced_completion_url(self):
        state = build_auth_state(ENTITY_ID, COMPLETION_URL)
        forged = build_auth_state(ENTITY_ID, "https://attacker.example/")
        entity_id, _, signature = state.split("~")
        _, forged_url, _ = forged.split("~")

        with pytest.raises(InvalidAuthState):
            parse_auth_state(f"{entity_id}~{forged_url}~{signature}")

    def test_rejects_replaced_entity_id(self):
        state = build_auth_state(ENTITY_ID, COMPLETION_URL)
        _, encoded_url, signature = state.split("~")

        with pytest.raises(InvalidAuthState):
            parse_auth_state(f"another-entity~{encoded_url}~{signature}")

    @pytest.mark.parametrize("signature", ["é", "sigñature", "🙂"])
    def test_rejects_non_ascii_signature(self, signature):
        with pytest.raises(InvalidAuthState):
            parse_auth_state(f"{ENTITY_ID}~~{signature}")

    def test_rejects_completion_url_with_unsupported_scheme(self):
        state = build_auth_state(ENTITY_ID, "javascript:alert(1)")

        with pytest.raises(InvalidAuthState):
            parse_auth_state(state)


class TestAllowedCompletionUrl:
    @pytest.mark.parametrize(
        "url",
        [
            "https://my-host.example/#/entities",
            "http://localhost:8080/",
            "ferrofolio://external/complete",
        ],
    )
    def test_allows(self, url):
        assert is_allowed_completion_url(url)

    @pytest.mark.parametrize(
        "url",
        [
            "javascript:alert(1)",
            "data:text/html,x",
            "/relative",
            "https://",
            "other://x",
        ],
    )
    def test_rejects(self, url):
        assert not is_allowed_completion_url(url)
