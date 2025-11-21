import pytest


@pytest.fixture
def temp_file():
    """Fixture providing a temporary file path."""
    return "/tmp/test_file.txt"


@pytest.fixture
def temp_dir():
    """Fixture providing a temporary directory."""
    return "/tmp/test_dir"


@pytest.fixture(autouse=True)
def auto_cleanup():
    """Autouse fixture that runs for every test."""
    yield
    # Cleanup happens here
