"""Deep nested fixtures for testing transitive imports."""

import pytest


@pytest.fixture
def deep_nested_fixture():
    """A fixture in a deeply nested module."""
    return "deep_value"


@pytest.fixture
def another_deep_fixture(deep_nested_fixture):
    """A fixture that depends on another deep fixture."""
    return f"wrapped_{deep_nested_fixture}"
