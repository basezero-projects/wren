/**
 * Hook for the Settings → AI → MCP servers section.
 *
 * Wraps the three Tauri commands the section needs (`mcp_list_servers`,
 * `mcp_connect_server`, `mcp_disconnect_server`) in a small async API.
 * Refetches the server list whenever the config changes (`resyncToken`)
 * so editing the JSON blob and saving it surfaces the new entries
 * without a manual reload, and after every successful connect /
 * disconnect so the connection-status badge updates immediately.
 *
 * The hook does not own the JSON-blob form value — that goes through
 * the standard `SaveField` -> `set_config_field` round-trip like every
 * other tunable field. It only owns the live connection-state view.
 */

import { useCallback, useEffect, useState } from 'react';

import { invoke } from '@tauri-apps/api/core';

/** Mirrors `crate::mcp::McpServerStatus` on the Rust side. */
export interface McpServerStatus {
  name: string;
  command: string;
  args: string[];
  /** True when there is a live child process and a tool list cached. */
  connected: boolean;
  /** Number of tools the server advertised on its last `tools/list`. */
  tool_count: number;
  /** Optional human-readable last-connect-error string. */
  last_error: string | null;
}

interface UseMcpResult {
  /** The most recent `mcp_list_servers` snapshot, or `null` while
   *  the very first list call is in flight. */
  servers: McpServerStatus[] | null;
  /** True when a `connectServer` / `disconnectServer` call is awaiting
   *  a response. The form disables its buttons while busy. */
  busy: boolean;
  /** Last error from any of the three commands, surfaced inline in
   *  the section. Cleared on the next successful command. */
  error: string | null;
  refresh: () => Promise<void>;
  connectServer: (name: string) => Promise<void>;
  disconnectServer: (name: string) => Promise<void>;
}

/**
 * Best-effort string coercion for the typed errors the Rust side
 * returns. The MCP commands use plain `Result<T, String>` so most paths
 * already arrive here as a string; the fallback handles the unusual
 * case of a serialized object error coming through.
 */
export function describeMcpError(err: unknown): string {
  if (typeof err === 'string') return err;
  if (err instanceof Error) return err.message;
  if (typeof err === 'object' && err !== null) {
    const obj = err as { message?: unknown };
    if (typeof obj.message === 'string') return obj.message;
  }
  return 'MCP command failed.';
}

export function useMcp(resyncToken: number): UseMcpResult {
  const [servers, setServers] = useState<McpServerStatus[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const list = await invoke<McpServerStatus[] | null | undefined>(
        'mcp_list_servers',
      );
      // The Rust command always returns an array, but a unit-test mock
      // (or a downlevel client) may pass `undefined` — coerce so the
      // section below renders the empty state instead of crashing on
      // `.length`.
      setServers(Array.isArray(list) ? list : []);
      setError(null);
    } catch (e) {
      setError(describeMcpError(e));
    }
  }, []);

  // Initial fetch + re-fetch whenever the config changes.
  useEffect(() => {
    void refresh();
  }, [refresh, resyncToken]);

  const connectServer = useCallback(
    async (name: string) => {
      setBusy(true);
      let actionError: string | null = null;
      try {
        await invoke('mcp_connect_server', { name });
      } catch (e) {
        actionError = describeMcpError(e);
      }
      // Always refresh so the registry's `last_error` (populated by the
      // backend on a connect failure) is reflected in the list. The
      // connect-call's own error wins over the refresh's null-clear so
      // the user sees the precise rejection reason.
      await refresh();
      if (actionError !== null) {
        setError(actionError);
      }
      setBusy(false);
    },
    [refresh],
  );

  const disconnectServer = useCallback(
    async (name: string) => {
      setBusy(true);
      let actionError: string | null = null;
      try {
        await invoke('mcp_disconnect_server', { name });
      } catch (e) {
        actionError = describeMcpError(e);
      }
      await refresh();
      if (actionError !== null) {
        setError(actionError);
      }
      setBusy(false);
    },
    [refresh],
  );

  return {
    servers,
    busy,
    error,
    refresh,
    connectServer,
    disconnectServer,
  };
}
