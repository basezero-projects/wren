import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

/**
 * Lists every model installed in the local Ollama, with file size and
 * a delete control per row. Calls `list_installed_models` on mount and
 * after any successful delete.
 *
 * Delete is gated by an inline confirm step — clicking the trash icon
 * once turns the button into a "Delete?" / "Cancel" pair, two-click
 * commit. No modal dialog, no scary native prompt; just enough friction
 * that the user does not nuke a 30 GB pull by accident.
 */

interface InstalledModel {
  name: string;
  size: number;
  modified_at: string;
}

type LoadState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'ready'; models: InstalledModel[] }
  | { status: 'error'; message: string };

export function InstalledModelsList() {
  const [state, setState] = useState<LoadState>({ status: 'idle' });
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);
  const [busyDelete, setBusyDelete] = useState<string | null>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setState({ status: 'loading' });
    try {
      const models = (await invoke('list_installed_models')) as InstalledModel[];
      // Sort by modified_at desc (most recent first) so a freshly-pulled
      // model jumps to the top.
      models.sort((a, b) => (a.modified_at < b.modified_at ? 1 : -1));
      setState({ status: 'ready', models });
    } catch (e) {
      setState({
        status: 'error',
        message: `Could not list models: ${(e as Error)?.message ?? e}`,
      });
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const startDelete = useCallback((name: string) => {
    setDeleteError(null);
    setPendingDelete(name);
  }, []);

  const cancelDelete = useCallback(() => {
    setPendingDelete(null);
  }, []);

  const confirmDelete = useCallback(
    async (name: string) => {
      setBusyDelete(name);
      setDeleteError(null);
      try {
        await invoke('delete_model', { name });
        setPendingDelete(null);
        await refresh();
      } catch (e) {
        setDeleteError(
          `Could not delete ${name}: ${(e as Error)?.message ?? e}`,
        );
      } finally {
        setBusyDelete(null);
      }
    },
    [refresh],
  );

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          fontSize: 12,
          color: 'rgba(255,255,255,0.6)',
        }}
      >
        <span>
          {state.status === 'ready'
            ? `${state.models.length} model${
                state.models.length === 1 ? '' : 's'
              } installed (${fmtBytes(totalSize(state.models))})`
            : state.status === 'loading'
              ? 'Loading…'
              : ''}
        </span>
        <button
          type="button"
          onClick={() => void refresh()}
          disabled={state.status === 'loading'}
          style={{
            padding: '2px 8px',
            background: 'transparent',
            color: 'rgba(255,255,255,0.6)',
            border: '1px solid rgba(255,255,255,0.18)',
            borderRadius: 4,
            fontSize: 11,
            cursor: state.status === 'loading' ? 'wait' : 'pointer',
          }}
        >
          Refresh
        </button>
      </div>

      {deleteError && (
        <div
          style={{
            padding: '6px 10px',
            borderRadius: 6,
            background: 'rgba(224,112,112,0.06)',
            borderLeft: '3px solid #e07070',
            color: '#e88a8a',
            fontSize: 12,
          }}
        >
          {deleteError}
        </div>
      )}

      {state.status === 'error' && (
        <div
          style={{
            padding: '6px 10px',
            borderRadius: 6,
            background: 'rgba(224,112,112,0.06)',
            borderLeft: '3px solid #e07070',
            color: '#e88a8a',
            fontSize: 12,
          }}
        >
          {state.message}
        </div>
      )}

      {state.status === 'ready' && state.models.length === 0 && (
        <div
          style={{
            padding: '12px',
            textAlign: 'center',
            color: 'rgba(255,255,255,0.5)',
            fontSize: 12,
            fontStyle: 'italic',
          }}
        >
          Nothing installed yet. Use the field above to pull a model.
        </div>
      )}

      {state.status === 'ready' && state.models.length > 0 && (
        <div
          style={{
            border: '1px solid rgba(255,255,255,0.08)',
            borderRadius: 6,
            overflow: 'hidden',
          }}
        >
          {state.models.map((model, i) => {
            const confirming = pendingDelete === model.name;
            const busy = busyDelete === model.name;
            return (
              <div
                key={model.name}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 10,
                  padding: '8px 10px',
                  borderBottom:
                    i < state.models.length - 1
                      ? '1px solid rgba(255,255,255,0.06)'
                      : 'none',
                  background:
                    i % 2 === 0 ? 'transparent' : 'rgba(255,255,255,0.02)',
                  fontSize: 13,
                }}
              >
                <div
                  style={{
                    flex: 1,
                    minWidth: 0,
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                    color: 'rgba(255,255,255,0.92)',
                    fontFamily:
                      'ui-monospace, SFMono-Regular, "SF Mono", Menlo, monospace',
                    fontSize: 12,
                  }}
                  title={model.name}
                >
                  {model.name}
                </div>
                <div
                  style={{
                    flexShrink: 0,
                    width: 70,
                    textAlign: 'right',
                    color: 'rgba(255,255,255,0.55)',
                    fontSize: 11,
                    fontVariantNumeric: 'tabular-nums',
                  }}
                >
                  {fmtBytes(model.size)}
                </div>
                <div style={{ display: 'flex', gap: 4, flexShrink: 0 }}>
                  {confirming ? (
                    <>
                      <button
                        type="button"
                        onClick={() => void confirmDelete(model.name)}
                        disabled={busy}
                        style={{
                          padding: '3px 10px',
                          background: '#c75050',
                          color: '#fff',
                          border: 'none',
                          borderRadius: 4,
                          fontSize: 11,
                          fontWeight: 600,
                          cursor: busy ? 'wait' : 'pointer',
                        }}
                      >
                        {busy ? 'Deleting…' : 'Delete'}
                      </button>
                      <button
                        type="button"
                        onClick={cancelDelete}
                        disabled={busy}
                        style={{
                          padding: '3px 10px',
                          background: 'transparent',
                          color: 'rgba(255,255,255,0.7)',
                          border: '1px solid rgba(255,255,255,0.2)',
                          borderRadius: 4,
                          fontSize: 11,
                          cursor: busy ? 'wait' : 'pointer',
                        }}
                      >
                        Cancel
                      </button>
                    </>
                  ) : (
                    <button
                      type="button"
                      onClick={() => startDelete(model.name)}
                      aria-label={`Uninstall ${model.name}`}
                      title="Uninstall"
                      style={{
                        width: 28,
                        height: 24,
                        display: 'inline-flex',
                        alignItems: 'center',
                        justifyContent: 'center',
                        background: 'transparent',
                        color: 'rgba(255,255,255,0.5)',
                        border: '1px solid transparent',
                        borderRadius: 4,
                        cursor: 'pointer',
                      }}
                      onMouseEnter={(e) => {
                        e.currentTarget.style.color = '#e88a8a';
                        e.currentTarget.style.borderColor =
                          'rgba(224,112,112,0.4)';
                      }}
                      onMouseLeave={(e) => {
                        e.currentTarget.style.color = 'rgba(255,255,255,0.5)';
                        e.currentTarget.style.borderColor = 'transparent';
                      }}
                    >
                      <TrashIcon />
                    </button>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

function totalSize(models: InstalledModel[]): number {
  let n = 0;
  for (const m of models) n += m.size;
  return n;
}

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function TrashIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <polyline points="3 6 5 6 21 6" />
      <path d="M19 6l-2 14a2 2 0 0 1-2 2H9a2 2 0 0 1-2-2L5 6" />
      <path d="M10 11v6" />
      <path d="M14 11v6" />
      <path d="M9 6V4a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2v2" />
    </svg>
  );
}
