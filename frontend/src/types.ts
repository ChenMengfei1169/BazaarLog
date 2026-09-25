// Shared domain types mirroring the backend models. Money values are integer
// cents so the UI never does floating point arithmetic on currency.
export interface BazaarClass {
  id: number;
  name: string;
  created_at: string;
}

export interface Semester {
  id: number;
  class_id: number;
  name: string;
  archived: boolean;
  start_date: string | null;
  end_date: string | null;
  created_at: string;
}

export interface Transaction {
  id: number;
  semester_id: number;
  class_id: number;
  kind: 'income' | 'expense';
  amount_cents: number;
  source: string | null;
  purpose: string | null;
  item: string | null;
  operator: string;
  occurred_at: string;
  created_at: string;
}

export interface TransactionList {
  data: Transaction[];
  total: number;
  page: number;
  page_size: number;
}

export interface ReportSummary {
  total_income_cents: number;
  total_expense_cents: number;
  balance_cents: number;
  income_count: number;
  expense_count: number;
}

export interface ItemRanking {
  item: string;
  quantity: number;
  total_cents: number;
}

export interface Report {
  summary: ReportSummary;
  item_ranking: ItemRanking[];
}

export interface AuditLog {
  id: number;
  transaction_id: number | null;
  class_id: number | null;
  action: 'create' | 'update' | 'delete';
  operator: string;
  payload_before: string | null;
  payload_after: string | null;
  occurred_at: string;
  // Audit chain hashes; entry_hash commits this row's content, prev_hash
  // links to the previous row. A missing or mismatching chain flags tampering.
  prev_hash: string;
  entry_hash: string;
}

// Result of verifying the tamper-evident audit hash chain.
export interface AuditChainReport {
  verified: boolean;
  first_broken_id: number | null;
  // True when the database's newest entry_hash disagrees with the external
  // seal file, i.e. the tail of the audit log was deleted or rolled back.
  // `verified` is also false in that case; this field distinguishes a broken
  // link in the middle of the chain from a truncated tail.
  truncated: boolean;
}

// Proof-of-work challenge issued by GET /api/classes/challenge.
export interface CreateChallenge {
  nonce: string;
  difficulty: number;
}

export interface TransactionQuery {
  kind?: string;
  from?: string;
  to?: string;
  search?: string;
  page?: number;
  page_size?: number;
}
