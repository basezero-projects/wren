import { renderHook, act, waitFor } from '@testing-library/react';
import { describe, it, expect, beforeEach } from 'vitest';

import { describeMcpError, useMcp } from '../useMcp';
import { invoke } from '../../testUtils/mocks/tauri';

const SAMPLE_SERVERS = [
  {
    name: 'syvault',
    command: 'syvault-mcp',
    args: [],
    connected: true,
    tool_count: 3,
    last_error: null,
  },
];

describe('useMcp', () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it('starts with servers=null and refreshes on mount', async () => {
    invoke.mockImplementation(async (cmd) => {
      if (cmd === 'mcp_list_servers') return SAMPLE_SERVERS;
      return undefined;
    });
    const { result } = renderHook(() => useMcp(0));
    expect(result.current.servers).toBeNull();
    await waitFor(() => {
      expect(result.current.servers).toEqual(SAMPLE_SERVERS);
    });
    expect(result.current.error).toBeNull();
  });

  it('coerces a non-array list payload to an empty array', async () => {
    // Defends against test-mock or downlevel-client undefined responses
    // — without the coercion, the section's `.length === 0` branch
    // would crash on undefined.
    invoke.mockImplementation(async (cmd) => {
      if (cmd === 'mcp_list_servers') return undefined;
      return undefined;
    });
    const { result } = renderHook(() => useMcp(0));
    await waitFor(() => {
      expect(result.current.servers).toEqual([]);
    });
  });

  it('records an error when mcp_list_servers rejects', async () => {
    invoke.mockImplementation(async (cmd) => {
      if (cmd === 'mcp_list_servers') {
        throw 'boom';
      }
      return undefined;
    });
    const { result } = renderHook(() => useMcp(0));
    await waitFor(() => {
      expect(result.current.error).toBe('boom');
    });
    // Servers stay null since no good payload arrived.
    expect(result.current.servers).toBeNull();
  });

  it('connectServer toggles busy and re-lists on success', async () => {
    let callCount = 0;
    invoke.mockImplementation(async (cmd) => {
      if (cmd === 'mcp_list_servers') {
        callCount += 1;
        return callCount === 1 ? [] : SAMPLE_SERVERS;
      }
      if (cmd === 'mcp_connect_server') {
        return SAMPLE_SERVERS[0];
      }
      return undefined;
    });
    const { result } = renderHook(() => useMcp(0));
    await waitFor(() => expect(result.current.servers).toEqual([]));

    await act(async () => {
      await result.current.connectServer('syvault');
    });
    expect(result.current.servers).toEqual(SAMPLE_SERVERS);
    expect(result.current.busy).toBe(false);
    expect(result.current.error).toBeNull();
  });

  it('connectServer surfaces the rejection and still re-lists', async () => {
    invoke.mockImplementation(async (cmd) => {
      if (cmd === 'mcp_list_servers') return SAMPLE_SERVERS;
      if (cmd === 'mcp_connect_server') throw new Error('binary not found');
      return undefined;
    });
    const { result } = renderHook(() => useMcp(0));
    await waitFor(() => expect(result.current.servers).toEqual(SAMPLE_SERVERS));

    await act(async () => {
      await result.current.connectServer('syvault');
    });
    expect(result.current.error).toBe('binary not found');
    expect(result.current.busy).toBe(false);
  });

  it('disconnectServer clears the connection and refreshes', async () => {
    let listed = SAMPLE_SERVERS;
    invoke.mockImplementation(async (cmd) => {
      if (cmd === 'mcp_list_servers') return listed;
      if (cmd === 'mcp_disconnect_server') {
        listed = [{ ...SAMPLE_SERVERS[0], connected: false, tool_count: 0 }];
        return null;
      }
      return undefined;
    });
    const { result } = renderHook(() => useMcp(0));
    await waitFor(() => expect(result.current.servers?.[0].connected).toBe(true));

    await act(async () => {
      await result.current.disconnectServer('syvault');
    });
    expect(result.current.servers?.[0].connected).toBe(false);
    expect(result.current.error).toBeNull();
  });

  it('disconnectServer surfaces the rejection', async () => {
    invoke.mockImplementation(async (cmd) => {
      if (cmd === 'mcp_list_servers') return SAMPLE_SERVERS;
      if (cmd === 'mcp_disconnect_server') throw 'cannot disconnect';
      return undefined;
    });
    const { result } = renderHook(() => useMcp(0));
    await waitFor(() => expect(result.current.servers).toEqual(SAMPLE_SERVERS));

    await act(async () => {
      await result.current.disconnectServer('syvault');
    });
    expect(result.current.error).toBe('cannot disconnect');
  });

  it('refresh re-fetches when called manually', async () => {
    let listed = SAMPLE_SERVERS;
    invoke.mockImplementation(async (cmd) => {
      if (cmd === 'mcp_list_servers') return listed;
      return undefined;
    });
    const { result } = renderHook(() => useMcp(0));
    await waitFor(() => expect(result.current.servers).toEqual(SAMPLE_SERVERS));

    listed = [];
    await act(async () => {
      await result.current.refresh();
    });
    expect(result.current.servers).toEqual([]);
  });

  it('refetches when resyncToken changes', async () => {
    invoke.mockImplementation(async () => SAMPLE_SERVERS);
    const { rerender } = renderHook((token: number) => useMcp(token), {
      initialProps: 0,
    });
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledTimes(1);
    });
    rerender(1);
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledTimes(2);
    });
  });
});

describe('describeMcpError', () => {
  it('returns the string verbatim', () => {
    expect(describeMcpError('plain message')).toBe('plain message');
  });

  it('extracts message from an Error instance', () => {
    expect(describeMcpError(new Error('boom'))).toBe('boom');
  });

  it('extracts message from a serialized object error', () => {
    expect(describeMcpError({ message: 'nope' })).toBe('nope');
  });

  it('falls back to a generic label for unsupported shapes', () => {
    expect(describeMcpError(42)).toBe('MCP command failed.');
    expect(describeMcpError(null)).toBe('MCP command failed.');
    expect(describeMcpError({ other: true })).toBe('MCP command failed.');
  });
});
