use rustpython_parser as parser;

fn main() {
    let code = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42

async def test_something(my_fixture):
    assert my_fixture == 42
"#;

    match parser::parse(code, parser::Mode::Module, "test.py") {
        Ok(ast) => {
            println!("Parsed successfully!");
            println!("{:#?}", ast);
        }
        Err(e) => {
            println!("Parse error: {:?}", e);
        }
    }
}
