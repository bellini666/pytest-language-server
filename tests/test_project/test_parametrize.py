"""Sample tests exercising the @pytest.mark.parametrize argname forms."""

import pytest


@pytest.mark.parametrize("value", ["a", "b"])
def test_single(value):
    assert value


@pytest.mark.parametrize("left, right", [(1, 2), (3, 4)])
def test_comma(left, right):
    assert left < right


@pytest.mark.parametrize(["first", "second"], [(1, 2)])
def test_list_form(first, second):
    assert first != second


@pytest.mark.parametrize(("alpha", "beta"), [(1, 2)])
def test_tuple_form(alpha, beta):
    assert alpha + beta


@pytest.mark.parametrize(argnames="kw", argvalues=[1, 2])
def test_keyword_form(kw):
    assert kw


@pytest.mark.parametrize("outer", [1])
@pytest.mark.parametrize("inner", [2])
def test_stacked(outer, inner):
    assert outer != inner
