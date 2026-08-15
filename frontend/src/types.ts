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
}

export interface TransactionQuery {
  kind?: string;
  from?: string;
  to?: string;
  search?: string;
  page?: number;
  page_size?: number;
}
