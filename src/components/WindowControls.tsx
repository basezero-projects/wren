/**
 * Window controls — Wren (Windows port of Wren).
 *
 * Original Wren design used macOS-style traffic lights (red/yellow/green
 * dots) on the left. Windows users expect window controls on the right
 * with a proper × close button, so we drop the dots and put a square
 * close button at the right edge of the toolbar.
 *
 * The empty left side of the bar still acts as the drag region (mousedown
 * bubbles up to the App root which calls startDragging).
 */

import { memo } from 'react';
import { Tooltip } from './Tooltip';

const BOOKMARK_ICON_EMPTY = (
  <svg
    width="13"
    height="13"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden="true"
  >
    <path d="M19 21l-7-5-7 5V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z" />
  </svg>
);

const BOOKMARK_ICON_FILLED = (
  <svg
    width="13"
    height="13"
    viewBox="0 0 24 24"
    fill="currentColor"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden="true"
  >
    <path d="M19 21l-7-5-7 5V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z" />
  </svg>
);

const NEW_CONVERSATION_ICON = (
  <svg
    width="13"
    height="13"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden="true"
  >
    <line x1="12" y1="5" x2="12" y2="19" />
    <line x1="5" y1="12" x2="19" y2="12" />
  </svg>
);

const CHIP_ICON = (
  <svg
    width="13"
    height="13"
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.5"
    strokeLinecap="round"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
  >
    <rect x="3" y="3" width="10" height="10" rx="1.5" />
    <path d="M5 1V3M8 1V3M11 1V3M5 13V15M8 13V15M11 13V15M1 5H3M1 8H3M1 11H3M13 5H15M13 8H15M13 11H15" />
  </svg>
);

const HISTORY_ICON = (
  <svg
    width="13"
    height="13"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden="true"
  >
    <circle cx="12" cy="12" r="10" />
    <polyline points="12 6 12 12 16 14" />
  </svg>
);

const CLOSE_ICON = (
  <svg
    width="11"
    height="11"
    viewBox="0 0 12 12"
    aria-hidden="true"
  >
    <path
      d="M1 1L11 11M11 1L1 11"
      stroke="currentColor"
      strokeWidth="1.4"
      strokeLinecap="round"
    />
  </svg>
);

interface WindowControlsProps {
  onClose: () => void;
  onSave?: () => void;
  isSaved?: boolean;
  canSave?: boolean;
  onHistoryOpen?: () => void;
  onNewConversation?: () => void;
  activeModel?: string | null;
  onModelPickerToggle?: () => void;
  isModelPickerOpen?: boolean;
}

export const WindowControls = memo(function WindowControls({
  onClose,
  onSave,
  isSaved = false,
  canSave = false,
  onHistoryOpen,
  onNewConversation,
  activeModel,
  onModelPickerToggle,
  isModelPickerOpen = false,
}: WindowControlsProps) {
  const saveDisabled = !isSaved && !canSave;

  return (
    <div className="shrink-0">
      <div className="group flex items-center px-3 py-2 min-h-[36px]">
        {/* Left toolbar: model picker, save, new conv, history. */}
        <div className="flex items-center gap-1">
          {onModelPickerToggle !== undefined && (
            <Tooltip label="Choose model">
              <button
                type="button"
                aria-label="Choose model"
                aria-expanded={isModelPickerOpen}
                aria-haspopup="listbox"
                data-model-picker-toggle
                onClick={onModelPickerToggle}
                className={`group/pill flex items-center gap-1.5 px-2 h-7 rounded-lg text-xs transition-colors duration-150 cursor-pointer ${
                  isModelPickerOpen ? 'bg-primary/10' : 'hover:bg-primary/8'
                }`}
              >
                <span
                  className={`shrink-0 transition-colors duration-150 ${
                    isModelPickerOpen
                      ? 'text-primary'
                      : 'text-text-secondary group-hover/pill:text-primary'
                  }`}
                >
                  {CHIP_ICON}
                </span>
                <span
                  className={`max-w-[120px] truncate transition-colors duration-150 ${
                    isModelPickerOpen
                      ? 'text-text-primary'
                      : 'text-text-secondary group-hover/pill:text-text-primary'
                  }`}
                >
                  {activeModel != null && activeModel.length > 0
                    ? activeModel
                    : 'Pick a model'}
                </span>
              </button>
            </Tooltip>
          )}

          {onSave !== undefined && (
            <Tooltip
              label={isSaved ? 'Remove from history' : 'Save conversation'}
            >
              <button
                type="button"
                onClick={onSave}
                disabled={saveDisabled}
                aria-label={
                  isSaved ? 'Remove from history' : 'Save conversation'
                }
                className={`w-7 h-7 flex items-center justify-center rounded-lg transition-colors duration-150 cursor-pointer disabled:cursor-default ${
                  isSaved
                    ? 'text-primary hover:text-text-secondary hover:bg-white/5'
                    : canSave
                      ? 'text-text-secondary hover:text-primary hover:bg-primary/8'
                      : 'text-text-secondary opacity-30'
                }`}
              >
                {isSaved ? BOOKMARK_ICON_FILLED : BOOKMARK_ICON_EMPTY}
              </button>
            </Tooltip>
          )}

          {onNewConversation !== undefined && (
            <Tooltip label="New conversation">
              <button
                type="button"
                onClick={onNewConversation}
                aria-label="New conversation"
                data-history-toggle
                className="w-7 h-7 flex items-center justify-center rounded-lg text-text-secondary hover:text-primary hover:bg-primary/8 transition-colors duration-150 cursor-pointer"
              >
                {NEW_CONVERSATION_ICON}
              </button>
            </Tooltip>
          )}

          {onHistoryOpen !== undefined && (
            <Tooltip label="Conversation history">
              <button
                type="button"
                onClick={onHistoryOpen}
                aria-label="Open history"
                data-history-toggle
                className="w-7 h-7 flex items-center justify-center rounded-lg text-text-secondary hover:text-primary hover:bg-primary/8 transition-colors duration-150 cursor-pointer"
              >
                {HISTORY_ICON}
              </button>
            </Tooltip>
          )}
        </div>

        {/* Drag region between toolbar and close button. */}
        <div className="flex-1" aria-hidden="true" />

        {/* Close button — Windows convention puts it at the far right. */}
        <Tooltip label="Hide">
          <button
            type="button"
            onClick={onClose}
            onFocus={(e) => {
              if (e.relatedTarget === null) e.currentTarget.blur();
            }}
            aria-label="Hide window"
            className="w-8 h-7 flex items-center justify-center rounded-lg text-text-secondary hover:text-white hover:bg-[#e81123] transition-colors duration-150 cursor-pointer"
          >
            {CLOSE_ICON}
          </button>
        </Tooltip>
      </div>

      <div className="h-px bg-surface-border" />
    </div>
  );
});
