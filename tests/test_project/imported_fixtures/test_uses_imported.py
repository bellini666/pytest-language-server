"""Test file that uses fixtures imported via conftest."""


def test_uses_imported_fixture(imported_fixture, local_fixture):
    """Test that uses a fixture imported via star import."""
    assert imported_fixture == "imported_value"
    assert local_fixture == "local_value"


def test_uses_another_imported(another_imported_fixture):
    """Test that uses another imported fixture."""
    assert another_imported_fixture == 42


def test_uses_deep_nested_fixture(deep_nested_fixture):
    """Test that uses a fixture from a deeply nested module via transitive import."""
    assert deep_nested_fixture == "deep_value"


def test_uses_fixture_with_deep_dependency(another_deep_fixture):
    """Test that uses a fixture with a dependency from the same nested module."""
    assert another_deep_fixture == "wrapped_deep_value"
