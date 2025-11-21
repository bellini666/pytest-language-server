import pytest


def test_api_client_fixture(api_client):
    """Test that uses the api_client fixture."""
    assert api_client["type"] == "api_client"
    assert api_client["authenticated"] is True


def test_api_token_fixture(api_token):
    """Test that uses the api_token fixture."""
    assert api_token.startswith("test-token")


def test_multiple_fixtures(api_client, api_token, mock_response):
    """Test that uses multiple fixtures."""
    assert api_client is not None
    assert api_token is not None
    assert mock_response["status"] == 200


@pytest.fixture
def local_fixture():
    """A fixture defined in the test file itself."""
    return "local"


def test_local_fixture(local_fixture):
    """Test using a fixture from the same file."""
    assert local_fixture == "local"
