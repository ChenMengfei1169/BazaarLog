// Compact semester switcher used in the top bar. Lists semesters for the
// active class and lets the user create a new one or archive the current
// selection.
import { useEffect, useState } from 'react';

import { api } from '../api';
import type { Semester } from '../types';

export function SemesterSwitcher({
  classId,
  currentId,
  onSelect,
}: {
  classId: number;
  currentId: number | null;
  onSelect: (semester: Semester) => void;
}): JSX.Element {
  const [semesters, setSemesters] = useState<Semester[]>([]);
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState('');
  const [startDate, setStartDate] = useState('');
  const [endDate, setEndDate] = useState('');
  const [error, setError] = useState('');

  function refresh(): void {
    api
      .get<Semester[]>(`/api/classes/${classId}/semesters`)
      .then(setSemesters)
      .catch((e: Error) => setError(e.message));
  }

  useEffect(refresh, [classId]);

  function create(e: React.FormEvent): void {
    e.preventDefault();
    if (!name.trim()) {
      setError('请填写学期名称。');
      return;
    }
    api
      .post<Semester>(`/api/classes/${classId}/semesters`, {
        name,
        start_date: startDate || null,
        end_date: endDate || null,
      })
      .then((s) => {
        setCreating(false);
        setName('');
        setStartDate('');
        setEndDate('');
        setError('');
        refresh();
        onSelect(s);
      })
      .catch((e: Error) => setError(e.message));
  }

  function archive(): void {
    if (currentId == null) return;
    if (!window.confirm('确定归档该学期吗？归档后仍可见，但将变为只读。')) return;
    api
      .post(`/api/semesters/${currentId}/archive`)
      .then(refresh)
      .catch((e: Error) => setError(e.message));
  }

  return (
    <div className="card space-y-3">
      <div className="flex items-center gap-2">
        <select
          className="input"
          value={currentId ?? ''}
          onChange={(e) => {
            const next = semesters.find((s) => s.id === Number(e.target.value));
            if (next) onSelect(next);
          }}
        >
          <option value="" disabled>
            请选择学期
          </option>
          {semesters.map((s) => (
            <option key={s.id} value={s.id}>
              {s.name}
              {s.archived ? '（已归档）' : ''}
            </option>
          ))}
        </select>
        <button type="button" className="btn" onClick={() => setCreating((v) => !v)}>
          {creating ? '取消' : '新建'}
        </button>
        {currentId != null && (
          <button type="button" className="btn" onClick={archive}>
            归档
          </button>
        )}
      </div>

      {creating && (
        <form onSubmit={create} className="space-y-2 border-t border-ink-500 pt-3">
          <div>
            <label className="label" htmlFor="sem-name">
              学期名称
            </label>
            <input
              id="sem-name"
              className="input"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="如：2026 春季"
            />
          </div>
          <div className="grid grid-cols-2 gap-2">
            <div>
              <label className="label" htmlFor="sem-start">
                开始日期
              </label>
              <input
                id="sem-start"
                type="date"
                className="input"
                value={startDate}
                onChange={(e) => setStartDate(e.target.value)}
              />
            </div>
            <div>
              <label className="label" htmlFor="sem-end">
                结束日期
              </label>
              <input
                id="sem-end"
                type="date"
                className="input"
                value={endDate}
                onChange={(e) => setEndDate(e.target.value)}
              />
            </div>
          </div>
          <button type="submit" className="btn-primary">
            创建学期
          </button>
        </form>
      )}

      {error && <p className="text-negative text-sm">{error}</p>}
    </div>
  );
}
