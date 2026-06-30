// Request and response models. Money is always represented as integer cents
// end-to-end to avoid floating point rounding. Timestamps travel as RFC3339
// UTC strings for portability across the SQLite and PostgreSQL backends.
use serde::{Deserialize, Serialize};

#[derive(Serialize, Debug)]
pub struct Class {
    pub id: i64,
    pub name: String,
    pub created_at: String,
}

#[derive(Serialize, Debug)]
pub struct Semester {
    pub id: i64,
    pub class_id: i64,
    pub name: String,
    pub archived: bool,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub created_at: String,
}

#[derive(Serialize, Debug, Clone)]
pub struct Transaction {
    pub id: i64,
    pub semester_id: i64,
    pub class_id: i64,
    pub kind: String,
    pub amount_cents: i64,
    pub source: Option<String>,
    pub purpose: Option<String>,
    pub item: Option<String>,
    pub operator: String,
    pub occurred_at: String,
    pub created_at: String,
}

#[derive(Serialize, Debug)]
pub struct AuditLog {
    pub id: i64,
    pub transaction_id: Option<i64>,
    pub class_id: Option<i64>,
    pub action: String,
    pub operator: String,
    pub payload_before: Option<String>,
    pub payload_after: Option<String>,
    pub occurred_at: String,
}

#[derive(Deserialize, Debug)]
pub struct CreateClass {
    pub name: String,
    pub password: String,
}

#[derive(Deserialize, Debug)]
pub struct AuthClass {
    pub password: String,
}

#[derive(Deserialize, Debug)]
pub struct CreateSemester {
    pub name: String,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct CreateTransaction {
    pub kind: String,
    pub amount_cents: i64,
    pub source: Option<String>,
    pub purpose: Option<String>,
    pub item: Option<String>,
    pub operator: String,
    pub occurred_at: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct TransactionQuery {
    pub kind: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub search: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[derive(Serialize, Debug)]
pub struct TransactionList {
    pub data: Vec<Transaction>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ReportSummary {
    pub total_income_cents: i64,
    pub total_expense_cents: i64,
    pub balance_cents: i64,
    pub income_count: i64,
    pub expense_count: i64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ItemRanking {
    pub item: String,
    pub quantity: i64,
    pub total_cents: i64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Report {
    pub summary: ReportSummary,
    pub item_ranking: Vec<ItemRanking>,
}
