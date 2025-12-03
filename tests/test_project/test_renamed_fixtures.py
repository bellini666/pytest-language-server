"""Test file for renamed fixtures (using name= parameter)."""

import pytest


@pytest.fixture(name="renamed_db")
def internal_database_fixture():
    """A fixture with a different public name."""
    return {"connection": "active"}


@pytest.fixture(name="user")
def create_user_fixture(renamed_db):
    """A fixture that depends on a renamed fixture."""
    return {"name": "test_user", "db": renamed_db}


@pytest.fixture
def normal_fixture():
    """A normal fixture without name= parameter."""
    return "normal"


def test_with_renamed_fixture(renamed_db):
    """Test using the renamed fixture by its public name."""
    assert renamed_db["connection"] == "active"


def test_with_chained_renamed(user):
    """Test using a fixture that depends on a renamed fixture."""
    assert user["name"] == "test_user"


def test_mixed_fixtures(renamed_db, normal_fixture):
    """Test using both renamed and normal fixtures."""
    assert renamed_db is not None
    assert normal_fixture == "normal"
