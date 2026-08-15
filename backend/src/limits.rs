// Centralized input-size limits and small validation helpers so every handler
// applies the same bounds and rejects oversized or malformed payloads before
// they reach the database.
use crate::error::{AppError, AppResult};

pub const MIN_PASSWORD_LENGTH: usize = 8;
pub const MAX_PASSWORD_LENGTH: usize = 128;
pub const MAX_CLASS_NAME_LENGTH: usize = 100;
pub const MAX_SEMESTER_NAME_LENGTH: usize = 100;
pub const MAX_OPERATOR_LENGTH: usize = 64;
pub const MAX_TEXT_FIELD_LENGTH: usize = 200;
pub const MAX_QUERY_FILTER_LENGTH: usize = 100;
// Upper bound on a single transaction amount (100,000,000.00 CNY in cents).
// Prevents absurd values from overflowing the SQL SUM aggregation or the
// i64 arithmetic in the report summary.
pub const MAX_AMOUNT_CENTS: i64 = 10_000_000_000;

/// Rejects an optional text field that exceeds the shared size limit.
pub fn check_optional_text(value: &Option<String>, field_name: &str) -> AppResult<()> {
    if let Some(text) = value {
        if text.chars().count() > MAX_TEXT_FIELD_LENGTH {
            return Err(AppError::BadRequest(format!(
                "{field_name} may not exceed {MAX_TEXT_FIELD_LENGTH} characters"
            )));
        }
    }
    Ok(())
}

/// Rejects a required text field that is empty or exceeds the given bound.
pub fn check_required_text(value: &str, field_name: &str, max_length: usize) -> AppResult<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(format!("{field_name} is required")));
    }
    if trimmed.chars().count() > max_length {
        return Err(AppError::BadRequest(format!(
            "{field_name} may not exceed {max_length} characters"
        )));
    }
    Ok(())
}

/// Rejects a timestamp that is not a valid RFC3339 UTC string.
pub fn check_rfc3339_timestamp(value: &str, field_name: &str) -> AppResult<()> {
    if chrono::DateTime::parse_from_rfc3339(value).is_err() {
        return Err(AppError::BadRequest(format!(
            "{field_name} must be an RFC3339 timestamp"
        )));
    }
    Ok(())
}

/// Rejects an optional date field that is not a valid YYYY-MM-DD value. The
/// archive sweep compares the end date lexicographically, so a malformed date
/// would silently break the archival window.
pub fn check_optional_date(value: &Option<String>, field_name: &str) -> AppResult<()> {
    if let Some(date) = value {
        if chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
            return Err(AppError::BadRequest(format!(
                "{field_name} must be a YYYY-MM-DD date"
            )));
        }
    }
    Ok(())
}

/// Rejects an optional filter value that is neither a YYYY-MM-DD date nor an
/// RFC3339 timestamp, so malformed list filters fail loudly instead of
/// silently returning empty results.
pub fn check_optional_query_date(value: &Option<String>, field_name: &str) -> AppResult<()> {
    if let Some(text) = value {
        let is_date = chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok();
        let is_timestamp = chrono::DateTime::parse_from_rfc3339(text).is_ok();
        if !is_date && !is_timestamp {
            return Err(AppError::BadRequest(format!(
                "{field_name} must be a YYYY-MM-DD date or RFC3339 timestamp"
            )));
        }
    }
    Ok(())
}