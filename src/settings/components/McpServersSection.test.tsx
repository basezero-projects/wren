import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';

import { McpServerList } from './McpServersSection';

describe('McpServerList', () => {
  it('renders a loading affordance while servers is null', () => {
    render(
      <McpServerList
        servers={null}
        busy={false}
        error={null}
        onConnect={vi.fn()}
        onDisconnect={vi.fn()}
      />,
    );
    expect(screen.getByRole('status')).toHaveTextContent(/Loading/);
  });

  it('renders the empty-state hint when no servers are configured', () => {
    render(
      <McpServerList
        servers={[]}
        busy={false}
        error={null}
        onConnect={vi.fn()}
        onDisconnect={vi.fn()}
      />,
    );
    expect(
      screen.getByText(/No MCP servers configured yet/),
    ).toBeInTheDocument();
  });

  it('renders a Connect button on a disconnected server and fires onConnect', () => {
    const onConnect = vi.fn().mockResolvedValue(undefined);
    render(
      <McpServerList
        servers={[
          {
            name: 'syvault',
            command: 'syvault-mcp',
            args: [],
            connected: false,
            tool_count: 0,
            last_error: null,
          },
        ]}
        busy={false}
        error={null}
        onConnect={onConnect}
        onDisconnect={vi.fn()}
      />,
    );
    expect(screen.getByText('syvault')).toBeInTheDocument();
    expect(screen.getByText('Disconnected')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
    expect(onConnect).toHaveBeenCalledWith('syvault');
  });

  it('renders a Disconnect button when connected and includes tool count + arg preview', () => {
    const onDisconnect = vi.fn().mockResolvedValue(undefined);
    render(
      <McpServerList
        servers={[
          {
            name: 'ghostface',
            command: 'node',
            args: ['server.js', '--port=9000'],
            connected: true,
            tool_count: 1,
            last_error: null,
          },
        ]}
        busy={false}
        error={null}
        onConnect={vi.fn()}
        onDisconnect={onDisconnect}
      />,
    );
    expect(screen.getByText(/Connected · 1 tool/)).toBeInTheDocument();
    // Plural form check is exercised in the multi-tool test below.
    expect(screen.getByText(/node/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Disconnect' }));
    expect(onDisconnect).toHaveBeenCalledWith('ghostface');
  });

  it('uses plural "tools" when count is not 1', () => {
    render(
      <McpServerList
        servers={[
          {
            name: 'svr',
            command: 'cmd',
            args: [],
            connected: true,
            tool_count: 4,
            last_error: null,
          },
        ]}
        busy={false}
        error={null}
        onConnect={vi.fn()}
        onDisconnect={vi.fn()}
      />,
    );
    expect(screen.getByText(/Connected · 4 tools/)).toBeInTheDocument();
  });

  it('surfaces last_error inline when disconnected with an error', () => {
    render(
      <McpServerList
        servers={[
          {
            name: 'svr',
            command: 'cmd',
            args: [],
            connected: false,
            tool_count: 0,
            last_error: 'spawn ENOENT',
          },
        ]}
        busy={false}
        error={null}
        onConnect={vi.fn()}
        onDisconnect={vi.fn()}
      />,
    );
    expect(screen.getByText(/last error below/)).toBeInTheDocument();
    expect(screen.getByText(/spawn ENOENT/)).toBeInTheDocument();
  });

  it('disables both buttons while busy is true', () => {
    render(
      <McpServerList
        servers={[
          {
            name: 'svr',
            command: 'cmd',
            args: [],
            connected: false,
            tool_count: 0,
            last_error: null,
          },
        ]}
        busy={true}
        error={null}
        onConnect={vi.fn()}
        onDisconnect={vi.fn()}
      />,
    );
    expect(screen.getByRole('button', { name: 'Connect' })).toBeDisabled();
  });

  it('renders the error banner when error is set', () => {
    render(
      <McpServerList
        servers={[
          {
            name: 'svr',
            command: 'cmd',
            args: [],
            connected: false,
            tool_count: 0,
            last_error: null,
          },
        ]}
        busy={false}
        error="something went wrong"
        onConnect={vi.fn()}
        onDisconnect={vi.fn()}
      />,
    );
    expect(screen.getByRole('alert')).toHaveTextContent('something went wrong');
  });
});
