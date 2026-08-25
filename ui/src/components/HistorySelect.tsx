import { Badge, Button, Checkbox, Group, Popover, Text, Tooltip } from '@mantine/core'
import { IconChevronDown } from '@tabler/icons-react'
import type { IssueCommit, IssueStatusResponse } from '~/api/issues'
import { approvalCommentUrl } from '~/api/issues'
import type { HistorySegment } from '~/utils/history'
import { describeCommits } from '~/utils/history'
import { FetchBranchBadge, InheritedBranchBadge, NoCohesiveHistoryBadge } from './RoundBadges'

interface Props {
  status: IssueStatusResponse
  history: HistorySegment[]
  selected: ReadonlySet<string>
  onToggle: (key: string) => void
  /** Commits currently on the slider (D84 — described, never capped). */
  commits: IssueCommit[]
  /**
   * D80: the tail block cannot be deselected. Notify needs a legal `to`, which only the
   * tail can provide (D79). Tabs whose selected commit takes no tail constraint (Review,
   * D82) pass `false` and every row is freely selectable.
   */
  pinTail: boolean
}

// One centre line for the whole rail: every graphic is positioned from it, so centring is
// arithmetic that cannot drift. `borderLeft` plus a content-box ring — the previous
// approach — put the line's centre and the dot's centre in different places.
const RAIL_WIDTH = 22
const RAIL_CENTER = 11
const LINE_WIDTH = 2
const DOT_SIZE = 9

/**
 * §22/D70: replaces the round `SegmentedControl`. Rows are segments — rounds as dots on a
 * timeline, the gaps between them as the line connecting those dots — and the slider shows
 * the union of what is ticked.
 *
 * Rendered **newest-first**, so it reads the way the slider does not: the round in progress
 * is at the top and round 1 sits at the bottom, matching how a history is usually scanned.
 * The *order* still comes from `status.history` (M2); only the display is reversed.
 *
 * D71: opens on the latest round alone, so the default view is exactly the pre-§22 one and
 * §0.5 (the slider that grew with the QC's life) is not reopened. Widening is deliberate.
 */
export function HistorySelect({ status, history, selected, onToggle, commits, pinTail }: Props) {
  const selectedCount = history.filter((segment) => selected.has(segment.key)).length
  // Newest-first for display only — `flattenSelection` still walks the server's order.
  const rows = [...history].reverse()

  return (
    <Group gap="xs" align="center" data-testid="history-select">
      <Text size="sm" fw={700}>History:</Text>
      <Popover position="bottom-start" withArrow shadow="md" width={520}>
        <Popover.Target>
          <Button
            size="compact-xs"
            variant="default"
            rightSection={<IconChevronDown size={14} />}
            data-testid="history-select-trigger"
          >
            {selectedCount === 1 ? '1 segment' : `${selectedCount} segments`}
          </Button>
        </Popover.Target>
        <Popover.Dropdown>
          <div>
            {rows.map((segment, position) => (
              <HistoryRow
                key={segment.key}
                status={status}
                segment={segment}
                checked={selected.has(segment.key)}
                hasLineAbove={position > 0}
                hasLineBelow={position < rows.length - 1}
                locked={pinTail && segment.isTail}
                onToggle={() => onToggle(segment.key)}
              />
            ))}
          </div>
        </Popover.Dropdown>
      </Popover>
      {/* D84: no cap — the user opted in — but §0.5 was about volume, so the size of the
          selection is always in view, in the same words the rows use. */}
      <Text size="xs" c="dimmed" data-testid="history-commit-count">
        {describeCommits(commits)}
      </Text>
    </Group>
  )
}

function HistoryRow({
  status,
  segment,
  checked,
  hasLineAbove,
  hasLineBelow,
  locked,
  onToggle,
}: {
  status: IssueStatusResponse
  segment: HistorySegment
  checked: boolean
  hasLineAbove: boolean
  hasLineBelow: boolean
  locked: boolean
  onToggle: () => void
}) {
  const { round, ref } = segment
  const isRound = ref.kind === 'round'
  // D90: a gap that owns **no commits at all** is not selectable — ticking it could not
  // change anything. This keys on the raw count, not the file-changing one: a gap holding
  // a commit that touched nothing still has something to put on the slider under "Show
  // all commits", and the row now says so rather than reading as empty.
  //
  // Rounds are exempt: an empty round is an unplaceable one (D53.3), where empty means
  // "fetch the branch", and selecting it is how D77's remedy notice is surfaced.
  const contributesNothing = !isRound && segment.commits.length === 0
  const disabledReason = locked
    ? "The latest round is always shown — a notification's current commit has to come from it"
    : contributesNothing
      ? 'This segment owns no commits'
      : null

  return (
    <div
      data-testid={`history-row-${segment.key}`}
      style={{
        display: 'grid',
        // rail | label+badges | checkbox, so every checkbox lands on the same right edge.
        gridTemplateColumns: `${RAIL_WIDTH}px 1fr auto`,
        alignItems: 'center',
        columnGap: 8,
        minHeight: isRound ? 30 : 26,
      }}
    >
      <Rail
        isRound={isRound}
        divergent={segment.divergent}
        hasLineAbove={hasLineAbove}
        hasLineBelow={hasLineBelow}
        segmentKey={segment.key}
      />

      <Group gap={6} wrap="nowrap" align="center" style={{ minWidth: 0 }}>
        {isRound ? (
          <Text size="xs" fw={600} span style={{ whiteSpace: 'nowrap' }}>{segment.label}</Text>
        ) : (
          // A gap belongs to the round it *precedes* (D9), and in newest-first order that
          // round is the row directly above. The label names it and the rail's stub points
          // at it, so the descriptor cannot be read as hanging off the older round below.
          <Text size="xs" c="dimmed" span style={{ whiteSpace: 'nowrap' }}>{segment.label}</Text>
        )}
        <Text size="10px" c="dimmed" style={{ whiteSpace: 'nowrap' }}>
          {describeCommits(segment.commits)}
        </Text>
        {isRound && round.state.kind === 'approved' && (
          <Badge color="green" variant="light" size="xs">
            {round.placement === 'unplaceable' || !approvalCommentUrl(status.issue, round)
              ? 'approved'
              : `approved ${round.state.commit.slice(0, 7)}`}
          </Badge>
        )}
        {isRound && round.state.kind === 'superseded' && (
          <Badge color="gray" variant="light" size="xs">superseded</Badge>
        )}
        {isRound && round.state.kind === 'open' && (
          <Badge color="blue" variant="light" size="xs">open</Badge>
        )}
        {/* D77/D53: an unplaceable round is a row that contributes nothing and names its
            remedy — never hidden. */}
        {isRound && round.placement === 'unplaceable' && <FetchBranchBadge branch={round.branch} />}
        {isRound && round.branch_inherited && <InheritedBranchBadge branch={round.branch} />}
        {segment.divergent && <NoCohesiveHistoryBadge />}
      </Group>

      <Tooltip
        label={disabledReason ?? ''}
        disabled={disabledReason === null}
        withArrow
        position="left"
        multiline
        w={260}
      >
        <span style={{ display: 'inline-flex' }}>
          <Checkbox
            size="xs"
            checked={checked}
            disabled={disabledReason !== null}
            onChange={onToggle}
            aria-label={segment.label}
            data-testid={`history-check-${segment.key}`}
          />
        </span>
      </Tooltip>
    </div>
  )
}

/**
 * The timeline graphic. A round is a dot; a gap is the line joining two dots. D74's break
 * is drawn *on the line* as two slashes with space between them — the conventional
 * "these ends are not continuous" mark — so divergence reads as a property of the
 * connection rather than of either round.
 */
function Rail({
  isRound,
  divergent,
  hasLineAbove,
  hasLineBelow,
  segmentKey,
}: {
  isRound: boolean
  divergent: boolean
  hasLineAbove: boolean
  hasLineBelow: boolean
  segmentKey: string
}) {
  const lineColor = 'var(--mantine-color-gray-4)'
  // Half the break's height, so the two line halves stop clear of the slashes.
  const breakHalf = 9

  return (
    <div style={{ position: 'relative', alignSelf: 'stretch', minHeight: 26 }}>
      {/* Upper half of the connector. */}
      {hasLineAbove && (
        <div style={{
          position: 'absolute',
          left: RAIL_CENTER - LINE_WIDTH / 2,
          width: LINE_WIDTH,
          top: 0,
          height: divergent ? `calc(50% - ${breakHalf}px)` : '50%',
          backgroundColor: lineColor,
        }} />
      )}
      {/* Lower half. */}
      {hasLineBelow && (
        <div style={{
          position: 'absolute',
          left: RAIL_CENTER - LINE_WIDTH / 2,
          width: LINE_WIDTH,
          top: divergent ? `calc(50% + ${breakHalf}px)` : '50%',
          bottom: 0,
          backgroundColor: lineColor,
        }} />
      )}

      {/* D74/D88: the break — two slashes with space between them, struck across the line
          and centred on it, so it reads as a property of the connection. */}
      {divergent && (
        <div
          data-testid={`history-break-${segmentKey}`}
          title="not adjacent in history"
          style={{
            position: 'absolute',
            left: RAIL_CENTER - 5,
            top: `calc(50% - ${breakHalf}px)`,
            width: 10,
            height: breakHalf * 2,
          }}
        >
          {[2, 13].map((offset) => (
            <div
              key={offset}
              style={{
                position: 'absolute',
                left: 0,
                top: offset,
                width: 10,
                height: 1.5,
                backgroundColor: 'var(--mantine-color-red-6)',
                transform: 'rotate(-55deg)',
              }}
            />
          ))}
        </div>
      )}

      {/* A round is a dot on the line, opaque so the connector passes behind it. */}
      {isRound && (
        <div style={{
          position: 'absolute',
          left: RAIL_CENTER - DOT_SIZE / 2,
          top: `calc(50% - ${DOT_SIZE / 2}px)`,
          width: DOT_SIZE,
          height: DOT_SIZE,
          borderRadius: '50%',
          backgroundColor: 'var(--mantine-color-gray-7)',
        }} />
      )}

      {/* D88: the stub from the connector to the gap's label. The row is
          `align-items: center`, so 50% of the rail is exactly the label's own centre —
          the previous fixed `top` sat 6px above it. */}
      {!isRound && (
        <div style={{
          position: 'absolute',
          left: RAIL_CENTER,
          top: `calc(50% - ${LINE_WIDTH / 2}px)`,
          width: RAIL_WIDTH - RAIL_CENTER,
          height: LINE_WIDTH,
          backgroundColor: lineColor,
        }} />
      )}
    </div>
  )
}
