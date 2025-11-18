import pytest


@pytest.fixture
def sample_fixture(sample_fixture: int) -> int:
    """Override parent fixture, adding 100 to the value."""
    return sample_fixture + 100


@pytest.fixture
def local_fixture() -> str:
    """A fixture local to this subdirectory."""
    return "local"


@pytest.fixture
def cli_runner(cli_runner: str) -> str:
    """Override parent cli_runner fixture - self-referencing override"""
    return cli_runner


@pytest.fixture
def database(database: str, shared_resource: dict[str, str]) -> str:
    """Override parent database fixture, depends on parent and another fixture"""
    return f"{database}_modified_{shared_resource['status']}"
