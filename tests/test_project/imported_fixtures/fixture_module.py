"""Module containing fixture definitions that will be imported elsewhere."""

import pytest

# Re-export fixtures from nested module (transitive import test)
from .nested.deep_fixtures import *  # noqa: F403


@pytest.fixture
def imported_fixture():
    """A fixture that will be imported via star import."""
    return "imported_value"


@pytest.fixture
def another_imported_fixture():
    """Another fixture that will be imported via star import."""
    return 42


@pytest.fixture
def explicitly_imported():
    """A fixture that will be imported by name."""
    return "explicit"


def not_a_fixture():
    """Regular function, should not be treated as a fixture."""
    return "not a fixture"
