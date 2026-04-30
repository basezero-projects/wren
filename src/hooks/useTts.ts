import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import { useConfig } from '../contexts/ConfigContext';

/**
 * Push-to-speech control for completed assistant responses.
 *
 * `speak(text)` checks the live `[voice].tts_enabled` setting and, if
 * enabled, hands the response to the Rust backend. The backend takes
 * care of cancelling any in-flight speaker, applying the active voice
 * and rate, and writing the text payload to a temp file before
 * spawning PowerShell — the hook only sends the text and tracks
 * "is something speaking right now" so the chat UI can render a Stop
 * button.
 *
 * Tracking the speaking state purely on the frontend (rather than
 * polling the backend) is fine for v0: the only paths that change it
 * are this hook's own calls, so nothing observable can drift. If a
 * later version adds a way to start speech from elsewhere, replace
 * this with a backend event.
 */
export function useTts() {
  const { voice } = useConfig();
  const [isSpeaking, setIsSpeaking] = useState(false);
  // Latest enabled flag in a ref so callers that closed over an old
  // value (e.g. `onTurnComplete`) still pick up the user's most recent
  // toggle without re-rendering them.
  const enabledRef = useRef(voice.ttsEnabled);
  useEffect(() => {
    enabledRef.current = voice.ttsEnabled;
  }, [voice.ttsEnabled]);

  const stop = useCallback(async () => {
    setIsSpeaking(false);
    try {
      await invoke('tts_stop');
    } catch {
      // Best-effort; the backend may already have nothing to stop.
    }
  }, []);

  const speak = useCallback(async (text: string) => {
    if (!enabledRef.current) return;
    const trimmed = text.trim();
    if (!trimmed) return;
    setIsSpeaking(true);
    try {
      await invoke('tts_speak', { text: trimmed });
    } catch {
      // Backend will report a stderr message in the dev console; we
      // never block chat on TTS failure.
      setIsSpeaking(false);
    }
  }, []);

  // Stop any in-flight speech when this hook unmounts (window
  // closing, conversation reset that re-mounts the chat tree, etc.).
  useEffect(() => {
    return () => {
      void invoke('tts_stop').catch(() => {});
    };
  }, []);

  return { speak, stop, isSpeaking };
}
