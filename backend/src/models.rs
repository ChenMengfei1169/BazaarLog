// Request and response models. Money is always represented as integer cents
// end-to-end to avoid floating point rounding. Timestamps travel as RFC3339
// UTC strings for portability across the SQLite and PostgreSQL backends.
//
// Every type here is part of the wire contract documented in docs/api.md, so
// field names must stay in step with the frontend types in
// frontend/src/types.ts.
use serde::{Deserialize, Serialize};

/// A class ledger, as returned by the class list and class creation endpoints.
#[derive(Serialize, Debug)]
pub struct Class {
    pub id: i64,
    pub name: String,
    pub created_at: String,
}

/// A semester within a class. Archived semesters stay queryable for reporting
/// but reject further mutations.
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

/// One ledger entry, as stored and as returned by the transaction endpoints.
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

/// One operation log entry, including the chain hashes needed to verify it.
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
    // Audit chain hashes. entry_hash commits this row's content to the chain;
    // prev_hash points at the previous row's entry_hash. Together they make
    // silent tampering detectable via GET /api/classes/:id/audit_logs/chain.
    pub prev_hash: String,
    pub entry_hash: String,
}

/// Body of POST /api/classes.
#[derive(Deserialize, Debug)]
pub struct CreateClass {
    pub name: String,
    pub password: String,
    // Proof-of-work challenge issued by GET /api/classes/challenge. Required
    // so scripted bulk class creation must spend CPU per attempt.
    pub challenge_nonce: Option<String>,
    pub challenge_solution: Option<String>,
}

/// Body of POST /api/classes/:id/auth.
#[derive(Deserialize, Debug)]
pub struct AuthClass {
    pub password: String,
    // Operator name typed at login time; captured server-side into the
    // session so the audit log can attribute operations authoritatively.
    pub operator: Option<String>,
}

/// Body of POST /api/classes/:id/semesters.
#[derive(Deserialize, Debug)]
pub struct CreateSemester {
    pub name: String,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

/// Body of POST /api/semesters/:id/transactions and PUT /api/transactions/:id.
/// The update path replaces every mutable field, so both verbs share one shape.
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

/// Query string of GET /api/semesters/:id/transactions. Every filter is
/// optional, and an absent filter is simply not applied.
#[derive(Deserialize, Debug)]
pub struct TransactionQuery {
    pub kind: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub search: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

/// One page of ledger entries plus the total row count for the whole filter,
/// so the client can render pagination without a second request.
#[derive(Serialize, Debug)]
pub struct TransactionList {
    pub data: Vec<Transaction>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

/// Headline figures for a semester. The dashboard and the Excel export share
/// this shape, and it is the payload that gets cached in memory.
#[derive(Serialize, Deserialize, Debug)]
pub struct ReportSummary {
    pub total_income_cents: i64,
    pub total_expense_cents: i64,
    pub balance_cents: i64,
    pub income_count: i64,
    pub expense_count: i64,
}

/// One row of the top-items ranking.
#[derive(Serialize, Deserialize, Debug)]
pub struct ItemRanking {
    pub item: String,
    pub quantity: i64,
    pub total_cents: i64,
}

/// The aggregated dashboard payload: headline figures plus the item ranking.
#[derive(Serialize, Deserialize, Debug)]
pub struct Report {
    pub summary: ReportSummary,
    pub item_ranking: Vec<ItemRanking>,
}
