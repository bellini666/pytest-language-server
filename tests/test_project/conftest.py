import pytest
from typing import Generator, Iterator


@pytest.fixture
def sample_fixture() -> int:
    """A sample fixture that returns a value."""
    return 42


@pytest.fixture
def another_fixture() -> str:
    """Another fixture."""
    return "hello world"


@pytest.fixture
def cli_runner() -> str:
    """Parent fixture defined in root conftest.py"""
    return "parent_cli_runner"


@pytest.fixture
def database() -> str:
    """Database fixture that will be overridden in subdir"""
    return "parent_database"


@pytest.fixture
def shared_resource() -> dict[str, str]:
    """Shared resource used by multiple tests"""
    return {"status": "ready"}


@pytest.fixture
def generator_fixture() -> Generator[str, None, None]:
    """A fixture that yields a value using Generator type"""
    yield "generated_value"


@pytest.fixture
def iterator_fixture() -> Iterator[int]:
    """A fixture that yields values using Iterator type"""
    yield 123
