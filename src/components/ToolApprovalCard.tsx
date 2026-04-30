import { useMemo, useState } from 'react';
import type { ToolApproval } from '../hooks/useOllama';

interface ToolApprovalCardProps {
  approval: ToolApproval;
  onDecide: (id: string, allowed: boolean) => void | Promise<void>;
}

/**
 * Inline card the assistant emits when it wants to run a destructive
 * tool.
 *
 * **Pending state** — full JSON arguments shown verbatim with Allow /
 * Deny buttons. The user is consenting to exactly what they see, so
 * the args are not summarized.
 *
 * **Resolved state** — the result line is the prominent first thing
 * after the header (since that is what the user actually wants to
 * know once they have made a decision). The JSON details collapse
 * behind a small Details disclosure so they are still inspectable
 * without dominating the bubble.
 */
export function ToolApprovalCard({ approval, onDecide }: ToolApprovalCardProps) {
  const [busy, setBusy] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const prettyArgs = useMemo(
    () => prettyJson(approval.argumentsJson),
    [approval.argumentsJson],
  );

  const click = async (allowed: boolean) => {
    if (busy || approval.status !== 'pending') return;
    setBusy(true);
    try {
      await onDecide(approval.id, allowed);
    } finally {
      setBusy(false);
    }
  };

  const isPending = approval.status === 'pending';
  const isAllowed = approval.status === 'allowed';
  const hasResult = approval.resultSummary !== undefined;

  // Border tint: gold for pending, green for allowed-with-good-result,
  // red for any error result, grey for terminal-not-run states.
  const borderColor = isPending
    ? 'rgba(212, 175, 55, 0.45)'
    : approval.resultOk === false
      ? 'rgba(224, 112, 112, 0.45)'
      : isAllowed
        ? 'rgba(92, 201, 126, 0.4)'
        : 'rgba(160,160,160,0.3)';
  const tintBg = isPending
    ? 'rgba(212, 175, 55, 0.06)'
    : approval.resultOk === false
      ? 'rgba(224, 112, 112, 0.05)'
      : isAllowed
        ? 'rgba(92, 201, 126, 0.05)'
        : 'rgba(160,160,160,0.04)';

  return (
    <div
      role="group"
      aria-label="Tool approval request"
      style={{
        margin: '8px 0',
        border: `1px solid ${borderColor}`,
        borderRadius: 8,
        background: tintBg,
        padding: '10px 12px',
        fontSize: 13,
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          marginBottom: hasResult || isPending ? 8 : 0,
          gap: 8,
        }}
      >
        <div style={{ fontWeight: 600 }}>
          {isPending ? 'Wren wants to run' : 'Wren'}{' '}
          <code
            style={{
              padding: '1px 6px',
              borderRadius: 4,
              background: 'rgba(212, 175, 55, 0.18)',
              color: '#f0d989',
              fontSize: 12,
            }}
          >
            {approval.name}
          </code>
        </div>
        <StatusBadge status={approval.status} />
      </div>

      {/* Pending: show JSON args front and center — that is what the
          user is consenting to. */}
      {isPending && (
        <pre
          style={{
            margin: 0,
            padding: '8px 10px',
            background: 'rgba(0, 0, 0, 0.35)',
            borderRadius: 6,
            fontSize: 12,
            lineHeight: 1.4,
            whiteSpace: 'pre-wrap',
            wordBreak: 'break-word',
            maxHeight: 240,
            overflow: 'auto',
          }}
        >
          {prettyArgs}
        </pre>
      )}

      {/* Resolved with a result: result line is the prominent thing. */}
      {hasResult && (
        <div
          style={{
            display: 'flex',
            alignItems: 'flex-start',
            gap: 10,
            padding: '8px 10px',
            borderLeft: `3px solid ${
              approval.resultOk ? '#5cc97e' : '#e07070'
            }`,
            background: 'rgba(255,255,255,0.04)',
            borderRadius: 4,
            fontSize: 13,
            lineHeight: 1.45,
          }}
        >
          <span
            aria-hidden="true"
            style={{
              fontSize: 16,
              lineHeight: 1,
              color: approval.resultOk ? '#7ed09a' : '#e88a8a',
            }}
          >
            {approval.resultOk ? '✓' : '✗'}
          </span>
          <div
            style={{
              flex: 1,
              color: 'rgba(255,255,255,0.92)',
              wordBreak: 'break-word',
            }}
          >
            {approval.resultSummary}
          </div>
        </div>
      )}

      {isPending && (
        <div style={{ display: 'flex', gap: 8, marginTop: 10 }}>
          <button
            type="button"
            onClick={() => click(true)}
            disabled={busy}
            style={{
              padding: '6px 14px',
              background: '#d4af37',
              color: '#0c0c0d',
              border: 'none',
              borderRadius: 5,
              fontWeight: 600,
              cursor: busy ? 'wait' : 'pointer',
              fontSize: 13,
            }}
          >
            Allow
          </button>
          <button
            type="button"
            onClick={() => click(false)}
            disabled={busy}
            style={{
              padding: '6px 14px',
              background: 'transparent',
              color: 'rgba(255,255,255,0.85)',
              border: '1px solid rgba(255,255,255,0.25)',
              borderRadius: 5,
              cursor: busy ? 'wait' : 'pointer',
              fontSize: 13,
            }}
          >
            Deny
          </button>
        </div>
      )}

      {/* Resolved: JSON details tuck behind a disclosure so the bubble
          stays compact. The user can still inspect what they consented
          to after the fact. */}
      {!isPending && (
        <details
          style={{ marginTop: 8 }}
          open={detailsOpen}
          onToggle={(e) => setDetailsOpen((e.target as HTMLDetailsElement).open)}
        >
          <summary
            style={{
              cursor: 'pointer',
              listStyle: 'none',
              fontSize: 11,
              color: 'rgba(255,255,255,0.55)',
              userSelect: 'none',
              outline: 'none',
            }}
          >
            {detailsOpen ? '▾ Hide arguments' : '▸ Show arguments'}
          </summary>
          <pre
            style={{
              marginTop: 6,
              marginBottom: 0,
              padding: '8px 10px',
              background: 'rgba(0, 0, 0, 0.35)',
              borderRadius: 6,
              fontSize: 12,
              lineHeight: 1.4,
              whiteSpace: 'pre-wrap',
              wordBreak: 'break-word',
              maxHeight: 240,
              overflow: 'auto',
            }}
          >
            {prettyArgs}
          </pre>
        </details>
      )}
    </div>
  );
}

function StatusBadge({ status }: { status: ToolApproval['status'] }) {
  const config = STATUS_BADGE_CONFIG[status];
  return (
    <span
      style={{
        fontSize: 11,
        padding: '2px 8px',
        borderRadius: 999,
        background: config.bg,
        color: config.fg,
        textTransform: 'uppercase',
        letterSpacing: 0.5,
        whiteSpace: 'nowrap',
      }}
    >
      {config.label}
    </span>
  );
}

const STATUS_BADGE_CONFIG: Record<
  ToolApproval['status'],
  { label: string; bg: string; fg: string }
> = {
  pending: {
    label: 'Awaiting approval',
    bg: 'rgba(212, 175, 55, 0.2)',
    fg: '#f0d989',
  },
  allowed: {
    label: 'Allowed',
    bg: 'rgba(60,180,90,0.18)',
    fg: '#7ed09a',
  },
  denied: {
    label: 'Denied',
    bg: 'rgba(220,80,80,0.18)',
    fg: '#e88a8a',
  },
  expired: {
    label: 'Expired — not run',
    bg: 'rgba(160,160,160,0.2)',
    fg: '#cfcfcf',
  },
  cancelled: {
    label: 'Cancelled — not run',
    bg: 'rgba(160,160,160,0.2)',
    fg: '#cfcfcf',
  },
  timed_out: {
    label: 'Timed out',
    bg: 'rgba(160,160,160,0.2)',
    fg: '#cfcfcf',
  },
};

function prettyJson(raw: string): string {
  try {
    return JSON.stringify(JSON.parse(raw), null, 2);
  } catch {
    return raw;
  }
}
