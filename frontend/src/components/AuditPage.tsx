// Read-only audit log view. Displays the most recent create/update/delete
// operations for the active class, with JSON payload snapshots for review.
import { useEffect, useState } from 'react';

import { api } from '../api';
import { formatDateTime } from '../format';
import type { AuditChainReport, AuditLog } from '../types';

export function AuditPage({ classId }: { classId: number }): JSX.Element {
  const [logs, setLogs] = useState<AuditLog[]>([]);
  const [chainVerified, setChainVerified] = useState(true);
  const [truncated, setTruncated] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    api
      .get<AuditLog[]>(`/api/classes/${classId}/audit_logs`)
      .then(setLogs)
      .catch((e: Error) => setError(e.message));
    // Verify the tamper-evident hash chain; a broken chain means some audit
    // record was deleted or edited outside the application.
    api
      .get<AuditChainReport>(`/api/classes/${classId}/audit_logs/chain`)
      .then((report) => {
        setChainVerified(report.verified);
        setTruncated(report.truncated);
      })
      .catch(() => setChainVerified(false));
  }, [classId]);

  if (error) return <p className="text-negative text-sm">{error}</p>;
  if (!chainVerified) {
    return (
      <div className="card border-negative">
        <p className="text-negative text-sm font-bold">审计链完整性校验失败</p>
        <p className="text-negative text-sm mt-1">
          {truncated
            ? '外部封印文件与数据库中的最新哈希不一致：操作日志的尾部可能被删除或回滚，或封印文件被移除。请立即检查数据库与封印文件（.audit.seal）的访问权限，并核对最近一次的审计备份。'
            : '操作日志哈希链出现断裂：可能有记录被外部修改或删除。请立即检查数据库文件的访问权限，并核对最近一次的审计备份。'}
        </p>
      </div>
    );
  }
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
