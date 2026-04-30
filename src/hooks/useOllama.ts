import { useCallback, useRef, useState } from 'react';
import { Channel, invoke } from '@tauri-apps/api/core';
import type {
  SearchEvent,
  SearchMetadata,
  SearchResultPreview,
  SearchStage,
  SearchTraceStep,
  SearchWarning,
} from '../types/search';

/** Mirrors the Rust OllamaErrorKind enum sent over IPC. */
export type OllamaErrorKind =
  | 'NotRunning'
  | 'ModelNotFound'
  | 'NoModelSelected'
  | 'Other';

/** A single destructive-tool call awaiting (or having already received)
 *  user approval. Rendered as an inline card inside the assistant bubble. */
export interface ToolApproval {
  /** UUID echoed back to the Rust side via `approve_tool_call`. */
  id: string;
  /** Tool catalog name, e.g. `write_file`. */
  name: string;
  /** Pretty-printed JSON arguments — exact text the user is consenting to. */
  argumentsJson: string;
  /** UI state of the card.
   *  - `pending`: Allow/Deny buttons are showing
   *  - `allowed`: user clicked Allow AND the backend confirmed dispatch
   *  - `denied`: user clicked Deny
   *  - `expired`: user clicked Allow but the backend had already cleaned up
   *               the pending entry (e.g. the generation was cancelled).
   *               No tool ran; the badge tells the truth.
   *  - `cancelled`: the whole generation was cancelled while this card
   *                 was still pending. No tool ran.
   *  - `timed_out`: the 5-minute server-side approval timer fired before
   *                 the user clicked anything. No tool ran. */
  status:
    | 'pending'
    | 'allowed'
    | 'denied'
    | 'expired'
    | 'cancelled'
    | 'timed_out';
  /** Optional first-line summary of the tool's output, populated when the
   *  backend emits ToolResult. Lets the user see what the tool actually
   *  did instead of guessing from the badge. */
  resultSummary?: string;
  /** True when the tool dispatched without error. */
  resultOk?: boolean;
}

/** Represents a single message in the chat thread. */
export interface Message {
  /** Unique identifier for stable React list keys. */
  id: string;
  role: 'user' | 'assistant';
  content: string;
  /** Ollama model slug attributed to this assistant message at creation time.
   *  Remains stable even if the user switches models mid-stream. Undefined for
   *  user messages and for legacy conversations loaded from pre-migration rows. */
  modelName?: string;
  /** Selected text from the host app that was quoted with this message, if any. */
  quotedText?: string;
  /** Absolute file paths of images attached to this message, if any. */
  imagePaths?: string[];
  /** Present on assistant messages that represent an Ollama error callout. */
  errorKind?: OllamaErrorKind;
  /** Accumulated thinking content from the model, if thinking mode was used. */
  thinkingContent?: string;
  /** Marks an assistant message produced through the `/search` pipeline. */
  fromSearch?: boolean;
  /** Marks an assistant message produced through a `/think` turn. */
  fromThink?: boolean;
  /** Source links forwarded by the search pipeline. */
  searchSources?: SearchResultPreview[];
  /** Warnings emitted by the `/search` pipeline during this turn. */
  searchWarnings?: SearchWarning[];
  /** When true, renders sandbox setup guidance instead of normal content. */
  sandboxUnavailable?: boolean;
  /** Ordered, user-facing timeline steps for a `/search` turn. */
  searchTraces?: SearchTraceStep[];
  /** Structured retrieval metadata emitted by the backend search pipeline. */
  searchMetadata?: SearchMetadata;
  /** In-flight or resolved destructive-tool approval requests for this turn. */
  toolApprovals?: ToolApproval[];
}

/** Raw streaming chunk payload emitted from the Rust chat backend. */
type RawStreamChunk =
  | { type: 'Token'; data: string }
  | { type: 'ThinkingToken'; data: string }
  | { type: 'Done' }
  | { type: 'Cancelled' }
  | { type: 'Error'; data: { kind: OllamaErrorKind; message: string } }
  | {
      type: 'ToolApprovalRequest';
      data: { id: string; name: string; arguments_json: string };
    }
  | {
      type: 'ToolResult';
      data: { id: string; name: string; ok: boolean; summary: string };
    };

/**
 * Normalized chat-stream chunk used inside the hook.
 *
 * The chat IPC payload uses `data` while the search pipeline uses `content`.
 * Normalizing here keeps the internal token contract consistent and prevents
 * accidental cross-assignment between the two event streams.
 */
type StreamChunk =
  | { type: 'Token'; content: string }
  | { type: 'ThinkingToken'; content: string }
  | { type: 'Done' }
  | { type: 'Cancelled' }
  | { type: 'Error'; error: { kind: OllamaErrorKind; message: string } }
  | { type: 'ToolApprovalRequest'; approval: ToolApproval }
  | {
      type: 'ToolResult';
      result: { id: string; name: string; ok: boolean; summary: string };
    };

function normalizeStreamChunk(chunk: RawStreamChunk): StreamChunk {
  switch (chunk.type) {
    case 'Token':
      return { type: 'Token', content: chunk.data };
    case 'ThinkingToken':
      return { type: 'ThinkingToken', content: chunk.data };
    case 'Done':
      return chunk;
    case 'Cancelled':
      return chunk;
    case 'Error':
      return { type: 'Error', error: chunk.data };
    case 'ToolApprovalRequest':
      return {
        type: 'ToolApprovalRequest',
        approval: {
          id: chunk.data.id,
          name: chunk.data.name,
          argumentsJson: chunk.data.arguments_json,
          status: 'pending',
        },
      };
    case 'ToolResult':
      return { type: 'ToolResult', result: chunk.data };
  }
}

/** Result payload delivered to callers when a `/search` pipeline turn finishes. */
export interface SearchOutcome {
  final: boolean;
}

interface ActiveGeneration {
  id: number;
  assistantId: string;
  hasVisibleOutput: boolean;
  resolveSearch?: (outcome: SearchOutcome) => void;
}

function upsertSearchTraceStep(
  steps: SearchTraceStep[],
  nextStep: SearchTraceStep,
): SearchTraceStep[] {
  const index = steps.findIndex((step) => step.id === nextStep.id);
  if (index === -1) {
    return [...steps, nextStep];
  }

  const next = [...steps];
  next[index] = nextStep;
  return next;
}

function finalizeSearchTraceSteps(
  steps: SearchTraceStep[],
): SearchTraceStep[] | undefined {
  if (steps.length === 0) return undefined;

  return steps.map((step) =>
    step.status === 'running' ? { ...step, status: 'completed' } : step,
  );
}

/**
 * Simplifies interactions with the local Ollama backend.
 *
 * Manages message history, streaming state, and the Tauri IPC channels used by
 * both the normal chat path and the `/search` pipeline.
 *
 * @param activeModel Ollama model slug that should be attributed to each
 *   assistant message produced by this hook. Passed as a hook parameter (not
 *   a per-call argument) so the latest App-level selection is captured via
 *   closure on every render. `null` (no model selected) and an empty string
 *   are both coerced to `undefined` on the emitted `Message`, so no
 *   attribution chip is rendered rather than a blank one.
 * @param onTurnComplete Optional callback invoked after each completed turn.
 */
export function useOllama(
  activeModel: string | null,
  onTurnComplete?: (userMsg: Message, assistantMsg: Message) => void,
) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [isGenerating, setIsGenerating] = useState(false);
  /** Transient stage indicator for the active `/search` pipeline, if any. */
  const [searchStage, setSearchStage] = useState<SearchStage>(null);
  const activeGenerationRef = useRef<ActiveGeneration | null>(null);
  const nextGenerationIdRef = useRef(0);
  const pendingCancelRef = useRef<Promise<void> | null>(null);

  const beginGeneration = (
    assistantId: string,
    resolveSearch?: (outcome: SearchOutcome) => void,
  ) => {
    const generation: ActiveGeneration = {
      id: nextGenerationIdRef.current + 1,
      assistantId,
      hasVisibleOutput: false,
      resolveSearch,
    };
    nextGenerationIdRef.current = generation.id;
    activeGenerationRef.current = generation;
    return generation.id;
  };

  const isActiveGeneration = (generationId: number) =>
    activeGenerationRef.current?.id === generationId;

  const markVisibleOutput = () => {
    activeGenerationRef.current!.hasVisibleOutput = true;
  };

  const completeGeneration = () => {
    const active = activeGenerationRef.current!;
    activeGenerationRef.current = null;
    return active;
  };

  const abortActiveGeneration = useCallback(() => {
    const active = activeGenerationRef.current;
    activeGenerationRef.current = null;
    setIsGenerating(false);
    setSearchStage(null);

    if (!active) {
      return false;
    }

    active.resolveSearch?.({ final: true });

    if (!active.hasVisibleOutput) {
      setMessages((prev) =>
        prev.filter((message) => message.id !== active.assistantId),
      );
    }

    return true;
  }, []);

  /**
   * Submits a message to the Ollama backend and starts the streaming response.
   *
   * The backend manages conversation history. Only the new user message is sent.
   */
  const ask = useCallback(
    async (
      displayContent: string,
      quotedText?: string,
      imagePaths?: string[],
      think?: boolean,
      promptOverride?: string,
    ) => {
      if (!displayContent.trim() && (!imagePaths || imagePaths.length === 0)) {
        return;
      }

      if (activeGenerationRef.current) return;
      const pendingCancel = pendingCancelRef.current;
      if (pendingCancel) {
        await pendingCancel;
      }
      if (activeGenerationRef.current) return;

      const userMsg: Message = {
        id: crypto.randomUUID(),
        role: 'user',
        content: displayContent,
        quotedText,
        imagePaths:
          imagePaths && imagePaths.length > 0 ? imagePaths : undefined,
      };

      const assistantId = crypto.randomUUID();
      const assistantMsg: Message = {
        id: assistantId,
        role: 'assistant',
        content: '',
        fromThink: think ? true : undefined,
        modelName: activeModel ?? undefined,
      };

      setMessages((prev) => [...prev, userMsg, assistantMsg]);
      setIsGenerating(true);
      const generationId = beginGeneration(assistantId);

      const channel = new Channel<RawStreamChunk>();
      let currentContent = '';
      let currentThinkingContent = '';

      // Frontend watchdog: if we go more than WATCHDOG_MS without any
      // chunk from Rust, surface an error in the assistant bubble. The
      // backend has its own server-side timeouts (per-chunk + total
      // request) so this only ever fires if the IPC channel itself
      // died — typically after a dev hot-reload or a backend crash.
      // 180 seconds covers a worst-case cold-load of an 8B Q4 model on
      // a busy machine plus a long thinking-mode generation. The
      // server-side request timeout is shorter (120s) so a real Ollama
      // hang produces a clean Error chunk before this fires; this only
      // wins when the channel itself is dead.
      const WATCHDOG_MS = 180_000;
      let watchdog: ReturnType<typeof setTimeout> | undefined;
      const armWatchdog = () => {
        if (watchdog) clearTimeout(watchdog);
        watchdog = setTimeout(() => {
          if (!isActiveGeneration(generationId)) return;
          completeGeneration();
          setMessages((prev) =>
            prev.map((message) =>
              message.id === assistantId
                ? {
                    ...message,
                    content: `Wren stopped hearing back from the backend\nNo response for ${WATCHDOG_MS / 1000} seconds. The dev server may have hot-reloaded, or the runner crashed. Cancel and try again.`,
                    errorKind: 'Other',
                  }
                : message,
            ),
          );
          setIsGenerating(false);
          setSearchStage(null);
        }, WATCHDOG_MS);
      };
      const disarmWatchdog = () => {
        if (watchdog) clearTimeout(watchdog);
        watchdog = undefined;
      };
      armWatchdog();

      channel.onmessage = (rawChunk) => {
        if (!isActiveGeneration(generationId)) {
          return;
        }
        // Any chunk = backend is alive. Reset the no-progress timer.
        armWatchdog();

        const chunk = normalizeStreamChunk(rawChunk);

        if (chunk.type === 'ThinkingToken') {
          currentThinkingContent += chunk.content;
          if (chunk.content) {
            markVisibleOutput();
          }
          setMessages((prev) =>
            prev.map((message) =>
              message.id === assistantId
                ? { ...message, thinkingContent: currentThinkingContent }
                : message,
            ),
          );
          return;
        }

        if (chunk.type === 'Token') {
          currentContent += chunk.content;
          if (chunk.content) {
            markVisibleOutput();
          }
          setMessages((prev) =>
            prev.map((message) =>
              message.id === assistantId
                ? { ...message, content: currentContent }
                : message,
            ),
          );
          return;
        }

        if (chunk.type === 'Done') {
          disarmWatchdog();
          completeGeneration();
          setIsGenerating(false);
          setSearchStage(null);
          onTurnComplete?.(userMsg, {
            ...assistantMsg,
            content: currentContent,
            thinkingContent: currentThinkingContent || undefined,
          });
          return;
        }

        if (chunk.type === 'Cancelled') {
          disarmWatchdog();
          completeGeneration();
          if (!currentContent && !currentThinkingContent) {
            // No visible content was produced. Drop the empty assistant
            // bubble entirely — including any pending approval cards
            // that came in before cancel landed. Otherwise an orphan
            // card would sit on screen, and clicking it would lie
            // about doing anything.
            setMessages((prev) =>
              prev.filter((message) => message.id !== assistantId),
            );
          } else {
            // Some content was streamed (thinking / tokens / cards).
            // Keep the bubble but mark every still-pending approval
            // as cancelled so the buttons disappear and the badge
            // reads truthfully.
            setMessages((prev) =>
              prev.map((message) =>
                message.id === assistantId && message.toolApprovals
                  ? {
                      ...message,
                      toolApprovals: message.toolApprovals.map((a) =>
                        a.status === 'pending'
                          ? { ...a, status: 'cancelled' }
                          : a,
                      ),
                    }
                  : message,
              ),
            );
          }
          setIsGenerating(false);
          setSearchStage(null);
          return;
        }

        if (chunk.type === 'ToolApprovalRequest') {
          markVisibleOutput();
          setMessages((prev) =>
            prev.map((message) =>
              message.id === assistantId
                ? {
                    ...message,
                    toolApprovals: [
                      ...(message.toolApprovals ?? []),
                      chunk.approval,
                    ],
                  }
                : message,
            ),
          );
          return;
        }

        if (chunk.type === 'ToolResult') {
          // Match by id when the tool was destructive (had an approval
          // card). Read-only tools emit results with an empty id; we
          // ignore those — the existing thinking-line trace is enough.
          if (!chunk.result.id) return;
          setMessages((prev) =>
            prev.map((message) =>
              message.id === assistantId && message.toolApprovals
                ? {
                    ...message,
                    toolApprovals: message.toolApprovals.map((a) =>
                      a.id === chunk.result.id
                        ? {
                            ...a,
                            resultOk: chunk.result.ok,
                            resultSummary: chunk.result.summary,
                          }
                        : a,
                    ),
                  }
                : message,
            ),
          );
          return;
        }

        disarmWatchdog();
        completeGeneration();

        setMessages((prev) =>
          prev.map((message) =>
            message.id === assistantId
              ? {
                  ...message,
                  content: chunk.error.message,
                  errorKind: chunk.error.kind,
                }
              : message,
          ),
        );
        setIsGenerating(false);
        setSearchStage(null);
      };

      try {
        await invoke('ask_ollama', {
          message: promptOverride ?? displayContent,
          quotedText: quotedText ?? null,
          imagePaths: imagePaths && imagePaths.length > 0 ? imagePaths : null,
          think: think ?? false,
          onEvent: channel,
        });
      } catch {
        disarmWatchdog();
        if (!isActiveGeneration(generationId)) {
          return;
        }
        completeGeneration();
        setMessages((prev) =>
          prev.map((message) =>
            message.id === assistantId
              ? {
                  ...message,
                  content: 'Something went wrong\nCould not reach Ollama.',
                  errorKind: 'Other',
                }
              : message,
          ),
        );
        setIsGenerating(false);
        setSearchStage(null);
      }
    },
    [onTurnComplete, activeModel],
  );

  /**
   * Submits a `/search` pipeline turn.
   *
   * @param query Text sent to the backend pipeline, without the `/search` trigger.
   * @param displayContent Text shown in the user bubble. Defaults to `query`.
   * @param quotedText Selected host-app text shown above the user bubble, if any.
   */
  const askSearch = useCallback(
    async (
      query: string,
      displayContent?: string,
      quotedText?: string,
    ): Promise<SearchOutcome> => {
      const trimmed = query.trim();
      if (!trimmed) return { final: true };

      if (activeGenerationRef.current) return { final: true };
      const pendingCancel = pendingCancelRef.current;
      if (pendingCancel) {
        await pendingCancel;
      }
      if (activeGenerationRef.current) return { final: true };

      const userMsg: Message = {
        id: crypto.randomUUID(),
        role: 'user',
        content: displayContent ?? trimmed,
        quotedText,
      };
      const assistantId = crypto.randomUUID();
      const assistantMsg: Message = {
        id: assistantId,
        role: 'assistant',
        content: '',
        fromSearch: true,
        modelName: activeModel ?? undefined,
      };

      setMessages((prev) => [...prev, userMsg, assistantMsg]);
      setIsGenerating(true);
      setSearchStage(null);

      const channel = new Channel<SearchEvent>();
      let currentContent = '';
      let sawToken = false;
      let pendingSources: SearchResultPreview[] | undefined;
      let warnings: SearchWarning[] = [];
      let pendingTraces: SearchTraceStep[] = [];
      let pendingMetadata: SearchMetadata | undefined;
      let awaitingClarification = false;
      let errored = false;
      let cancelled = false;

      const updateAssistant = (patch: Partial<Message>) => {
        setMessages((prev) =>
          prev.map((message) =>
            message.id === assistantId ? { ...message, ...patch } : message,
          ),
        );
      };

      return new Promise<SearchOutcome>((resolve) => {
        const generationId = beginGeneration(assistantId, resolve);

        const finish = (final: boolean) => {
          const active = completeGeneration();

          setIsGenerating(false);
          setSearchStage(null);

          const finalizedTraces = finalizeSearchTraceSteps(pendingTraces);
          if (finalizedTraces) {
            pendingTraces = finalizedTraces;
          }
          const persistedTraces = finalizedTraces;

          if (!errored && !cancelled && currentContent) {
            updateAssistant({
              searchSources: pendingSources,
              searchWarnings: warnings.length > 0 ? warnings : undefined,
              searchTraces: persistedTraces,
              searchMetadata: pendingMetadata,
            });
            onTurnComplete?.(userMsg, {
              ...assistantMsg,
              content: currentContent,
              searchSources: pendingSources,
              searchWarnings: warnings.length > 0 ? warnings : undefined,
              searchTraces: persistedTraces,
              searchMetadata: pendingMetadata,
            });
          }

          active.resolveSearch?.({ final });
        };

        // Once the backend emits RefiningSearch, every later searching or
        // reading stage belongs to a follow-up round rather than the initial one.
        let inGapRound = false;

        channel.onmessage = (event) => {
          if (!isActiveGeneration(generationId)) {
            return;
          }

          switch (event.type) {
            case 'Trace': {
              pendingTraces = upsertSearchTraceStep(pendingTraces, event.step);
              awaitingClarification ||= event.step.kind === 'clarify';
              updateAssistant({ searchTraces: pendingTraces });
              break;
            }
            case 'AnalyzingQuery': {
              setSearchStage({ kind: 'analyzing_query' });
              break;
            }
            case 'Searching': {
              setSearchStage(
                inGapRound
                  ? { kind: 'searching', gap: true }
                  : { kind: 'searching' },
              );
              break;
            }
            case 'FetchingUrl':
            case 'ReadingSources': {
              setSearchStage(
                inGapRound
                  ? { kind: 'reading_sources', gap: true }
                  : { kind: 'reading_sources' },
              );
              break;
            }
            case 'RefiningSearch': {
              inGapRound = true;
              setSearchStage({
                kind: 'refining_search',
                attempt: event.attempt,
                total: event.total,
              });
              break;
            }
            case 'Composing': {
              setSearchStage(
                inGapRound
                  ? { kind: 'composing', gap: true }
                  : { kind: 'composing' },
              );
              break;
            }
            case 'Sources': {
              pendingSources = event.results;
              break;
            }
            case 'Token': {
              sawToken ||= event.content.length > 0;
              currentContent += event.content;
              if (event.content) {
                markVisibleOutput();
              }
              setSearchStage(null);
              updateAssistant({ content: currentContent });
              break;
            }
            case 'IterationComplete': {
              const finalizedTraces = finalizeSearchTraceSteps(pendingTraces);
              if (finalizedTraces) {
                pendingTraces = finalizedTraces;
                updateAssistant({ searchTraces: finalizedTraces });
              }
              break;
            }
            case 'Warning': {
              warnings = [...warnings, event.warning];
              break;
            }
            case 'Done': {
              pendingMetadata = event.metadata ?? pendingMetadata;
              finish(!awaitingClarification && sawToken);
              break;
            }
            case 'Cancelled': {
              const active = completeGeneration();
              cancelled = true;
              if (!currentContent) {
                setMessages((prev) =>
                  prev.filter((message) => message.id !== assistantId),
                );
              }
              setIsGenerating(false);
              setSearchStage(null);
              active.resolveSearch?.({ final: true });
              break;
            }
            case 'Error': {
              errored = true;
              updateAssistant({
                content: event.message,
                errorKind: 'Other',
              });
              finish(true);
              break;
            }
            case 'SandboxUnavailable': {
              errored = true;
              updateAssistant({ sandboxUnavailable: true });
              finish(true);
              break;
            }
          }
        };

        invoke('search_pipeline', {
          message: trimmed,
          onEvent: channel,
        }).catch(() => {
          if (!isActiveGeneration(generationId) || errored || cancelled) return;
          errored = true;
          updateAssistant({
            content: 'Something went wrong\nCould not start search.',
            errorKind: 'Other',
          });
          finish(true);
        });
      });
    },
    [onTurnComplete, activeModel],
  );

  /** Cancels the currently active generation. */
  const cancel = useCallback(async () => {
    if (
      !activeGenerationRef.current &&
      !isGenerating &&
      !pendingCancelRef.current
    ) {
      return;
    }

    abortActiveGeneration();

    if (!pendingCancelRef.current) {
      const cancelPromise = (async () => {
        try {
          await invoke('cancel_generation');
        } catch {
          // Local hard-abort already reset the UI; backend best-effort only.
        } finally {
          pendingCancelRef.current = null;
        }
      })();
      pendingCancelRef.current = cancelPromise;
    }

    await pendingCancelRef.current;
  }, [abortActiveGeneration, isGenerating]);

  /** Resolves a destructive-tool approval card. Sends the user's decision
   *  to the Rust side via `approve_tool_call` and updates the matching
   *  card based on what the backend reports. The Rust command returns
   *  true when a matching pending entry was found and signalled, false
   *  when the entry had already been cleaned up (e.g. the generation
   *  was cancelled before the user clicked). The card status reflects
   *  what actually happened, not what the user wished had happened. */
  const approveToolCall = useCallback(
    async (id: string, allowed: boolean) => {
      let resolvedByBackend = false;
      try {
        resolvedByBackend = (await invoke('approve_tool_call', {
          id,
          allowed,
        })) as boolean;
      } catch {
        // Treat an invoke failure (Tauri layer) the same as a stale id:
        // we cannot prove the tool dispatched, so do not claim it did.
        resolvedByBackend = false;
      }
      setMessages((prev) =>
        prev.map((message) =>
          message.toolApprovals?.some((a) => a.id === id)
            ? {
                ...message,
                toolApprovals: message.toolApprovals.map((a) =>
                  a.id === id
                    ? {
                        ...a,
                        status: !resolvedByBackend
                          ? 'expired'
                          : allowed
                            ? 'allowed'
                            : 'denied',
                      }
                    : a,
                ),
              }
            : message,
        ),
      );
    },
    [],
  );

  /** Resets all conversation state for a fresh session. */
  const reset = useCallback(() => {
    abortActiveGeneration();
    setMessages([]);
    void invoke('reset_conversation');
  }, [abortActiveGeneration]);

  /** Replaces the current message list with a previously loaded set of messages. */
  const loadMessages = useCallback(
    (msgs: Message[]) => {
      abortActiveGeneration();
      setMessages(msgs);
    },
    [abortActiveGeneration],
  );

  return {
    messages,
    ask,
    askSearch,
    cancel,
    isGenerating,
    searchStage,
    reset,
    loadMessages,
    approveToolCall,
  };
}
