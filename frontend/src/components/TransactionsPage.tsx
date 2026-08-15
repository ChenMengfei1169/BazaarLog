// Transaction list with filters, search, pagination, and an inline
// create/edit form. Money is entered as decimal yuan and converted to cents
// at the boundary so the rest of the application uses integer semantics.
import { useCallback, useEffect, useState } from 'react';

import { api } from '../api';
import { formatCents, formatDateTime } from '../format';
import type { Semester, Transaction, TransactionList } from '../types';

interface FormState {
  kind: 'income' | 'expense';
  amount: string;
  source: string;
  purpose: string;
  item: string;
  operator: string;
  occurredAt: string;
}

const EMPTY_FORM: FormState = {
  kind: 'income',
  amount: '',
  source: '',
  purpose: '',
  item: '',
  operator: '',
  occurredAt: '',
};

function toFormState(tx: Transaction): FormState {
  return {
    kind: tx.kind,
    amount: (tx.amount_cents / 100).toString(),
    source: tx.source ?? '',
    purpose: tx.purpose ?? '',
    item: tx.item ?? '',
    operator: tx.operator,
    occurredAt: tx.occurred_at.slice(0, 16),
  };
}

export function TransactionsPage({ semester }: { semester: Semester }): JSX.Element {
  const [items, setItems] = useState<Transaction[]>([]);
  const [total, setTotal] = useState(0);
  const [page, setPage] = useState(1);
  const [pageSize] = useState(20);
  const [kind, setKind] = useState('');
  const [from, setFrom] = useState('');
  const [to, setTo] = useState('');
  const [query, setQuery] = useState('');
  const [editing, setEditing] = useState<Transaction | null>(null);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);

  const fetchPage = useCallback(() => {
    const params = new URLSearchParams();
    params.set('page', String(page));
    params.set('page_size', String(pageSize));
    if (kind) params.set('kind', kind);
    if (from) params.set('from', from);
    if (to) params.set('to', to);
    if (query) params.set('search', query);
    api
      .get<TransactionList>(`/api/semesters/${semester.id}/transactions?${params}`)
      .then((r) => {
        setItems(r.data);
        setTotal(r.total);
      })
      .catch((e: Error) => setError(e.message));
  }, [semester.id, page, pageSize, kind, from, to, query]);

  useEffect(() => {
    fetchPage();
  }, [fetchPage]);

  function startCreate(): void {
    setEditing(null);
    setForm({ ...EMPTY_FORM, operator: '', occurredAt: new Date().toISOString().slice(0, 16) });
  }

  function startEdit(tx: Transaction): void {
    setEditing(tx);
    setForm(toFormState(tx));
  }

  function submit(e: React.FormEvent): void {
    e.preventDefault();
    setError('');
    const amountCents = Math.round(Number(form.amount) * 100);
    if (!Number.isFinite(amountCents) || amountCents <= 0) {
      setError('金额必须是正数。');
      return;
    }
    if (!form.operator.trim()) {
      setError('请填写操作人。');
      return;
    }
    setSaving(true);
    const body = {
      kind: form.kind,
      amount_cents: amountCents,
      source: form.source || null,
      purpose: form.purpose || null,
      item: form.item || null,
      operator: form.operator,
      occurred_at: form.occurredAt ? new Date(form.occurredAt).toISOString() : null,
    };
    const req = editing
      ? api.put<Transaction>(`/api/transactions/${editing.id}`, body)
      : api.post<Transaction>(`/api/semesters/${semester.id}/transactions`, body);
    req
      .then(() => {
        setEditing(null);
        setForm(EMPTY_FORM);
        fetchPage();
      })
      .catch((e: Error) => setError(e.message))
      .finally(() => setSaving(false));
  }

  function remove(tx: Transaction): void {
    if (!window.confirm(`确认删除流水 #${tx.id}？此操作会记入操作日志。`)) return;
    api
      .delete(`/api/transactions/${tx.id}`)
      .then(fetchPage)
      .catch((e: Error) => setError(e.message));
  }

  const totalPages = Math.max(1, Math.ceil(total / pageSize));

  return (
    <div className="grid lg:grid-cols-3 gap-4">
      <div className="lg:col-span-2 space-y-4">
        <div className="card grid sm:grid-cols-4 gap-2">
          <select className="input" value={kind} onChange={(e) => setKind(e.target.value)}>
            <option value="">全部类型</option>
            <option value="income">收入</option>
            <option value="expense">支出</option>
          </select>
          <input type="date" className="input" value={from} onChange={(e) => setFrom(e.target.value)} />
          <input type="date" className="input" value={to} onChange={(e) => setTo(e.target.value)} />
          <input
            className="input"
            placeholder="搜索来源/物品/操作人"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>

        {error && <p className="text-negative text-sm">{error}</p>}

        <div className="card overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-ink-300 border-b border-ink-500">
                <th className="py-2 pr-3">时间</th>
                <th className="py-2 pr-3">类型</th>
                <th className="py-2 pr-3 text-right">金额</th>
                <th className="py-2 pr-3">明细</th>
                <th className="py-2 pr-3">操作人</th>
                <th className="py-2"></th>
              </tr>
            </thead>
            <tbody>
              {items.length === 0 && (
                <tr>
                  <td colSpan={6} className="py-6 text-center text-ink-300">
                    暂无流水记录。
                  </td>
                </tr>
              )}
              {items.map((tx) => (
                <tr key={tx.id} className="border-b border-ink-600 last:border-0">
                  <td className="py-2 pr-3 whitespace-nowrap">{formatDateTime(tx.occurred_at)}</td>
                  <td className="py-2 pr-3">
                    <span className={tx.kind === 'income' ? 'text-positive' : 'text-negative'}>
                      {tx.kind === 'income' ? '收入' : '支出'}
                    </span>
                  </td>
                  <td className="py-2 pr-3 text-right font-mono">{formatCents(tx.amount_cents)}</td>
                  <td className="py-2 pr-3">
                    {tx.kind === 'income'
                      ? tx.source ?? tx.item ?? '-'
                      : tx.purpose ?? tx.item ?? '-'}
                  </td>
                  <td className="py-2 pr-3">{tx.operator}</td>
                  <td className="py-2 whitespace-nowrap">
                    <button className="btn px-2 py-1 text-xs mr-1" onClick={() => startEdit(tx)}>
                      编辑
                    </button>
                    <button className="btn-danger px-2 py-1 text-xs" onClick={() => remove(tx)}>
                      删除
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        <div className="flex items-center justify-between text-sm text-ink-300">
          <span>
            第 {page} / {totalPages} 页（共 {total} 条）
          </span>
          <div className="flex gap-2">
            <button className="btn" disabled={page <= 1} onClick={() => setPage((p) => p - 1)}>
              上一页
            </button>
            <button
              className="btn"
              disabled={page >= totalPages}
              onClick={() => setPage((p) => p + 1)}
            >
              下一页
            </button>
          </div>
        </div>
      </div>

      <div className="space-y-4">
        <div className="card space-y-3">
          <div className="flex items-center justify-between">
            <h3 className="text-lg font-semibold">
              {editing ? `编辑 #${editing.id}` : '新增流水'}
            </h3>
            {editing && (
              <button
                className="btn px-2 py-1 text-xs"
                onClick={() => {
                  setEditing(null);
                  setForm(EMPTY_FORM);
                }}
              >
                取消
              </button>
            )}
            {!editing && (
              <button className="btn-primary px-2 py-1 text-xs" onClick={startCreate}>
                新增
              </button>
            )}
          </div>

          <form onSubmit={submit} className="space-y-2">
            <div className="grid grid-cols-2 gap-2">
              <select
                className="input"
                value={form.kind}
                onChange={(e) => setForm({ ...form, kind: e.target.value as 'income' | 'expense' })}
              >
                <option value="income">收入</option>
                <option value="expense">支出</option>
              </select>
              <input
                className="input"
                type="number"
                step="0.01"
                min="0"
                placeholder="金额（元）"
                value={form.amount}
                onChange={(e) => setForm({ ...form, amount: e.target.value })}
              />
            </div>
            {form.kind === 'income' ? (
              <>
                <input
                  className="input"
                  placeholder="来源（付款方）"
                  value={form.source}
                  onChange={(e) => setForm({ ...form, source: e.target.value })}
                />
                <input
                  className="input"
                  placeholder="义卖物品（可选）"
                  value={form.item}
                  onChange={(e) => setForm({ ...form, item: e.target.value })}
                />
              </>
            ) : (
              <>
                <input
                  className="input"
                  placeholder="用途"
                  value={form.purpose}
                  onChange={(e) => setForm({ ...form, purpose: e.target.value })}
                />
                <input
                  className="input"
                  placeholder="购买物品（可选）"
                  value={form.item}
                  onChange={(e) => setForm({ ...form, item: e.target.value })}
                />
              </>
            )}
            <input
              className="input"
              placeholder="操作人"
              value={form.operator}
              onChange={(e) => setForm({ ...form, operator: e.target.value })}
            />
            <input
              className="input"
              type="datetime-local"
              value={form.occurredAt}
              onChange={(e) => setForm({ ...form, occurredAt: e.target.value })}
            />
            <button type="submit" className="btn-primary w-full" disabled={saving}>
              {saving ? '保存中...' : editing ? '更新' : '添加流水'}
            </button>
          </form>
        </div>
      </div>
    </div>
  );
}
