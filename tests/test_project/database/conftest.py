import pytest


@pytest.fixture
def db_connection():
    """Fixture providing a database connection."""
    return {"connected": True, "db": "test_database"}


@pytest.fixture
def db_cursor(db_connection):
    """Fixture that depends on db_connection."""
    return {"cursor": "test_cursor", "connection": db_connection}


@pytest.fixture
def transaction(db_cursor):
    """Fixture that depends on db_cursor (3-level chain)."""
    return {"transaction_id": "txn_123", "cursor": db_cursor}
