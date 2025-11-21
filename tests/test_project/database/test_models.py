def test_db_connection(db_connection):
    """Test database connection fixture."""
    assert db_connection["connected"] is True


def test_db_cursor(db_cursor):
    """Test database cursor fixture."""
    assert "cursor" in db_cursor
    assert db_cursor["connection"]["connected"] is True


def test_transaction(transaction):
    """Test transaction fixture (3-level dependency chain)."""
    assert transaction["transaction_id"] == "txn_123"
    assert transaction["cursor"]["connection"]["connected"] is True
