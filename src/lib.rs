pub mod fixtures;

pub use fixtures::{
    CompletionContext, FixtureDatabase, FixtureDefinition, FixtureUsage, ParamInsertionInfo,
    UndeclaredFixture,
};

// Expose decorators module for testing
#[cfg(test)]
pub use fixtures::decorators;
