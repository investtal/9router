import { createRootRoute, Link, Outlet, useNavigate, useRouterState } from '@tanstack/react-router';
import { useEffect, useState } from 'react';
import { fetchMe, logout, type DashboardUser } from '../lib/api';

export const Route = createRootRoute({
  component: RootLayout,
});

type NavItem = { to: string; label: string; adminOnly?: boolean; hideForViewer?: boolean };

const NAV: NavItem[] = [
  { to: '/', label: 'Overview' },
  { to: '/models', label: 'Models' },
  { to: '/usage', label: 'Usage' },
  { to: '/members', label: 'Members' },
  { to: '/providers', label: 'Providers' },
  { to: '/logs', label: 'Logs' },
  { to: '/admin/keys', label: 'Admin keys', adminOnly: true, hideForViewer: true },
  { to: '/admin/export', label: 'Export', adminOnly: true },
];

function RootLayout() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const isLogin = pathname === '/login';
  const navigate = useNavigate();
  const [user, setUser] = useState<DashboardUser | null | undefined>(undefined);

  useEffect(() => {
    if (isLogin) {
      setUser(null);
      return;
    }
    let cancelled = false;
    fetchMe()
      .then((u) => {
        if (!cancelled) setUser(u);
      })
      .catch(() => {
        if (!cancelled) {
          setUser(null);
          navigate({ to: '/login' });
        }
      });
    return () => {
      cancelled = true;
    };
  }, [isLogin, navigate, pathname]);

  async function onLogout() {
    try {
      await logout();
    } catch {
      // ignore
    }
    setUser(null);
    navigate({ to: '/login' });
  }

  if (isLogin) {
    return <Outlet />;
  }

  if (user === undefined) {
    return (
      <div className="main">
        <p className="muted">Loading session…</p>
      </div>
    );
  }

  if (user === null) {
    return null;
  }

  const isAdmin = user.role === 'admin';
  const visibleNav = NAV.filter((item) => {
    if (item.adminOnly && !isAdmin) return false;
    if (item.hideForViewer && user.role === 'viewer') return false;
    return true;
  });

  return (
    <div className="layout">
      <nav className="nav">
        <div className="brand">TAGW</div>
        {visibleNav.map((item) => (
          <Link
            key={item.to}
            to={item.to}
            className={pathname === item.to ? 'active' : undefined}
          >
            {item.label}
          </Link>
        ))}
        <div className="footer">
          <div>
            {user.username}{' '}
            <span className="badge">{user.role}</span>
          </div>
          <button type="button" className="secondary" style={{ marginTop: 8 }} onClick={onLogout}>
            Log out
          </button>
        </div>
      </nav>
      <main className="main">
        <Outlet />
      </main>
    </div>
  );
}
