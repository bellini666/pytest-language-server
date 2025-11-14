import pytest


@pytest.fixture
def sample_fixture():
    """A sample fixture that returns a value."""
    return 42


@pytest.fixture
def another_fixture():
    """Another fixture."""
    return "hello world"
