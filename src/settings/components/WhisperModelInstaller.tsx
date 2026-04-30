import { useCallback, useEffect, useRef, useState } from 'react';
import { Channel, invoke } from '@tauri-apps/api/core';

/**
 * Pulls a whisper.cpp ggml model from HuggingFace into Wren's
 * `<app_data_dir>/whisper-models/` directory. Mirrors `ModelPullField`
 * for Ollama models, but the Rust side knows the URL up front, so the
 * UI is a fixed dropdown of curated sizes rather than a free-form slug
 * input.
 *
 * The user picks a model size, hits Install, and watches the byte
 * progress bar; on success the parent's `onInstalled` callback fires
 * with the filename so it can refresh the installed-models list and
 * (if no model was selected before) auto-pick this one.
 */

interface WhisperModelOption {
  filename: string;
  /** Friendly label rendered in the dropdown. */
  label: string;
  /** Approximate disk size for the helper text. */
  approxMB: number;
  /** One-line description of the trade-off. */
  description: string;
}

/**
 * Curated subset of the ggerganov/whisper.cpp model lineup. The full
 * lineup includes language-specific variants and non-quantized weights;
 * we ship only the four sizes that hit the speed/accuracy sweet spots
 * for English push-to-talk dictation. Users who want a different model
 * can drop the .bin file directly into the models directory and it
 * shows up in the installed list.
 */
const MODEL_OPTIONS: ReadonlyArray<WhisperModelOption> = [
  {
    filename: 'ggml-tiny.en-q5_1.bin',
    label: 'Tiny (English) — fastest',
    approxMB: 33,
    description:
      'Instant transcription, more mistakes on tricky words. Fine for everyday dictation.',
  },
  {
    filename: 'ggml-base.en-q5_1.bin',
    label: 'Base (English) — balanced',
    approxMB: 60,
    description:
      'Good accuracy, ~1s pause on release. Recommended starting point.',
  },
  {
    filename: 'ggml-small.en-q5_1.bin',
    label: 'Small (English) — accurate',
    approxMB: 190,
    description:
      'Catches unusual words and accents. ~2-3s pause on release.',
  },
  {
    filename: 'ggml-medium.en-q5_1.bin',
    label: 'Medium (English) — most accurate',
    approxMB: 539,
    description:
      'Highest accuracy of the curated lineup. ~4-5s pause on release.',
  },
];

type DownloadEvent =
  | { type: 'Status'; data: string }
  | { type: 'Progress'; data: { total: number; completed: number } }
  | { type: 'Done' }
  | { type: 'Cancelled' }
  | { type: 'Error'; data: string };

interface DownloadState {
  status: 'idle' | 'running' | 'done' | 'error' | 'cancelled';
  message: string;
  total: number;
  completed: number;
}

const INITIAL_STATE: DownloadState = {
  status: 'idle',
  message: '',
  total: 0,
  completed: 0,
};

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

export function WhisperModelInstaller({
  onInstalled,
  excludeFilenames,
}: {
  onInstalled: (filename: string) => void;
  /** Filenames already on disk; we hide them from the dropdown. */
  excludeFilenames: ReadonlySet<string>;
}) {
  const available = MODEL_OPTIONS.filter(
    (m) => !excludeFilenames.has(m.filename),
  );

  const [selected, setSelected] = useState<string>(
    () => available[0]?.filename ?? '',
  );
  const [state, setState] = useState<DownloadState>(INITIAL_STATE);
  const cancellingRef = useRef(false);

  // If the install set changes (a download finished, or the user
  // deleted one elsewhere), reseat the dropdown to a still-installable
  // option so the controlled value never points at a missing item.
  useEffect(() => {
    if (!available.find((m) => m.filename === selected)) {
      setSelected(available[0]?.filename ?? '');
    }
  }, [available, selected]);

  const isRunning = state.status === 'running';
  const pct =
    state.total > 0
      ? Math.min(100, Math.round((state.completed / state.total) * 100))
      : 0;

  const start = useCallback(async () => {
    if (!selected || isRunning) return;
    cancellingRef.current = false;
    setState({
      status: 'running',
      message: 'Connecting to HuggingFace…',
      total: 0,
      completed: 0,
    });

    const channel = new Channel<DownloadEvent>();
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
                  total: event.data.total,
                  completed: event.data.completed,
                }
              : prev,
          );
          break;
        case 'Done':
          setState({
            status: 'done',
            message: `Installed ${selected}.`,
            total: 0,
            completed: 0,
          });
          onInstalled(selected);
          break;
        case 'Cancelled':
          setState({
            status: 'cancelled',
            message: 'Cancelled.',
            total: 0,
            completed: 0,
          });
          break;
        case 'Error':
          setState({
            status: 'error',
            message: event.data,
            total: 0,
            completed: 0,
          });
          break;
      }
    };

    try {
      await invoke('download_whisper_model', {
        filename: selected,
        onEvent: channel,
      });
    } catch (e) {
      setState({
        status: 'error',
        message: `Could not start install: ${(e as Error)?.message ?? e}`,
        total: 0,
        completed: 0,
      });
    }
  }, [selected, isRunning, onInstalled]);

  const cancel = useCallback(async () => {
    if (!isRunning || cancellingRef.current) return;
    cancellingRef.current = true;
    try {
      await invoke('cancel_whisper_download');
    } catch {
      // Best-effort; channel will surface Cancelled / Error.
    }
  }, [isRunning]);

  const reset = useCallback(() => setState(INITIAL_STATE), []);

  if (available.length === 0 && !isRunning && state.status === 'idle') {
    return (
      <div style={{ fontSize: 12, color: 'rgba(255,255,255,0.55)' }}>
        Every curated model is already installed. Drop other ggml-*.bin files
        into the models folder to add them.
      </div>
    );
  }

  const selectedOption = MODEL_OPTIONS.find((m) => m.filename === selected);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
        <select
          value={selected}
          onChange={(e) => setSelected(e.target.value)}
          disabled={isRunning || available.length === 0}
          aria-label="Model size to install"
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
        >
          {available.map((m) => (
            <option key={m.filename} value={m.filename}>
              {m.label} (~{m.approxMB} MB)
            </option>
          ))}
        </select>
        {!isRunning ? (
          <button
            type="button"
            onClick={() => void start()}
            disabled={!selected}
            style={{
              padding: '6px 14px',
              background: selected ? '#d4af37' : 'rgba(212,175,55,0.3)',
              color: '#0c0c0d',
              border: 'none',
              borderRadius: 5,
              fontWeight: 600,
              cursor: selected ? 'pointer' : 'not-allowed',
              fontSize: 13,
            }}
          >
            Install
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

      {selectedOption && state.status === 'idle' ? (
        <div style={{ fontSize: 11, color: 'rgba(255,255,255,0.55)', lineHeight: 1.5 }}>
          {selectedOption.description}
        </div>
      ) : null}

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
          <div style={{ marginBottom: state.total > 0 ? 6 : 0 }}>
            {state.message}
          </div>
          {state.total > 0 && state.status === 'running' && (
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
                {fmtBytes(state.completed)} / {fmtBytes(state.total)} ({pct}%)
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

function barColor(status: DownloadState['status']): string {
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
