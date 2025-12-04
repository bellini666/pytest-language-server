# This file defines a fixture that should NOT be available to sibling test files
import pytest


@pytest.fixture
def isolated_fixture():
    """A fixture only available in this file."""
    return "isolated"


def test_uses_own_fixture(isolated_fixture):
    """Test that uses the fixture defined in the same file."""
    assert isolated_fixture == "isolated"
