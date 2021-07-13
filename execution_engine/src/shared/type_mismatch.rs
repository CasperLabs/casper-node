use serde::{Deserialize, Serialize};

#[derive(PartialEq, Eq, Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("Type mismatch. Expected {expected} but found {found}.")]
pub struct TypeMismatch {
    pub expected: String,
    pub found: String,
}

impl TypeMismatch {
    pub fn new(expected: String, found: String) -> TypeMismatch {
        TypeMismatch { expected, found }
    }
}
