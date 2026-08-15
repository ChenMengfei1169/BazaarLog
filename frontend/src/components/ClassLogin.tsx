// Class selection and authentication screen. Lists every class so the user
// can pick one, then prompts for that class's password. On success the
// password is held in memory by the api module and the parent component
// switches to the main application view.
import { useEffect, useState } from 'react';

import { api } from '../api';
import type { BazaarClass } from '../types';

interface AuthResponse {
  authenticated: boolean;
  token?: string;
}

export function ClassLogin({
  onAuthenticated,
}: {
  onAuthenticated: (
    classId: number,
    className: string,
    password: string,
    operator: string,
    token: string | null,
  ) => void;
}): JSX.Element {
  const [classes, setClasses] = useState<BazaarClass[]>([]);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [name, setName] = useState('');
  const [password, setPassword] = useState('');
  const [operator, setOperator] = useState('');
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    api
      .get<BazaarClass[]>('/api/classes')
      .then(setClasses)
      .catch((e: Error) => setError(e.message));
  }, []);

  function submit(e: React.FormEvent): void {
    e.preventDefault();
    setError('');
    if (creating) {
      if (!name.trim() || password.length < 8) {
        setError('请填写班级名称，密码至少 8 位。');
        return;
      }
      api
        .post<BazaarClass>('/api/classes', { name, password })
        // A newly created class has no token yet; authenticate immediately so
        // the rest of the session is token-backed like a normal login.
        .then((c) =>
          api
            .post<AuthResponse>(`/api/classes/${c.id}/auth`, { password, operator })
            .then((a) => onAuthenticated(c.id, c.name, password, operator, a.token ?? null)),
        )
        .catch((e: Error) => setError(e.message));
      return;
    }
    if (selectedId == null) {
      setError('请选择一个班级。');
      return;
    }
    const cls = classes.find((c) => c.id === selectedId);
    if (!cls) return;
    api
      .post<AuthResponse>(`/api/classes/${selectedId}/auth`, { password, operator })
      .then((a) => onAuthenticated(cls.id, cls.name, password, operator, a.token ?? null))
      .catch((e: Error) => setError(e.message));
  }

  return (
    <div className="min-h-screen flex items-center justify-center p-4">
      <form onSubmit={submit} className="card w-full max-w-md space-y-4">
        <h1 className="text-3xl font-bold tracking-tight">BazaarLog</h1>
        <p className="text-sm text-ink-300">
          {creating
            ? '新建一个班级义卖账本，开始登记流水。'
            : '请选择班级并输入密码进入系统。'}
        </p>

        {error && <p className="text-negative text-sm">{error}</p>}

        {creating ? (
          <div>
            <label className="label" htmlFor="class-name">
              班级名称
            </label>
            <input
              id="class-name"
              className="input"
              value={name}
              onChange={(e) => setName(e.target.value)}
              autoComplete="off"
            />
          </div>
        ) : (
          <div>
            <label className="label" htmlFor="class-select">
              班级
            </label>
            <select
              id="class-select"
              className="input"
              value={selectedId ?? ''}
              onChange={(e) => setSelectedId(Number(e.target.value))}
            >
              <option value="" disabled>
                请选择班级
              </option>
              {classes.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name}
                </option>
              ))}
            </select>
          </div>
        )}

        <div>
          <label className="label" htmlFor="password">
            密码
          </label>
          <input
            id="password"
            type="password"
            className="input"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            autoComplete="new-password"
          />
        </div>

        <div>
          <label className="label" htmlFor="operator">
            操作人姓名（可选）
          </label>
          <input
            id="operator"
            className="input"
            value={operator}
            onChange={(e) => setOperator(e.target.value)}
            placeholder="anonymous"
          />
        </div>

        <div className="flex gap-2">
          <button type="submit" className="btn-primary flex-1">
            {creating ? '创建班级' : '登录'}
          </button>
          <button
            type="button"
            className="btn"
            onClick={() => {
              setCreating((v) => !v);
              setError('');
            }}
          >
            {creating ? '返回' : '新建班级'}
          </button>
        </div>
      </form>
    </div>
  );
}
