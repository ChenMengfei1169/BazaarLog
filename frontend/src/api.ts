// Thin fetch wrapper that injects per-class authentication headers and
// normalizes API errors into a consistent Error shape. The class id and
// password are held in memory only (no localStorage), so a browser refresh
// logs the user out.
export interface Session {
  classId: number;
  password: string;
  operator: string;
  token: string | null;
}

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
    this.name = 'ApiError';
  }
}

// Abort a request that has not completed within this window so a stalled
// network or backend never leaves the UI waiting forever.
const REQUEST_TIMEOUT_MS = 30_000;

let session: Session | null = null;

export function setSession(next: Session | null): void {
  session = next;
}

export function getSession(): Session | null {
  return session;
}

// Builds a Headers object with the per-class authentication headers. fetch's
// Headers only accepts ISO-8859-1 characters, so non-ASCII values (Chinese
// operator names, passwords) would throw "String contains non ISO-8859-1 code
// point". encodeURIComponent converts them to pure ASCII; the backend
// percent-decodes them back in auth.rs. Shared by request() and download().
function buildAuthHeaders(base?: HeadersInit): Headers {
  const headers = new Headers(base);
  if (session?.token) {
    // Server-issued session token binds the class id and the operator name,
    // so the audit log cannot be spoofed via a client header.
    headers.set('X-Session-Token', session.token);
  } else if (session) {
    // Legacy password-header path for pre-token clients and scripts.
    headers.set('X-Class-Id', String(session.classId));
    headers.set('X-Class-Password', encodeURIComponent(session.password));
    headers.set('X-Operator', encodeURIComponent(session.operator));
  }
  return headers;
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);
  try {
    const headers = buildAuthHeaders(init.headers);
    headers.set('Accept', 'application/json');
    if (init.body && !headers.has('Content-Type')) {
      headers.set('Content-Type', 'application/json');
    }
    const res = await fetch(path, { ...init, headers, signal: controller.signal });
    if (res.status === 204) {
    return undefined as T;
  }
  const text = await res.text();
  let payload: unknown = null;
  if (text) {
    try {
      payload = JSON.parse(text);
    } catch {
      payload = text;
    }
  }
  if (!res.ok) {
    const message =
      payload && typeof payload === 'object' && 'error' in payload
        ? String((payload as { error: string }).error)
        : `request failed (${res.status})`;
    throw new ApiError(res.status, message);
  }
  return payload as T;
  } finally {
    clearTimeout(timeout);
  }
}

export const api = {
  get: <T>(path: string) => request<T>(path),
  post: <T>(path: string, body?: unknown) =>
    request<T>(path, { method: 'POST', body: body ? JSON.stringify(body) : undefined }),
  put: <T>(path: string, body: unknown) =>
    request<T>(path, { method: 'PUT', body: JSON.stringify(body) }),
  delete: <T>(path: string) => request<T>(path, { method: 'DELETE' }),
  // Binary downloads skip JSON parsing and return the raw Blob. Requires
  // authentication headers because the export endpoint is ClassAuth-protected.
  async download(path: string): Promise<Blob> {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);
    try {
      const res = await fetch(path, { headers: buildAuthHeaders(), signal: controller.signal });
      if (!res.ok) throw new ApiError(res.status, 'download failed');
      return res.blob();
    } finally {
      clearTimeout(timeout);
    }
  },
};
