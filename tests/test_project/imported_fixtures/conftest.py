"""Conftest that imports fixtures from fixture_module."""

from .fixture_module import *  # noqa: F403

import pytest


@pytest.fixture
def local_fixture():
    """A fixture defined directly in this conftest."""
    return "local_value"
