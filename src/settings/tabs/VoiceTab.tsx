/**
 * Voice tab.
 *
 * Three controls in one panel:
 *   1. Enable toggle — gates the Ctrl+Shift+Space hotkey.
 *   2. Active-model picker — dropdown over what's installed on disk.
 *   3. Install / uninstall — pull from HuggingFace, delete from disk.
 *
 * The model picker is its own field rather than a free-form text input
 * because the user needs to pick from a known-installed list, not type a
 * filename. We push the picked filename through `set_config_field` like
 * any other config value.
 */

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import { Section, SettingRow } from '../components';
import { SaveField } from '../components/SaveField';
import { WhisperModelInstaller } from '../components/WhisperModelInstaller';
import { configHelp } from '../configHelpers';
import type { RawAppConfig } from '../types';

interface InstalledWhisperModel {
  filename: string;
  size: number;
}

interface InstalledSapiVoice {
  name: string;
  culture: string;
}

interface VoiceTabProps {
  config: RawAppConfig;
  resyncToken: number;
  onSaved: (next: RawAppConfig) => void;
}

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

export function VoiceTab({ config, resyncToken, onSaved }: VoiceTabProps) {
  const [installed, setInstalled] = useState<InstalledWhisperModel[]>([]);
  const [confirmDeleteName, setConfirmDeleteName] = useState<string | null>(
    null,
  );
  const [sapiVoices, setSapiVoices] = useState<InstalledSapiVoice[]>([]);
  const [sapiVoicesError, setSapiVoicesError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const list = await invoke<InstalledWhisperModel[]>('list_whisper_models');
      setInstalled(list);
    } catch {
      setInstalled([]);
    }
  }, []);

  const refreshSapiVoices = useCallback(async () => {
    try {
      const list = await invoke<InstalledSapiVoice[]>('tts_list_voices');
      setSapiVoices(list);
      setSapiVoicesError(null);
    } catch (err) {
      setSapiVoices([]);
      setSapiVoicesError(
        typeof err === 'string'
          ? err
          : 'Could not query Windows SAPI voices.',
      );
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    void refreshSapiVoices();
  }, [refreshSapiVoices]);

  const installedFilenames = new Set(installed.map((m) => m.filename));

  const handleInstalled = useCallback(
    async (filename: string) => {
      await refresh();
      // If the user has no active model picked yet, auto-pick the
      // freshly-installed one so push-to-talk works immediately.
      if (!config.voice.model) {
        try {
          const next = await invoke<RawAppConfig>('set_config_field', {
            section: 'voice',
            key: 'model',
            value: filename,
          });
          onSaved(next);
        } catch {
          // Non-fatal: user can still pick it manually from the
          // dropdown below. The picker reflects the current config.
        }
      }
    },
    [refresh, config.voice.model, onSaved],
  );

  const handleDelete = useCallback(
    async (filename: string) => {
      try {
        await invoke('delete_whisper_model', { filename });
      } catch {
        // No-op; the next refresh will reveal whatever actual state.
      }
      // If the user just deleted the active model, blank the config
      // field so we don't leave a dangling reference.
      if (config.voice.model === filename) {
        try {
          const next = await invoke<RawAppConfig>('set_config_field', {
            section: 'voice',
            key: 'model',
            value: '',
          });
          onSaved(next);
        } catch {
          // Best effort.
        }
      }
      await refresh();
      setConfirmDeleteName(null);
    },
    [refresh, config.voice.model, onSaved],
  );

  return (
    <>
      <Section heading="Push-to-talk">
        <div
          style={{
            marginBottom: 8,
            fontSize: 12,
            color: 'rgba(255,255,255,0.6)',
            lineHeight: 1.5,
          }}
        >
          Hold <kbd style={kbdStyle}>Ctrl+Shift+Space</kbd> while Wren is open
          to dictate; release to transcribe. Audio stays on this machine —
          Wren runs whisper.cpp locally with the model you pick below.
        </div>
        <SaveField
          section="voice"
          fieldKey="enabled"
          label="Enable voice input"
          helper={configHelp('voice', 'enabled')}
          initialValue={config.voice.enabled}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <label
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                gap: 8,
                cursor: 'pointer',
              }}
            >
              <input
                type="checkbox"
                checked={value}
                onChange={(e) => setValue(e.target.checked)}
                aria-label="Enable voice input"
              />
              <span style={{ fontSize: 13, color: 'rgba(255,255,255,0.85)' }}>
                {value ? 'On — hotkey active' : 'Off'}
              </span>
            </label>
          )}
        />
      </Section>

      <Section heading="Active model">
        {installed.length === 0 ? (
          <div style={{ fontSize: 12, color: 'rgba(255,255,255,0.55)' }}>
            No models installed yet. Install one below to enable dictation.
          </div>
        ) : (
          <SaveField
            section="voice"
            fieldKey="model"
            label="Whisper model"
            helper={configHelp('voice', 'model')}
            initialValue={config.voice.model}
            resyncToken={resyncToken}
            onSaved={onSaved}
            render={(value, setValue) => (
              <select
                value={value}
                onChange={(e) => setValue(e.target.value)}
                aria-label="Active whisper model"
                style={{
                  padding: '6px 10px',
                  borderRadius: 5,
                  border: '1px solid rgba(255,255,255,0.18)',
                  background: 'rgba(0,0,0,0.3)',
                  color: 'rgba(255,255,255,0.92)',
                  fontSize: 13,
                  outline: 'none',
                  minWidth: 280,
                }}
              >
                <option value="">— No model selected —</option>
                {installed.map((m) => (
                  <option key={m.filename} value={m.filename}>
                    {m.filename} ({fmtBytes(m.size)})
                  </option>
                ))}
              </select>
            )}
          />
        )}
      </Section>

      <Section heading="Install a model">
        <WhisperModelInstaller
          excludeFilenames={installedFilenames}
          onInstalled={handleInstalled}
        />
      </Section>

      {installed.length > 0 ? (
        <Section heading="Installed models">
          <SettingRow label="On disk">
            <ul
              style={{
                listStyle: 'none',
                padding: 0,
                margin: 0,
                display: 'flex',
                flexDirection: 'column',
                gap: 6,
                width: '100%',
              }}
            >
              {installed.map((m) => {
                const isActive = m.filename === config.voice.model;
                const confirming = confirmDeleteName === m.filename;
                return (
                  <li
                    key={m.filename}
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: 8,
                      padding: '6px 8px',
                      background: 'rgba(255,255,255,0.03)',
                      borderRadius: 5,
                      borderLeft: isActive
                        ? '3px solid #d4af37'
                        : '3px solid transparent',
                    }}
                  >
                    <span
                      style={{
                        flex: 1,
                        fontFamily: 'monospace',
                        fontSize: 12,
                        color: 'rgba(255,255,255,0.92)',
                      }}
                    >
                      {m.filename}
                      {isActive ? (
                        <span
                          style={{
                            marginLeft: 8,
                            fontSize: 10,
                            color: '#d4af37',
                            textTransform: 'uppercase',
                            letterSpacing: 0.5,
                          }}
                        >
                          active
                        </span>
                      ) : null}
                    </span>
                    <span
                      style={{
                        fontSize: 11,
                        color: 'rgba(255,255,255,0.55)',
                        minWidth: 70,
                        textAlign: 'right',
                      }}
                    >
                      {fmtBytes(m.size)}
                    </span>
                    {confirming ? (
                      <>
                        <button
                          type="button"
                          onClick={() => void handleDelete(m.filename)}
                          style={{
                            padding: '4px 10px',
                            background: '#e07070',
                            color: '#0c0c0d',
                            border: 'none',
                            borderRadius: 4,
                            fontSize: 12,
                            cursor: 'pointer',
                            fontWeight: 600,
                          }}
                          aria-label={`Confirm delete ${m.filename}`}
                        >
                          Delete
                        </button>
                        <button
                          type="button"
                          onClick={() => setConfirmDeleteName(null)}
                          style={{
                            padding: '4px 10px',
                            background: 'transparent',
                            color: 'rgba(255,255,255,0.7)',
                            border: '1px solid rgba(255,255,255,0.2)',
                            borderRadius: 4,
                            fontSize: 12,
                            cursor: 'pointer',
                          }}
                        >
                          Cancel
                        </button>
                      </>
                    ) : (
                      <button
                        type="button"
                        onClick={() => setConfirmDeleteName(m.filename)}
                        aria-label={`Delete ${m.filename}`}
                        style={{
                          padding: '4px 8px',
                          background: 'transparent',
                          color: 'rgba(255,255,255,0.6)',
                          border: '1px solid rgba(255,255,255,0.18)',
                          borderRadius: 4,
                          cursor: 'pointer',
                          display: 'inline-flex',
                          alignItems: 'center',
                          justifyContent: 'center',
                        }}
                      >
                        <svg
                          width="13"
                          height="13"
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          strokeWidth="2"
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          aria-hidden
                        >
                          <polyline points="3 6 5 6 21 6" />
                          <path d="M19 6l-1 14a2 2 0 01-2 2H8a2 2 0 01-2-2L5 6" />
                          <path d="M10 11v6" />
                          <path d="M14 11v6" />
                          <path d="M9 6V4a2 2 0 012-2h2a2 2 0 012 2v2" />
                        </svg>
                      </button>
                    )}
                  </li>
                );
              })}
            </ul>
          </SettingRow>
        </Section>
      ) : null}

      <Section heading="Text-to-speech">
        <div
          style={{
            marginBottom: 8,
            fontSize: 12,
            color: 'rgba(255,255,255,0.6)',
            lineHeight: 1.5,
          }}
        >
          When enabled, Wren reads completed responses aloud through Windows
          SAPI. Cancel a generation to stop speech mid-sentence.
        </div>
        <SaveField
          section="voice"
          fieldKey="tts_enabled"
          label="Speak responses"
          helper={configHelp('voice', 'tts_enabled')}
          initialValue={config.voice.tts_enabled}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <label
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                gap: 8,
                cursor: 'pointer',
              }}
            >
              <input
                type="checkbox"
                checked={value}
                onChange={(e) => setValue(e.target.checked)}
                aria-label="Enable text-to-speech"
              />
              <span style={{ fontSize: 13, color: 'rgba(255,255,255,0.85)' }}>
                {value ? 'On — responses are spoken' : 'Off'}
              </span>
            </label>
          )}
        />

        <SaveField
          section="voice"
          fieldKey="tts_voice"
          label="Voice"
          helper={configHelp('voice', 'tts_voice')}
          initialValue={config.voice.tts_voice}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <select
              value={value}
              onChange={(e) => setValue(e.target.value)}
              aria-label="SAPI voice"
              style={{
                padding: '6px 10px',
                borderRadius: 5,
                border: '1px solid rgba(255,255,255,0.18)',
                background: 'rgba(0,0,0,0.3)',
                color: 'rgba(255,255,255,0.92)',
                fontSize: 13,
                outline: 'none',
                minWidth: 280,
              }}
            >
              <option value="">— System default —</option>
              {sapiVoices.map((v) => (
                <option key={v.name} value={v.name}>
                  {v.name}
                  {v.culture ? ` (${v.culture})` : ''}
                </option>
              ))}
              {/* If the user already has a saved voice that isn't in the
                  current install (renamed, removed, or another machine),
                  surface it so the dropdown faithfully reflects what's in
                  config rather than silently snapping to system default. */}
              {value && !sapiVoices.some((v) => v.name === value) ? (
                <option value={value}>
                  {value} (not installed on this PC)
                </option>
              ) : null}
            </select>
          )}
        />
        {sapiVoicesError ? (
          <div
            style={{
              marginTop: 4,
              fontSize: 11,
              color: '#e07070',
            }}
          >
            {sapiVoicesError}
          </div>
        ) : null}

        <SaveField
          section="voice"
          fieldKey="tts_rate"
          label="Speed"
          helper={configHelp('voice', 'tts_rate')}
          initialValue={config.voice.tts_rate}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <div style={{ display: 'inline-flex', alignItems: 'center', gap: 10 }}>
              <input
                type="range"
                min={-10}
                max={10}
                step={1}
                value={value}
                onChange={(e) => setValue(Number(e.target.value))}
                aria-label="Speech rate"
                style={{ width: 200 }}
              />
              <span
                style={{
                  fontSize: 12,
                  color: 'rgba(255,255,255,0.7)',
                  minWidth: 30,
                  textAlign: 'right',
                }}
              >
                {value > 0 ? `+${value}` : value}
              </span>
            </div>
          )}
        />
      </Section>
    </>
  );
}

const kbdStyle: React.CSSProperties = {
  padding: '1px 5px',
  background: 'rgba(255,255,255,0.08)',
  border: '1px solid rgba(255,255,255,0.15)',
  borderRadius: 3,
  fontFamily: 'monospace',
  fontSize: 11,
};
