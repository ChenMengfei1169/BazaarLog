// Read-only audit log view. Displays the most recent create/update/delete
// operations for the active class, with JSON payload snapshots for review.
import { useEffect, useState } from 'react';

import { api } from '../api';
import { formatDateTime } from '../format';
import type { AuditLog } from '../types';

export function AuditPage({ classId }: { classId: number }): JSX.Element {
  const [logs, setLogs] = useState<AuditLog[]>([]);
  const [error, setError] = useState('');

  useEffect(() => {
    api
      .get<AuditLog[]>(`/api/classes/${classId}/audit_logs`)
      .then(setLogs)
      .catch((e: Error) => setError(e.message));
  }, [classId]);

  if (error) return <p className="text-negative text-sm">{error}</p>;
  if (logs.length === 0) return <p className="text-ink-300 text-sm">暂无操作记录。</p>;

  return (
    <div className="card overflow-x-auto">
      <table className="w-full text-sm">
        <thead>
          <tr className="text-left text-ink-300 border-b border-ink-500">
            <th className="py-2 pr-3">时间</th>
            <th className="py-2 pr-3">动作</th>
            <th className="py-2 pr-3">流水号</th>
            <th className="py-2 pr-3">操作人</th>
            <th className="py-2">变更明细</th>
          </tr>
        </thead>
        <tbody>
          {logs.map((log) => (
            <tr key={log.id} className="border-b border-ink-600 last:border-0 align-top">
              <td className="py-2 pr-3 whitespace-nowrap">{formatDateTime(log.occurred_at)}</td>
              <td className="py-2 pr-3">
                <span
                  className={
                    log.action === 'delete'
                      ? 'text-negative'
                      : log.action === 'create'
                        ? 'text-positive'
                        : 'text-ink-200'
                  }
                >
                  {log.action === 'create'
                    ? '新建'
                    : log.action === 'update'
                      ? '修改'
                      : log.action === 'delete'
                        ? '删除'
                        : log.action}
                </span>
              </td>
              <td className="py-2 pr-3 font-mono">{log.transaction_id ?? '-'}</td>
              <td className="py-2 pr-3">{log.operator}</td>
              <td className="py-2 text-xs font-mono text-ink-200">
                {log.payload_before && (
                  <div>
                    <span className="text-ink-300">变更前：</span> {log.payload_before}
                  </div>
                )}
                {log.payload_after && (
                  <div>
                    <span className="text-ink-300">变更后：</span> {log.payload_after}
                  </div>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
