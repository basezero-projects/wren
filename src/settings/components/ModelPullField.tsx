import { useCallback, useRef, useState } from 'react';
import { Channel, invoke } from '@tauri-apps/api/core';

/**
 * Lets the user install an Ollama model from inside Wren — no
 * terminal trip to `ollama pull`. Streams progress over a Tauri
 * Channel from the `pull_model` command.
 *
 * One pull at a time per Wren window. The same `cancel_generation`
 * Tauri command that interrupts a chat turn also stops a running
 * pull (the backend command stores its CancellationToken in the
 * shared GenerationState).
 */

type PullEvent =
  | { type: 'Status'; data: string }
  | { type: 'Progress'; data: { digest: string; total: number; completed: number } }
  | { type: 'Done' }
  | { type: 'Cancelled' }
  | { type: 'Error'; data: string };

interface PullState {
  status: 'idle' | 'running' | 'done' | 'error' | 'cancelled';
  // Latest free-form Ollama status line ("pulling manifest", etc.).
  message: string;
  // Per-digest byte progress, keyed by digest. Aggregate to display.
  byDigest: Record<string, { total: number; completed: number }>;
}

const INITIAL_STATE: PullState = {
  status: 'idle',
  message: '',
  byDigest: {},
};

function aggregate(byDigest: PullState['byDigest']) {
  let total = 0;
  let completed = 0;
  for (const v of Object.values(byDigest)) {
    total += v.total;
    completed += v.completed;
  }
  return { total, completed };
}

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

export function ModelPullField() {
  const [name, setName] = useState('');
  const [state, setState] = useState<PullState>(INITIAL_STATE);
  const cancellingRef = useRef(false);

  const isRunning = state.status === 'running';
  const { total, completed } = aggregate(state.byDigest);
  const pct = total > 0 ? Math.min(100, Math.round((completed / total) * 100)) : 0;

  const start = useCallback(async () => {
    const slug = name.trim();
    if (!slug || isRunning) return;

    cancellingRef.current = false;
    setState({
      status: 'running',
      message: 'Connecting to Ollama…',
      byDigest: {},
    });

    const channel = new Channel<PullEvent>();
    channel.onmessage = (event) => {
      switch (event.type) {
        case 'Status':
          setState((prev) =>
            prev.status === 'running'
              ? { ...prev, message: event.data }
              : prev,
          );
          break;
        case 'Progress':
          setState((prev) =>
            prev.status === 'running'
              ? {
                  ...prev,
                  byDigest: {
                    ...prev.byDigest,
                    [event.data.digest]: {
                      total: event.data.total,
                      completed: event.data.completed,
                    },
                  },
                }
              : prev,
          );
          break;
        case 'Done':
          setState((prev) => ({
            ...prev,
            status: 'done',
            message: `Installed ${slug}.`,
          }));
          break;
        case 'Cancelled':
          setState((prev) => ({
            ...prev,
            status: 'cancelled',
            message: 'Cancelled.',
          }));
          break;
        case 'Error':
          setState((prev) => ({
            ...prev,
            status: 'error',
            message: event.data,
          }));
          break;
      }
    };

    try {
      await invoke('pull_model', { name: slug, onEvent: channel });
    } catch (e) {
      // The Tauri command itself rarely throws; almost everything is
      // surfaced as an Error event over the channel. Treat anything
      // that escapes here as a fallback failure.
      setState((prev) => ({
        ...prev,
        status: 'error',
        message: `Could not start pull: ${(e as Error)?.message ?? e}`,
      }));
    }
  }, [name, isRunning]);

  const cancel = useCallback(async () => {
    if (!isRunning || cancellingRef.current) return;
    cancellingRef.current = true;
    try {
      await invoke('cancel_generation');
    } catch {
      // Best-effort; channel will eventually emit Cancelled or Error.
    }
  }, [isRunning]);

  const reset = useCallback(() => {
    setState(INITIAL_STATE);
  }, []);

  const canPull = !isRunning && name.trim().length > 0;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
        <input
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="qwen3:8b"
          aria-label="Model to download"
          disabled={isRunning}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && canPull) {
              e.preventDefault();
              void start();
            }
          }}
          style={{
            flex: 1,
            padding: '6px 10px',
            borderRadius: 5,
            border: '1px solid rgba(255,255,255,0.18)',
            background: 'rgba(0,0,0,0.3)',
            color: 'rgba(255,255,255,0.92)',
            fontSize: 13,
            outline: 'none',
          }}
        />
        {!isRunning ? (
          <button
            type="button"
            onClick={() => void start()}
            disabled={!canPull}
            style={{
              padding: '6px 14px',
              background: canPull ? '#d4af37' : 'rgba(212,175,55,0.3)',
              color: '#0c0c0d',
              border: 'none',
              borderRadius: 5,
              fontWeight: 600,
              cursor: canPull ? 'pointer' : 'not-allowed',
              fontSize: 13,
            }}
          >
            Pull
          </button>
        ) : (
          <button
            type="button"
            onClick={() => void cancel()}
            style={{
              padding: '6px 14px',
              background: 'transparent',
              color: 'rgba(255,255,255,0.85)',
              border: '1px solid rgba(255,255,255,0.25)',
              borderRadius: 5,
              cursor: 'pointer',
              fontSize: 13,
            }}
          >
            Cancel
          </button>
        )}
      </div>

      {state.status !== 'idle' && (
        <div
          style={{
            padding: '8px 10px',
            borderRadius: 6,
            background: 'rgba(255,255,255,0.04)',
            borderLeft: `3px solid ${barColor(state.status)}`,
            fontSize: 12,
            color: 'rgba(255,255,255,0.85)',
          }}
        >
          <div style={{ marginBottom: total > 0 ? 6 : 0 }}>{state.message}</div>
          {total > 0 && state.status === 'running' && (
            <>
              <div
                style={{
                  height: 6,
                  background: 'rgba(255,255,255,0.08)',
                  borderRadius: 3,
                  overflow: 'hidden',
                }}
              >
                <div
                  style={{
                    height: '100%',
                    width: `${pct}%`,
                    background: '#d4af37',
                    transition: 'width 200ms ease-out',
                  }}
                />
              </div>
              <div
                style={{
                  marginTop: 4,
                  fontSize: 11,
                  color: 'rgba(255,255,255,0.55)',
                }}
              >
                {fmtBytes(completed)} / {fmtBytes(total)} ({pct}%)
              </div>
            </>
          )}
          {(state.status === 'done' ||
            state.status === 'error' ||
            state.status === 'cancelled') && (
            <button
              type="button"
              onClick={reset}
              style={{
                marginTop: 6,
                padding: '2px 8px',
                background: 'transparent',
                color: 'rgba(255,255,255,0.6)',
                border: '1px solid rgba(255,255,255,0.18)',
                borderRadius: 4,
                fontSize: 11,
                cursor: 'pointer',
              }}
            >
              Dismiss
            </button>
          )}
        </div>
      )}
    </div>
  );
}

function barColor(status: PullState['status']): string {
  switch (status) {
    case 'running':
      return '#d4af37';
    case 'done':
      return '#5cc97e';
    case 'error':
      return '#e07070';
    case 'cancelled':
      return '#888';
    default:
      return 'transparent';
  }
}
