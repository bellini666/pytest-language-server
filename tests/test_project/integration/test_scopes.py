import pytest


@pytest.fixture(scope="session")
def session_fixture():
    """Session-scoped fixture."""
    return "session_data"


@pytest.fixture(scope="module")
def module_fixture():
    """Module-scoped fixture."""
    return "module_data"


def test_with_session_scope(session_fixture):
    """Test using session-scoped fixture."""
    assert session_fixture == "session_data"


def test_with_module_scope(module_fixture):
    """Test using module-scoped fixture."""
    assert module_fixture == "module_data"


@pytest.mark.parametrize("value", [1, 2, 3])
def test_parametrized(value):
    """Parametrized test."""
    assert value > 0
