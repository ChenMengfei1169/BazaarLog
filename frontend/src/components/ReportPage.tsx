// Reporting dashboard: headline figures, item ranking table, two Chart.js
// charts (income/expense split pie and item ranking bar), and Excel export.
// Charts are lazily created and destroyed on unmount to avoid canvas leaks.
import { useEffect, useRef, useState } from 'react';
import {
  Chart,
  BarController,
  BarElement,
  PieController,
  ArcElement,
  CategoryScale,
  LinearScale,
  Tooltip,
  Legend,
  Title,
} from 'chart.js';

import { api } from '../api';
import { formatCents, formatCount } from '../format';
import type { Report, Semester } from '../types';

// Register only the controllers used by this application to keep the bundle
// small and friendly to Win7-era browsers.
Chart.register(
  BarController,
  BarElement,
  PieController,
  ArcElement,
  CategoryScale,
  LinearScale,
  Tooltip,
  Legend,
  Title,
);

const CHART_COLORS = ['#e5e5e5', '#a3a3a3', '#737373', '#3d3d3d', '#1f1f1f'];

export function ReportPage({ semester }: { semester: Semester }): JSX.Element {
  const [report, setReport] = useState<Report | null>(null);
  const [error, setError] = useState('');
  const [exporting, setExporting] = useState(false);
  const pieRef = useRef<HTMLCanvasElement | null>(null);
  const barRef = useRef<HTMLCanvasElement | null>(null);
  const pieChart = useRef<Chart | null>(null);
  const barChart = useRef<Chart | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .get<Report>(`/api/semesters/${semester.id}/report`)
      .then((r) => {
        if (!cancelled) setReport(r);
      })
      .catch((e: Error) => setError(e.message));
    return () => {
      cancelled = true;
    };
  }, [semester.id]);

  useEffect(() => {
    if (!report) return;
    // Income vs expense pie chart.
    if (pieRef.current) {
      pieChart.current?.destroy();
      pieChart.current = new Chart(pieRef.current, {
        type: 'pie',
        data: {
          labels: ['收入', '支出'],
          datasets: [
            {
              data: [report.summary.total_income_cents, report.summary.total_expense_cents],
              backgroundColor: CHART_COLORS,
            },
          ],
        },
        options: {
          plugins: { legend: { position: 'bottom', labels: { color: '#e5e5e5' } } },
        },
      });
    }
    // Item ranking bar chart.
    if (barRef.current) {
      barChart.current?.destroy();
      barChart.current = new Chart(barRef.current, {
        type: 'bar',
        data: {
          labels: report.item_ranking.map((r) => r.item).slice(0, 10),
          datasets: [
            {
              label: '销售数量',
              data: report.item_ranking.map((r) => r.quantity).slice(0, 10),
              backgroundColor: '#a3a3a3',
            },
          ],
        },
        options: {
          plugins: { legend: { labels: { color: '#e5e5e5' } } },
          scales: {
            x: { ticks: { color: '#a3a3a3' }, grid: { color: '#1f1f1f' } },
            y: { ticks: { color: '#a3a3a3' }, grid: { color: '#1f1f1f' }, beginAtZero: true },
          },
        },
      });
    }
    return () => {
      pieChart.current?.destroy();
      barChart.current?.destroy();
    };
  }, [report]);

  function exportExcel(): void {
    setExporting(true);
    api
      .download(`/api/semesters/${semester.id}/export.xlsx`)
      .then((blob) => {
        const url = URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        a.download = `bazaarlog_semester_${semester.id}.xlsx`;
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        URL.revokeObjectURL(url);
      })
      .catch((e: Error) => setError(e.message))
      .finally(() => setExporting(false));
  }

  if (error) return <p className="text-negative text-sm">{error}</p>;
  if (!report) return <p className="text-ink-300 text-sm">报表加载中...</p>;

  const { summary } = report;

  return (
    <div className="space-y-4">
      <div className="grid sm:grid-cols-4 gap-3">
        <StatCard label="总收入" value={formatCents(summary.total_income_cents)} sub={`${formatCount(summary.income_count)} 笔`} />
        <StatCard label="总支出" value={formatCents(summary.total_expense_cents)} sub={`${formatCount(summary.expense_count)} 笔`} />
        <StatCard
          label="结余"
          value={formatCents(summary.balance_cents)}
          accent={summary.balance_cents >= 0 ? 'positive' : 'negative'}
        />
        <StatCard label="流水总数" value={formatCount(summary.income_count + summary.expense_count)} />
      </div>

      <div className="grid lg:grid-cols-2 gap-4">
        <div className="card">
          <h3 className="text-lg font-semibold mb-2">收入与支出占比</h3>
          <canvas ref={pieRef} />
        </div>
        <div className="card">
          <h3 className="text-lg font-semibold mb-2">热销物品排行</h3>
          <canvas ref={barRef} />
        </div>
      </div>

      <div className="card">
        <div className="flex items-center justify-between mb-2">
          <h3 className="text-lg font-semibold">物品排行</h3>
          <button className="btn-primary" onClick={exportExcel} disabled={exporting}>
            {exporting ? '导出中...' : '导出 Excel'}
          </button>
        </div>
        <table className="w-full text-sm">
          <thead>
            <tr className="text-left text-ink-300 border-b border-ink-500">
              <th className="py-2 pr-3">名次</th>
              <th className="py-2 pr-3">物品</th>
              <th className="py-2 pr-3 text-right">数量</th>
              <th className="py-2 text-right">金额（元）</th>
            </tr>
          </thead>
          <tbody>
            {report.item_ranking.length === 0 && (
              <tr>
                <td colSpan={4} className="py-4 text-center text-ink-300">
                  暂无收入物品记录。
                </td>
              </tr>
            )}
            {report.item_ranking.map((r, i) => (
              <tr key={r.item} className="border-b border-ink-600 last:border-0">
                <td className="py-2 pr-3">{i + 1}</td>
                <td className="py-2 pr-3">{r.item}</td>
                <td className="py-2 pr-3 text-right font-mono">{r.quantity}</td>
                <td className="py-2 text-right font-mono">{formatCents(r.total_cents)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function StatCard({
  label,
  value,
  sub,
  accent,
}: {
  label: string;
  value: string;
  sub?: string;
  accent?: 'positive' | 'negative';
}): JSX.Element {
  const tone = accent === 'negative' ? 'text-negative' : accent === 'positive' ? 'text-positive' : 'text-ink-100';
  return (
    <div className="card">
      <p className="text-xs uppercase tracking-wider text-ink-300">{label}</p>
      <p className={`text-3xl font-bold mt-1 ${tone}`}>{value}</p>
      {sub && <p className="text-xs text-ink-300 mt-1">{sub}</p>}
    </div>
  );
}
