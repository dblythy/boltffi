#[error]
pub enum MathError {
    InvalidInput,
}

#[export]
pub async fn checked_double(value: i32) -> Result<i32, MathError> {
    if value < 0 {
        Err(MathError::InvalidInput)
    } else {
        Ok(value * 2)
    }
}
