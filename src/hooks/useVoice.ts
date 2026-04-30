import { useCallback, useEffect, useRef, useState } from 'react';
import { Channel, invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

/**
 * Push-to-talk dictation hook.
 *
 * Listens to the `wren://voice-hotkey` event emitted by the backend
 * Ctrl+Shift+Space global shortcut (overlay-only) and drives the full
 * lifecycle: arm a Channel, invoke `voice_record`, finalize on release,
 * forward the transcript to whatever the caller wires `onTranscript`
 * to. Cancellable via `cancel()` for "esc / overlay closed" code paths.
 *
 * Single-recording-at-a-time is enforced backend-side; this hook tracks
 * its own UI status independently (`idle | listening | transcribing`)
 * so the AskBar can show a glowing mic indicator even before the first
 * VoiceEvent arrives over the channel.
 */

export type VoiceStatus = 'idle' | 'listening' | 'transcribing' | 'error';

type VoiceEvent =
  | { type: 'Listening' }
  | { type: 'Transcribing' }
  | { type: 'Final'; data: string }
  | { type: 'Cancelled' }
  | { type: 'Error'; data: string };

interface UseVoiceOptions {
  /** Filename of the active whisper model (e.g. `ggml-base.en-q5_1.bin`). */
  modelFilename: string;
  /** Whether voice input is enabled in config. The hook short-circuits to a no-op when false. */
  enabled: boolean;
  /** Called with the final transcript text. Empty string means "silence — do nothing". */
  onTranscript: (text: string) => void;
  /** Called with a user-facing error message when the pipeline fails. */
  onError?: (message: string) => void;
}

interface UseVoiceResult {
  status: VoiceStatus;
  /** Manually start a recording session (e.g. mic button). */
  start: () => Promise<void>;
  /** Manually finalize the active session and run whisper. */
  finalize: () => Promise<void>;
  /** Manually cancel the active session without transcribing. */
  cancel: () => Promise<void>;
  /** Last error message, or null. Cleared on each new start. */
  errorMessage: string | null;
}

export function useVoice({
  modelFilename,
  enabled,
  onTranscript,
  onError,
}: UseVoiceOptions): UseVoiceResult {
  const [status, setStatus] = useState<VoiceStatus>('idle');
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  // Refs so the hotkey listener and start/finalize/cancel functions
  // share state without triggering re-renders. The listener also
  // closes over `enabled` and `modelFilename` via refs so a stale
  // listener registration cannot record with the wrong model.
  const inFlightRef = useRef(false);
  const enabledRef = useRef(enabled);
  const modelRef = useRef(modelFilename);
  const onTranscriptRef = useRef(onTranscript);
  const onErrorRef = useRef(onError);

  useEffect(() => {
    enabledRef.current = enabled;
  }, [enabled]);
  useEffect(() => {
    modelRef.current = modelFilename;
  }, [modelFilename]);
  useEffect(() => {
    onTranscriptRef.current = onTranscript;
  }, [onTranscript]);
  useEffect(() => {
    onErrorRef.current = onError;
  }, [onError]);

  const start = useCallback(async () => {
    if (inFlightRef.current) return;
    if (!enabledRef.current) return;
    const model = modelRef.current.trim();
    if (!model) {
      const msg = 'No voice model installed. Pick one in Settings → Voice.';
      setErrorMessage(msg);
      setStatus('error');
      onErrorRef.current?.(msg);
      return;
    }

    inFlightRef.current = true;
    setErrorMessage(null);
    setStatus('listening');

    const channel = new Channel<VoiceEvent>();
    channel.onmessage = (event) => {
      switch (event.type) {
        case 'Listening':
          setStatus('listening');
          break;
        case 'Transcribing':
          setStatus('transcribing');
          break;
        case 'Final': {
          inFlightRef.current = false;
          setStatus('idle');
          const text = event.data.trim();
          if (text.length > 0) {
            onTranscriptRef.current(text);
          }
          break;
        }
        case 'Cancelled':
          inFlightRef.current = false;
          setStatus('idle');
          break;
        case 'Error':
          inFlightRef.current = false;
          setStatus('error');
          setErrorMessage(event.data);
          onErrorRef.current?.(event.data);
          break;
      }
    };

    try {
      await invoke('voice_record', {
        modelFilename: model,
        onEvent: channel,
      });
    } catch (e) {
      inFlightRef.current = false;
      const msg = `Could not start recording: ${(e as Error)?.message ?? e}`;
      setStatus('error');
      setErrorMessage(msg);
      onErrorRef.current?.(msg);
    }
  }, []);

  const finalize = useCallback(async () => {
    if (!inFlightRef.current) return;
    try {
      await invoke('voice_finalize');
    } catch {
      // Best-effort; the channel will surface any failure.
    }
  }, []);

  const cancel = useCallback(async () => {
    if (!inFlightRef.current) return;
    try {
      await invoke('voice_cancel');
    } catch {
      // Best-effort.
    }
  }, []);

  // Wire the global Ctrl+Shift+Space hotkey to start/finalize. The
  // backend already gates the event on overlay-visible, so we don't
  // need to re-check that here. We DO gate on `enabled` so the hotkey
  // doesn't fire while voice is disabled in config.
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void listen<{ phase: 'press' | 'release' }>(
      'wren://voice-hotkey',
      (event) => {
        if (!enabledRef.current) return;
        if (event.payload.phase === 'press') {
          void start();
        } else {
          void finalize();
        }
      },
    ).then((un) => {
      if (cancelled) un();
      else unlisten = un;
    });
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [start, finalize]);

  return { status, start, finalize, cancel, errorMessage };
}
