// Top-level application shell. Handles authentication, holds the active
// semester selection, and switches between the transactions/report/audit
// tabs.
import { useState } from 'react';

import { setSession } from './api';
import type { Semester } from './types';
import { AuditPage } from './components/AuditPage';
import { ClassLogin } from './components/ClassLogin';
import { ReportPage } from './components/ReportPage';
import { SemesterSwitcher } from './components/SemesterSwitcher';
import { TransactionsPage } from './components/TransactionsPage';

type Tab = 'transactions' | 'report' | 'audit';

interface ActiveClass {
  id: number;
  name: string;
  password: string;
  operator: string;
}

export function App(): JSX.Element {
  const [activeClass, setActiveClass] = useState<ActiveClass | null>(null);
  const [semester, setSemester] = useState<Semester | null>(null);
  const [tab, setTab] = useState<Tab>('transactions');

  function handleAuthenticated(classId: number, className: string, password: string): void {
    const operator = activeClass?.operator ?? 'anonymous';
    setSession({ classId, password, operator });
    setActiveClass({ id: classId, name: className, password, operator });
    setSemester(null);
    setTab('transactions');
  }

  function signOut(): void {
    setSession(null);
    setActiveClass(null);
    setSemester(null);
  }

  if (!activeClass) {
    return <ClassLogin onAuthenticated={handleAuthenticated} />;
  }

  return (
    <div className="min-h-screen">
      <header className="border-b border-ink-600 bg-ink-800">
        <div className="max-w-6xl mx-auto px-4 py-3 flex flex-wrap items-center gap-3">
          <h1 className="text-xl font-bold tracking-tight">BazaarLog</h1>
          <span className="text-sm text-ink-300">/ {activeClass.name}</span>
          <div className="ml-auto flex items-center gap-2">
            <span className="text-xs text-ink-300">操作人：{activeClass.operator}</span>
          <button className="btn px-2 py-1 text-xs" onClick={signOut}>
            退出登录
          </button>
          </div>
        </div>
      </header>

      <main className="max-w-6xl mx-auto px-4 py-4 space-y-4">
        <SemesterSwitcher
          classId={activeClass.id}
          currentId={semester?.id ?? null}
          onSelect={setSemester}
        />

        <nav className="flex gap-2">
          {([
            ['transactions', '流水登记'],
            ['report', '汇总汇报'],
            ['audit', '操作日志'],
          ] as [Tab, string][]).map(([t, label]) => (
            <button
              key={t}
              className={`btn ${tab === t ? 'btn-primary' : ''}`}
              onClick={() => setTab(t)}
            >
              {label}
            </button>
          ))}
        </nav>

        {semester ? (
          tab === 'transactions' ? (
            <TransactionsPage semester={semester} />
          ) : tab === 'report' ? (
            <ReportPage semester={semester} />
          ) : (
            <AuditPage classId={activeClass.id} />
          )
        ) : (
          <p className="text-ink-300 text-sm">请先选择或新建一个学期再继续。</p>
        )}
      </main>
    </div>
  );
}
