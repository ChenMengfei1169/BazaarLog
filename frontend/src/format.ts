// Formatting helpers shared across components. Money is rendered as a fixed
// two-decimal CNY string; timestamps are shown as a short local date-time so
// the dashboard stays readable on a projector.

export function formatCents(cents: number): string {
  const sign = cents < 0 ? '-' : '';
  const abs = Math.abs(cents);
  const yuan = Math.floor(abs / 100);
  const fen = abs % 100;
  return `${sign}${yuan}.${fen.toString().padStart(2, '0')}`;
}

export function formatCount(n: number): string {
  return n.toLocaleString('en-US');
}

export function formatDateTime(iso: string): string {
  // The backend sends RFC3339 UTC strings; render as local time.
  // Keeping the format compact helps the dashboard layout.
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  const pad = (n: number) => n.toString().padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

export function formatDate(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  const pad = (n: number) => n.toString().padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}
