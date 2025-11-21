import pytest


@pytest.fixture
def api_client():
    """Fixture providing an API client for testing."""
    return {"type": "api_client", "authenticated": True}


@pytest.fixture
def api_token():
    """Fixture providing an authentication token."""
    return "test-token-12345"


@pytest.fixture
def mock_response():
    """Fixture providing a mock HTTP response."""
    return {"status": 200, "data": {"result": "success"}}
