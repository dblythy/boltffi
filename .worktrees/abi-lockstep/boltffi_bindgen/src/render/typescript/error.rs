use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeScriptLowerError {
    ValueTypeMemberNameCollision {
        owner_name: String,
        generated_name: String,
        existing_source: String,
        colliding_source: String,
    },
    TopLevelFunctionNameCollision {
        generated_name: String,
        existing_function: String,
        colliding_function: String,
    },
}

impl fmt::Display for TypeScriptLowerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ValueTypeMemberNameCollision {
                owner_name,
                generated_name,
                existing_source,
                colliding_source,
            } => write!(
                f,
                "TypeScript value type `{owner_name}` has colliding member name `{generated_name}` from {existing_source} and {colliding_source}"
            ),
            Self::TopLevelFunctionNameCollision {
                generated_name,
                existing_function,
                colliding_function,
            } => write!(
                f,
                "TypeScript top-level function name `{generated_name}` collides between function `{existing_function}` and function `{colliding_function}`"
            ),
        }
    }
}

impl std::error::Error for TypeScriptLowerError {}
