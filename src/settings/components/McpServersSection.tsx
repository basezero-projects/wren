/**
 * Settings → AI tab "MCP servers" section.
 *
 * Two stacked controls:
 *
 *  1. A `SaveField`-driven JSON textarea over `mcp.servers_json`. Edits
 *     round-trip through the standard `set_config_field` path so the
 *     loader's parse + validate + dedupe rules apply identically here
 *     and to a hand-edit of `config.toml`.
 *
 *  2. A live status list driven by the `useMcp` hook. Each row shows
 *     the server's name, command preview, current connection state,
 *     advertised tool count, and a Connect / Disconnect button. When
 *     the registry has a `last_error` for the server it surfaces inline
 *     so the user can see WHY it failed without digging into the dev
 *     console.
 *
 * The section deliberately renders even when no servers are configured
 * so first-time users see the JSON helper text and the empty-state
 * "no servers configured yet" hint instead of the section vanishing.
 */

import { Section, Textarea } from './index';
import { SaveField } from './SaveField';
import { configHelp } from '../configHelpers';
import { useMcp } from '../../hooks/useMcp';
import styles from '../../styles/settings.module.css';
import type { RawAppConfig } from '../types';

const SERVERS_JSON_MAX_CHARS = 64 * 1024;

interface McpServersSectionProps {
  config: RawAppConfig;
  resyncToken: number;
  onSaved: (next: RawAppConfig) => void;
}

export function McpServersSection({
  config,
  resyncToken,
  onSaved,
}: McpServersSectionProps) {
  const { servers, busy, error, connectServer, disconnectServer } =
    useMcp(resyncToken);

  return (
    <Section heading="MCP servers">
      <div
        style={{
          marginBottom: 8,
          fontSize: 12,
          color: 'rgba(255,255,255,0.6)',
          lineHeight: 1.5,
        }}
      >
        Wren can act as a client for any local MCP server. Each server's tools
        show up to the AI as <code>mcp__&lt;server&gt;__&lt;tool&gt;</code> and
        require an explicit approval card for every call. Paste a JSON array
        below — Wren auto-connects to every entry on launch.
      </div>
      <SaveField
        section="mcp"
        fieldKey="servers_json"
        label="Server config (JSON)"
        helper={configHelp('mcp', 'servers_json')}
        vertical
        initialValue={config.mcp.servers_json}
        resyncToken={resyncToken}
        onSaved={onSaved}
        render={(value, setValue) => (
          <>
            <Textarea
              value={value}
              onChange={setValue}
              placeholder={'[\n  {"name": "syvault", "command": "syvault-mcp"}\n]'}
              maxLength={SERVERS_JSON_MAX_CHARS}
              ariaLabel="MCP server config JSON"
            />
            <div className={styles.charCounter}>
              {value.length} / {SERVERS_JSON_MAX_CHARS}
            </div>
          </>
        )}
      />

      <McpServerList
        servers={servers}
        busy={busy}
        error={error}
        onConnect={connectServer}
        onDisconnect={disconnectServer}
      />
    </Section>
  );
}

interface McpServerListProps {
  servers: ReturnType<typeof useMcp>['servers'];
  busy: boolean;
  error: string | null;
  onConnect: (name: string) => Promise<void>;
  onDisconnect: (name: string) => Promise<void>;
}

/**
 * Pure presentational component for the per-server status rows. Split
 * out from `McpServersSection` so the rendering branches (loading /
 * empty / error / connected / disconnected) are unit-testable without
 * having to mock `invoke`.
 */
export function McpServerList({
  servers,
  busy,
  error,
  onConnect,
  onDisconnect,
}: McpServerListProps) {
  if (servers === null) {
    return (
      <div
        style={{
          marginTop: 12,
          fontSize: 12,
          color: 'rgba(255,255,255,0.5)',
        }}
        role="status"
      >
        Loading MCP server status…
      </div>
    );
  }

  if (servers.length === 0) {
    return (
      <div
        style={{
          marginTop: 12,
          fontSize: 12,
          color: 'rgba(255,255,255,0.5)',
        }}
      >
        No MCP servers configured yet. Save a JSON array above and the entries
        will appear here.
      </div>
    );
  }

  return (
    <div style={{ marginTop: 12, display: 'flex', flexDirection: 'column', gap: 8 }}>
      {error ? (
        <div
          role="alert"
          style={{
            fontSize: 12,
            color: '#ff8e8e',
            padding: '6px 10px',
            background: 'rgba(255, 80, 80, 0.08)',
            borderRadius: 6,
          }}
        >
          {error}
        </div>
      ) : null}
      {servers.map((s) => {
        const argsPreview = s.args.length > 0 ? ' ' + s.args.join(' ') : '';
        const statusLabel = s.connected
          ? `Connected · ${s.tool_count} tool${s.tool_count === 1 ? '' : 's'}`
          : s.last_error
            ? 'Disconnected — last error below'
            : 'Disconnected';
        return (
          <div
            key={s.name}
            style={{
              display: 'flex',
              alignItems: 'flex-start',
              justifyContent: 'space-between',
              gap: 12,
              padding: '8px 10px',
              borderRadius: 6,
              background: 'rgba(255,255,255,0.03)',
              border: '1px solid rgba(255,255,255,0.06)',
            }}
          >
            <div style={{ flex: 1, minWidth: 0 }}>
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                  fontSize: 13,
                  fontWeight: 600,
                }}
              >
                <span
                  aria-hidden
                  style={{
                    width: 8,
                    height: 8,
                    borderRadius: '50%',
                    background: s.connected ? '#7cd17c' : 'rgba(255,255,255,0.3)',
                  }}
                />
                {s.name}
              </div>
              <div
                style={{
                  fontSize: 11,
                  color: 'rgba(255,255,255,0.5)',
                  marginTop: 2,
                  fontFamily:
                    'ui-monospace, SFMono-Regular, "SF Mono", Menlo, monospace',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                }}
                title={`${s.command}${argsPreview}`}
              >
                {s.command}
                {argsPreview}
              </div>
              <div
                style={{
                  fontSize: 11,
                  color: s.connected
                    ? '#7cd17c'
                    : 'rgba(255,255,255,0.45)',
                  marginTop: 2,
                }}
              >
                {statusLabel}
              </div>
              {!s.connected && s.last_error ? (
                <div
                  style={{
                    fontSize: 11,
                    color: '#ff8e8e',
                    marginTop: 2,
                    whiteSpace: 'pre-wrap',
                  }}
                >
                  {s.last_error}
                </div>
              ) : null}
            </div>
            <button
              type="button"
              onClick={() =>
                s.connected ? onDisconnect(s.name) : onConnect(s.name)
              }
              disabled={busy}
              style={{
                fontSize: 12,
                padding: '4px 10px',
                borderRadius: 4,
                border: '1px solid rgba(255,255,255,0.15)',
                background: s.connected
                  ? 'rgba(255,255,255,0.04)'
                  : 'rgba(124, 209, 124, 0.1)',
                color: 'rgba(255,255,255,0.85)',
                cursor: busy ? 'not-allowed' : 'pointer',
                whiteSpace: 'nowrap',
              }}
            >
              {s.connected ? 'Disconnect' : 'Connect'}
            </button>
          </div>
        );
      })}
    </div>
  );
}
