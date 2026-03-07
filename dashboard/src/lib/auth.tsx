import { createContext, useContext, useState, useEffect, useCallback } from "react";
import type { ReactNode } from "react";
import {
  fetchAuthStatus,
  fetchMe,
  loginUser,
  setupAdmin,
  setAuthToken,
  tokenReady,
} from "./api";
import type { AuthUser } from "./api";

interface AuthState {
  ready: boolean;
  needsSetup: boolean;
  user: AuthUser | null;
  login: (username: string, password: string) => Promise<string | null>;
  setup: (username: string, password: string, displayName?: string) => Promise<string | null>;
  logout: () => void;
}

const AuthContext = createContext<AuthState>({
  ready: false,
  needsSetup: false,
  user: null,
  login: async () => "not ready",
  setup: async () => "not ready",
  logout: () => {},
});

export function AuthProvider({ children }: { children: ReactNode }) {
  const [ready, setReady] = useState(false);
  const [needsSetup, setNeedsSetup] = useState(false);
  const [user, setUser] = useState<AuthUser | null>(null);

  useEffect(() => {
    (async () => {
      await tokenReady;

      // Check if users exist
      const status = await fetchAuthStatus();
      if (status.auth_disabled) {
        setUser({ id: 0, username: "admin", display_name: "Admin", is_admin: true });
        setReady(true);
        return;
      }
      if (status.needs_setup) {
        setNeedsSetup(true);
        setReady(true);
        return;
      }

      // Try to get current user (works with JWT or shared token)
      const me = await fetchMe();
      if (me) {
        setUser(me);
      }
      setReady(true);
    })();
  }, []);

  const login = useCallback(async (username: string, password: string): Promise<string | null> => {
    const res = await loginUser(username, password);
    if (res.error) return res.error;
    if (!res.token) return "login failed";
    setAuthToken(res.token);
    setUser(res.user);
    setNeedsSetup(false);
    return null;
  }, []);

  const setup = useCallback(async (username: string, password: string, displayName?: string): Promise<string | null> => {
    const res = await setupAdmin(username, password, displayName);
    if (res.error) return res.error;
    if (!res.token) return "setup failed";
    setAuthToken(res.token);
    setUser(res.user);
    setNeedsSetup(false);
    return null;
  }, []);

  const logout = useCallback(() => {
    setAuthToken(null);
    setUser(null);
    window.location.reload();
  }, []);

  return (
    <AuthContext.Provider value={{ ready, needsSetup, user, login, setup, logout }}>
      {children}
    </AuthContext.Provider>
  );
}

export function useAuth() {
  return useContext(AuthContext);
}
