#[error]
pub enum MathError {
    InvalidInput,
}

#[export]
pub async fn clear_all(should_fail: bool) -> Result<(), MathError> {
    if should_fail {
        Err(MathError::InvalidInput)
    } else {
        Ok(())
    }
}
