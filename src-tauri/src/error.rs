use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl AppError {
    pub fn new(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self { code: code.into(), message: message.into(), retryable }
    }
}

impl From<String> for AppError {
    fn from(message: String) -> Self {
        Self::new("operation_failed", message, true)
    }
}

impl From<&str> for AppError {
    fn from(message: &str) -> Self {
        Self::new("operation_failed", message, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_stable_error_contract() {
        let value = serde_json::to_value(AppError::new("profile_locked", "Close the browser", true)).unwrap();
        assert_eq!(value["code"], "profile_locked");
        assert_eq!(value["retryable"], true);
    }
}
