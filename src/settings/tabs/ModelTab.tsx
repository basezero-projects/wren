/**
 * AI tab.
 *
 * Holds the local Ollama endpoint and the custom system prompt — the two
 * AI-shaped knobs that persist to TOML. The active model picker lives in
 * the main app overlay (see ModelPickerPanel) since model selection is
 * runtime UI state owned by ActiveModelState in the backend, not a
 * TOML-persisted field. The Window/Quote knobs live in the Display tab.
 */

import { Section, TextField, Textarea } from '../components';
import { SaveField } from '../components/SaveField';
import { ModelPullField } from '../components/ModelPullField';
import { configHelp } from '../configHelpers';
import styles from '../../styles/settings.module.css';
import type { RawAppConfig } from '../types';

interface ModelTabProps {
  config: RawAppConfig;
  resyncToken: number;
  onSaved: (next: RawAppConfig) => void;
}

const PROMPT_MAX_CHARS = 8000;

export function ModelTab({ config, resyncToken, onSaved }: ModelTabProps) {
  return (
    <>
      <Section heading="Ollama">
        <SaveField
          section="inference"
          fieldKey="ollama_url"
          label="Ollama URL"
          helper={configHelp('inference', 'ollama_url')}
          initialValue={config.inference.ollama_url}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue, errored) => (
            <TextField
              value={value}
              onChange={setValue}
              placeholder="http://127.0.0.1:11434"
              errored={errored}
              ariaLabel="Ollama URL"
            />
          )}
        />
        <SaveField
          section="inference"
          fieldKey="tool_model"
          label="Tool model"
          helper={configHelp('inference', 'tool_model')}
          initialValue={config.inference.tool_model}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue, errored) => (
            <TextField
              value={value}
              onChange={setValue}
              placeholder="leave empty to use your chat model"
              errored={errored}
              ariaLabel="Tool model"
            />
          )}
        />
      </Section>

      <Section heading="Install a model">
        <div style={{ marginBottom: 8, fontSize: 12, color: 'rgba(255,255,255,0.6)', lineHeight: 1.5 }}>
          Pulls a model into your local Ollama, the same way <code>ollama pull</code> does in a terminal. Two slug formats work:
          <ul style={{ margin: '6px 0 0 16px', padding: 0 }}>
            <li>
              Ollama library — <code>qwen3:8b</code>, <code>qwen2.5vl:7b</code>, <code>gemma3:12b</code>, anything from <a href="https://ollama.com/library" target="_blank" rel="noreferrer">ollama.com/library</a>.
            </li>
            <li>
              HuggingFace GGUF — <code>hf.co/&lt;owner&gt;/&lt;repo&gt;:&lt;quant&gt;</code>, e.g. <code>hf.co/bartowski/Qwen2.5-14B-Instruct-GGUF:Q4_K_M</code>. Browse <a href="https://huggingface.co/models?library=gguf" target="_blank" rel="noreferrer">HF GGUF models</a>.
            </li>
          </ul>
        </div>
        <ModelPullField />
      </Section>

      <Section heading="Prompt">
        <SaveField
          section="prompt"
          fieldKey="system"
          label="System prompt"
          helper={configHelp('prompt', 'system')}
          vertical
          initialValue={config.prompt.system}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <>
              <Textarea
                value={value}
                onChange={setValue}
                placeholder="Use built-in secretary persona…"
                maxLength={PROMPT_MAX_CHARS}
                ariaLabel="System prompt"
              />
              <div className={styles.charCounter}>
                {value.length} / {PROMPT_MAX_CHARS}
              </div>
            </>
          )}
        />
      </Section>
    </>
  );
}
