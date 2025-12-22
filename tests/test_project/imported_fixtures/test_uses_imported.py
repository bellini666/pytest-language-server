"""Test file that uses fixtures imported via conftest."""


def test_uses_imported_fixture(imported_fixture, local_fixture):
    """Test that uses a fixture imported via star import."""
    assert imported_fixture == "imported_value"
    assert local_fixture == "local_value"


def test_uses_another_imported(another_imported_fixture):
    """Test that uses another imported fixture."""
    assert another_imported_fixture == 42
