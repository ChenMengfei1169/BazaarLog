// Thin fetch wrapper that injects the session token and normalizes API errors
// into a consistent Error shape. The session lives in memory only (no
// localStorage), so a browser refresh logs the user out.
//
// The class password is deliberately NOT kept here. Every authenticated request
// carries the server-issued session token, so retaining the plaintext password
// past login would only widen the blast radius of an XSS or a memory dump.
export interface Session {
  classId: number;
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

// Builds a Headers object with the session token. The backend only accepts
// server-issued session tokens (the legacy password-header path is disabled by
// default), so the operator name recorded in the audit log always comes from
// the login, never from a client-declared header.
function buildAuthHeaders(base?: HeadersInit): Headers {
  const headers = new Headers(base);
  if (session?.token) {
    headers.set('X-Session-Token', session.token);
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
