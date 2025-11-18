def test_sample(sample_fixture):
    """Test using sample_fixture."""
    assert sample_fixture == 42


def test_another(another_fixture):
    """Test using another_fixture."""
    assert another_fixture == "hello world"


def test_both(sample_fixture, another_fixture, generator_fixture):
    """Test using both fixtures."""
    assert sample_fixture == 42
    assert another_fixture == "hello world"
